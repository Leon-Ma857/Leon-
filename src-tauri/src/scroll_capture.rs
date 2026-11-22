use image::{DynamicImage, ImageFormat, RgbaImage};
use std::io::Cursor;

/// Find vertical overlap (number of bottom rows of `top` that match the top rows of `bottom`).
/// Ignores a narrow right strip (scrollbar) and compares interior rows using mean absolute difference.
pub fn find_vertical_overlap(top: &RgbaImage, bottom: &RgbaImage) -> Option<u32> {
    let (tw, th) = top.dimensions();
    let (bw, bh) = bottom.dimensions();
    if tw != bw || tw < 24 || th < 24 || bh < 24 {
        return None;
    }

    // Ignore rightmost up to 20px (scrollbar) and 4px outer margin so fixed borders don't skew matching.
    let x_start = 4u32.min(tw / 8);
    let x_end = tw.saturating_sub(20).max(x_start + 8).min(tw);
    let step_x = ((x_end - x_start) / 64).max(1);

    // Search overlap in [16 .. min(th, bh) * 85 / 100]
    let max_overlap = (th.min(bh) * 85 / 100).max(16).min(th).min(bh);
    let min_overlap = 16u32.min(max_overlap);

    let top_raw = top.as_raw();
    let bot_raw = bottom.as_raw();
    let stride = (tw * 4) as usize;

    let mut best_overlap: Option<u32> = None;
    let mut best_mad = f64::MAX;

    // Coarse-to-fine search: step 2 rows first, then refine +-2 rows.
    let mut candidate_overlaps = Vec::new();
    let mut ov = max_overlap;
    while ov >= min_overlap {
        candidate_overlaps.push(ov);
        if ov < min_overlap + 2 {
            break;
        }
        ov -= 2;
    }

    let eval_overlap = |ov_rows: u32| -> f64 {
        let step_y = (ov_rows / 24).max(1);
        let top_base = th - ov_rows;
        let mut diff_sum = 0u64;
        let mut samples = 0u64;
        let mut y = 0u32;
        while y < ov_rows {
            let tr = ((top_base + y) as usize) * stride;
            let br = (y as usize) * stride;
            let mut x = x_start;
            while x < x_end {
                let off = (x * 4) as usize;
                let dr = (top_raw[tr + off] as i32 - bot_raw[br + off] as i32).abs();
                let dg = (top_raw[tr + off + 1] as i32 - bot_raw[br + off + 1] as i32).abs();
                let db = (top_raw[tr + off + 2] as i32 - bot_raw[br + off + 2] as i32).abs();
                diff_sum += (dr + dg + db) as u64;
                samples += 3;
                x += step_x;
            }
            y += step_y;
        }
        if samples == 0 {
            f64::MAX
        } else {
            diff_sum as f64 / samples as f64
        }
    };

    for ov_rows in candidate_overlaps {
        let mad = eval_overlap(ov_rows);
        if mad < best_mad {
            best_mad = mad;
            best_overlap = Some(ov_rows);
        }
    }

    if let Some(coarse) = best_overlap {
        let lo = coarse.saturating_sub(2).max(min_overlap);
        let hi = (coarse + 2).min(max_overlap);
        for ov_rows in lo..=hi {
            let mad = eval_overlap(ov_rows);
            if mad < best_mad {
                best_mad = mad;
                best_overlap = Some(ov_rows);
            }
        }
    }

    // Require strong match (average channel diff < 6.5 / 255)
    if best_mad < 6.5 {
        best_overlap
    } else {
        None
    }
}

pub fn frames_nearly_identical(a: &RgbaImage, b: &RgbaImage) -> bool {
    if a.dimensions() != b.dimensions() {
        return false;
    }
    let (w, h) = a.dimensions();
    let x_start = 4u32.min(w / 8);
    let x_end = w.saturating_sub(20).max(x_start + 8).min(w);
    let step_x = ((x_end - x_start) / 48).max(1);
    let step_y = (h / 48).max(1);
    let ar = a.as_raw();
    let br = b.as_raw();
    let stride = (w * 4) as usize;
    let mut diff_sum = 0u64;
    let mut samples = 0u64;

    let mut y = 0u32;
    while y < h {
        let row = (y as usize) * stride;
        let mut x = x_start;
        while x < x_end {
            let off = row + (x * 4) as usize;
            diff_sum += (ar[off] as i32 - br[off] as i32).abs() as u64;
            diff_sum += (ar[off + 1] as i32 - br[off + 1] as i32).abs() as u64;
            diff_sum += (ar[off + 2] as i32 - br[off + 2] as i32).abs() as u64;
            samples += 3;
            x += step_x;
        }
        y += step_y;
    }

    samples > 0 && (diff_sum as f64 / samples as f64) < 1.2
}

pub fn stitch_vertical_frames(frames: &[RgbaImage]) -> Result<(RgbaImage, Vec<u8>), String> {
    if frames.is_empty() {
        return Err("没有捕获到任何长截图帧".to_string());
    }
    let width = frames[0].width();
    let mut strips: Vec<&[u8]> = Vec::new();
    let mut total_height = frames[0].height();
    let stride = (width * 4) as usize;
    strips.push(frames[0].as_raw());

    let mut prev = &frames[0];
    for frame in frames.iter().skip(1) {
        if frame.width() != width {
            break;
        }
        if frames_nearly_identical(prev, frame) {
            break;
        }
        let overlap = find_vertical_overlap(prev, frame).unwrap_or(0);
        let new_rows = frame.height().saturating_sub(overlap);
        if new_rows < 8 {
            break;
        }
        let start_byte = (overlap as usize) * stride;
        strips.push(&frame.as_raw()[start_byte..]);
        total_height = total_height.saturating_add(new_rows);
        prev = frame;
        if total_height >= 16000 {
            break;
        }
    }

    let mut combined = Vec::with_capacity((width as usize) * (total_height as usize) * 4);
    for s in strips {
        combined.extend_from_slice(s);
    }
    let out_img = RgbaImage::from_raw(width, total_height, combined)
        .ok_or_else(|| "拼接长截图缓冲区失败".to_string())?;
    let mut png_bytes = Vec::new();
    DynamicImage::ImageRgba8(out_img.clone())
        .write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
        .map_err(|e| format!("编码长截图失败: {e}"))?;
    Ok((out_img, png_bytes))
}

#[cfg(windows)]
pub fn send_scroll_wheel_at(phys_x: i32, phys_y: i32, delta: i32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;

    unsafe {
        let _ = SetCursorPos(phys_x, phys_y);
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: delta as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let _ = SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
pub fn is_escape_pressed() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
    unsafe { (GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000) != 0 }
}

#[cfg(target_os = "macos")]
pub fn send_scroll_wheel_at(phys_x: i32, phys_y: i32, delta: i32) {
    let lines = if delta == 0 {
        0
    } else {
        (delta / 40).clamp(-12, 12)
    };
    crate::macos_bridge::send_scroll_wheel_at(phys_x, phys_y, lines);
}

#[cfg(target_os = "macos")]
pub fn is_escape_pressed() -> bool {
    crate::macos_bridge::is_escape_pressed()
}

#[cfg(test)]
mod tests {
    use super::{find_vertical_overlap, stitch_vertical_frames};
    use image::{Rgba, RgbaImage};

    fn make_striped_frame(w: u32, h: u32, y_offset: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            let gy = y + y_offset;
            for x in 0..w {
                let r = ((gy * 7 + x) % 251) as u8;
                let g = ((gy * 13) % 241) as u8;
                let b = ((gy * 3 + x * 2) % 239) as u8;
                img.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
        img
    }

    #[test]
    fn stitches_overlapping_frames_accurately() {
        let f1 = make_striped_frame(120, 100, 0);
        let f2 = make_striped_frame(120, 100, 60); // 40px overlap
        let ov = find_vertical_overlap(&f1, &f2);
        assert_eq!(ov, Some(40));
        let (stitched, png) = stitch_vertical_frames(&[f1, f2]).unwrap();
        assert_eq!(stitched.width(), 120);
        assert_eq!(stitched.height(), 160);
        assert!(!png.is_empty());
    }
}
