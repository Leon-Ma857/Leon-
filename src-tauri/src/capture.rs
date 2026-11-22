use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::imageops::FilterType as ResizeFilter;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use serde::Serialize;
use xcap::Monitor;

#[derive(Clone)]
pub struct CapturedScreen {
    pub image: RgbaImage,
    pub monitor_x: i32,
    pub monitor_y: i32,
    pub monitor_width: u32,
    pub monitor_height: u32,
    pub scale_factor: f32,
    pub png_bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct CroppedRegion {
    pub raw_base64: String,
    pub png_bytes: Vec<u8>,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_factor: f32,
}

impl CroppedRegion {
    pub fn data_url(&self) -> String {
        format!("data:image/png;base64,{}", self.raw_base64)
    }

    /// Generate a compact thumbnail data URL for history storage so memory stays minimal.
    pub fn make_thumbnail_data_url(&self) -> String {
        if self.pixel_width <= 260 && self.pixel_height <= 180 {
            return self.data_url();
        }
        let Ok(dyn_img) = image::load_from_memory(&self.png_bytes) else {
            return String::new();
        };
        let thumb = dyn_img.thumbnail(260, 180);
        let rgba = thumb.to_rgba8();
        let mut buf = Vec::new();
        let encoder =
            PngEncoder::new_with_quality(&mut buf, CompressionType::Default, FilterType::NoFilter);
        if encoder
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                ExtendedColorType::Rgba8,
            )
            .is_err()
        {
            return String::new();
        }
        format!("data:image/png;base64,{}", BASE64.encode(&buf))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayPayload {
    pub capture_id: u64,
    pub monitor_width: u32,
    pub monitor_height: u32,
    pub scale_factor: f32,
    pub auto_translate_on_release: bool,
    pub translate_engine: String,
    pub target_lang: String,
    pub staged_count: usize,
    pub max_stack_size: usize,
    pub latest_staged_thumb: String,
}

fn encode_rgba_to_png_bytes(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::with_capacity((image.width() * image.height() / 4) as usize);
    let encoder =
        PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("PNG 编码失败: {e}"))?;
    Ok(buf)
}

pub fn decode_png_to_rgba(png_bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let dyn_img =
        image::load_from_memory(png_bytes).map_err(|e| format!("解析 PNG 图像失败: {e}"))?;
    let rgba = dyn_img.to_rgba8();
    Ok((rgba.width() as usize, rgba.height() as usize, rgba.into_raw()))
}

pub fn decode_crop_from_base64(
    image_base64: &str,
    scale_factor: f32,
) -> Result<CroppedRegion, String> {
    let pure_b64 = image_base64
        .trim()
        .strip_prefix("data:image/png;base64,")
        .or_else(|| image_base64.trim().strip_prefix("data:image/jpeg;base64,"))
        .unwrap_or(image_base64.trim());

    let png_bytes = BASE64
        .decode(pure_b64)
        .map_err(|e| format!("解码图像 Base64 失败: {e}"))?;
    let dyn_img =
        image::load_from_memory(&png_bytes).map_err(|e| format!("读取图像尺寸失败: {e}"))?;
    let pixel_width = dyn_img.width();
    let pixel_height = dyn_img.height();
    if pixel_width < 4 || pixel_height < 4 {
        return Err("选区过小，请框选更大区域".to_string());
    }

    // Re-encode if input was JPEG so downstream always gets clean PNG bytes
    let (final_png_bytes, final_b64) = if image_base64.trim().starts_with("data:image/jpeg") {
        let rgba = dyn_img.to_rgba8();
        let bytes = encode_rgba_to_png_bytes(&rgba)?;
        let b64 = BASE64.encode(&bytes);
        (bytes, b64)
    } else {
        (png_bytes, pure_b64.to_string())
    };

    Ok(CroppedRegion {
        raw_base64: final_b64,
        png_bytes: final_png_bytes,
        pixel_width,
        pixel_height,
        scale_factor: scale_factor.max(1.0),
    })
}

/// Resize down oversized screenshots before sending to vision LLMs to cut latency and memory.
pub fn encode_vision_optimized_base64(crop: &CroppedRegion) -> String {
    let max_side = 1600u32;
    if crop.pixel_width <= max_side && crop.pixel_height <= max_side {
        return crop.raw_base64.clone();
    }
    let Ok(dyn_img) = image::load_from_memory(&crop.png_bytes) else {
        return crop.raw_base64.clone();
    };
    let resized = dyn_img.resize(max_side, max_side, ResizeFilter::Triangle);
    let rgba = resized.to_rgba8();
    match encode_rgba_to_png_bytes(&rgba) {
        Ok(bytes) => BASE64.encode(&bytes),
        Err(_) => crop.raw_base64.clone(),
    }
}

fn select_target_monitor(cursor: Option<(i32, i32)>) -> Result<Monitor, String> {
    if let Some((cx, cy)) = cursor {
        #[cfg(target_os = "macos")]
        if let Ok(monitors) = Monitor::all() {
            if let Some(found) = monitors.into_iter().find(|m| {
                let sf = f64::from(m.scale_factor().unwrap_or(1.0).max(1.0));
                let x = (f64::from(m.x().unwrap_or(0)) * sf).round() as i32;
                let y = (f64::from(m.y().unwrap_or(0)) * sf).round() as i32;
                let w = (f64::from(m.width().unwrap_or(0)) * sf).round() as i32;
                let h = (f64::from(m.height().unwrap_or(0)) * sf).round() as i32;
                cx >= x && cx < x + w && cy >= y && cy < y + h
            }) {
                return Ok(found);
            }
        }
        #[cfg(not(target_os = "macos"))]
        if let Ok(monitor) = Monitor::from_point(cx, cy) {
            return Ok(monitor);
        }
    }

    let monitors = Monitor::all().map_err(|e| format!("枚举显示器失败: {e}"))?;
    if monitors.is_empty() {
        return Err("未检测到可用显示器".to_string());
    }

    if let Some(primary) = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .cloned()
    {
        return Ok(primary);
    }

    Ok(monitors.into_iter().next().unwrap())
}

pub fn capture_active_monitor(cursor: Option<(i32, i32)>) -> Result<CapturedScreen, String> {
    let monitor = select_target_monitor(cursor)?;
    let monitor_x = monitor
        .x()
        .map_err(|e| format!("读取显示器 X 坐标失败: {e}"))?;
    let monitor_y = monitor
        .y()
        .map_err(|e| format!("读取显示器 Y 坐标失败: {e}"))?;
    let monitor_width = monitor
        .width()
        .map_err(|e| format!("读取显示器宽度失败: {e}"))?;
    let monitor_height = monitor
        .height()
        .map_err(|e| format!("读取显示器高度失败: {e}"))?;
    let scale_factor = monitor.scale_factor().unwrap_or(1.0);

    let image = monitor
        .capture_image()
        .map_err(|e| format!("屏幕截图失败: {e}"))?;

    #[cfg(target_os = "macos")]
    let (monitor_x, monitor_y, monitor_width, monitor_height) = {
        let sf = f64::from(scale_factor.max(1.0));
        (
            (f64::from(monitor_x) * sf).round() as i32,
            (f64::from(monitor_y) * sf).round() as i32,
            image.width(),
            image.height(),
        )
    };

    let png_bytes = encode_rgba_to_png_bytes(&image)?;
    Ok(CapturedScreen {
        image,
        monitor_x,
        monitor_y,
        monitor_width,
        monitor_height,
        scale_factor,
        png_bytes,
    })
}

pub fn compute_popup_position(
    captured: &CapturedScreen,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    popup_w_logical: f64,
    popup_h_logical: f64,
    over_selection: bool,
) -> (i32, i32) {
    let nx = rx.clamp(0.0, 1.0);
    let ny = ry.clamp(0.0, 1.0);
    let nh = rh.clamp(0.0, 1.0 - ny);
    let _nw = rw.clamp(0.0, 1.0 - nx);

    let scale = f64::from(captured.scale_factor.max(1.0));
    let popup_w_phys = (popup_w_logical * scale) as i32;
    let popup_h_phys = (popup_h_logical * scale) as i32;
    let margin = (12.0 * scale) as i32;

    let mon_x = captured.monitor_x;
    let mon_y = captured.monitor_y;
    let mon_right = mon_x + captured.monitor_width as i32;
    let mon_bottom = mon_y + captured.monitor_height as i32;

    let sel_x = mon_x + ((nx * captured.monitor_width as f64).round() as i32);
    let sel_y = mon_y + ((ny * captured.monitor_height as f64).round() as i32);
    let sel_bottom = sel_y + ((nh * captured.monitor_height as f64).round() as i32);

    let mut target_x = sel_x.min(mon_right - popup_w_phys - margin);
    target_x = target_x.max(mon_x + margin);

    if over_selection {
        let target_y = sel_y
            .min(mon_bottom - popup_h_phys - margin)
            .max(mon_y + margin);
        return (target_x, target_y);
    }

    let mut target_y = sel_bottom + margin;
    if target_y + popup_h_phys > mon_bottom - margin {
        let above_y = sel_y - popup_h_phys - margin;
        if above_y >= mon_y + margin {
            target_y = above_y;
        } else {
            target_y = (mon_bottom - popup_h_phys - margin).max(mon_y + margin);
        }
    }

    (target_x, target_y)
}

/// Crop normalized [0.0, 1.0] rectangle from the captured screen.
pub fn crop_normalized(
    captured: &CapturedScreen,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
) -> Result<CroppedRegion, String> {
    let img_w = captured.image.width() as f64;
    let img_h = captured.image.height() as f64;

    let nx = rx.clamp(0.0, 1.0);
    let ny = ry.clamp(0.0, 1.0);
    let nw = rw.clamp(0.0, 1.0 - nx);
    let nh = rh.clamp(0.0, 1.0 - ny);

    let px = (nx * img_w).round() as u32;
    let py = (ny * img_h).round() as u32;
    let pw = ((nw * img_w).round() as u32).min(captured.image.width().saturating_sub(px));
    let ph = ((nh * img_h).round() as u32).min(captured.image.height().saturating_sub(py));

    if pw < 6 || ph < 6 {
        return Err("选区过小，请拖拽框选更大区域".to_string());
    }

    let cropped = image::imageops::crop_imm(&captured.image, px, py, pw, ph).to_image();
    let png_bytes = encode_rgba_to_png_bytes(&cropped)?;
    let raw_base64 = BASE64.encode(&png_bytes);

    Ok(CroppedRegion {
        raw_base64,
        png_bytes,
        pixel_width: pw,
        pixel_height: ph,
        scale_factor: captured.scale_factor.max(1.0),
    })
}

/// Stitch multiple cropped regions into a single clean PNG image ("v" | "h" | "grid").
pub fn stitch_regions(regions: &[CroppedRegion], direction: &str) -> Result<CroppedRegion, String> {
    if regions.is_empty() {
        return Err("没有可拼接的图片".to_string());
    }
    if regions.len() == 1 {
        return Ok(regions[0].clone());
    }

    let gap = 12u64;
    let pad = 12u64;
    let count = regions.len() as u64;
    let cell_width = u64::from(regions.iter().map(|crop| crop.pixel_width).max().unwrap_or(1));
    let cell_height = u64::from(regions.iter().map(|crop| crop.pixel_height).max().unwrap_or(1));
    let padding = 2 * pad;
    let gaps = (count - 1) * gap;
    let dir = direction.trim().to_ascii_lowercase();
    let horizontal = matches!(dir.as_str(), "h" | "horizontal");
    let grid = matches!(dir.as_str(), "g" | "grid");
    let columns = (count as f64).sqrt().ceil() as u64;
    let rows = count.div_ceil(columns);
    let (width, height) = if horizontal {
        (regions.iter().map(|crop| u64::from(crop.pixel_width)).sum::<u64>() + padding + gaps,
            cell_height + padding)
    } else if grid {
        (columns * cell_width + (columns - 1) * gap + padding,
            rows * cell_height + (rows - 1) * gap + padding)
    } else {
        (cell_width + padding,
            regions.iter().map(|crop| u64::from(crop.pixel_height)).sum::<u64>() + padding + gaps)
    };

    // Scale the layout before allocating. Decode one input at a time.
    let max_edge = 8192.0;
    let max_pixels = 24_000_000.0;
    let scale = (max_edge / width.max(height) as f64)
        .min((max_pixels / (width as f64 * height as f64)).sqrt())
        .min(1.0);
    let scaled_pos = |value: u64| (value as f64 * scale).floor() as u32;
    let scaled_len = |value: u64| scaled_pos(value).max(1);
    let mut canvas = RgbaImage::from_pixel(scaled_len(width), scaled_len(height), image::Rgba([241, 245, 249, 255]));
    let mut cursor = pad;
    for (index, crop) in regions.iter().enumerate() {
        let mut image = image::load_from_memory(&crop.png_bytes)
            .map_err(|err| format!("解析第 {} 张图片失败: {err}", index + 1))?
            .into_rgba8();
        let image_width = u64::from(image.width());
        let image_height = u64::from(image.height());
        let (x, y) = if horizontal {
            let position = (cursor, pad + cell_height.saturating_sub(image_height) / 2);
            cursor += image_width + gap;
            position
        } else if grid {
            let index = index as u64;
            let x = pad + index % columns * (cell_width + gap);
            let y = pad + index / columns * (cell_height + gap);
            (x + cell_width.saturating_sub(image_width) / 2, y + cell_height.saturating_sub(image_height) / 2)
        } else {
            let position = (pad + cell_width.saturating_sub(image_width) / 2, cursor);
            cursor += image_height + gap;
            position
        };
        if scale < 1.0 {
            image = image::imageops::resize(&image, scaled_len(image_width), scaled_len(image_height), ResizeFilter::Triangle);
        }
        image::imageops::replace(&mut canvas, &image, i64::from(scaled_pos(x)), i64::from(scaled_pos(y)));
    }

    let pixel_width = canvas.width();
    let pixel_height = canvas.height();
    let png_bytes = encode_rgba_to_png_bytes(&canvas)?;
    let raw_base64 = BASE64.encode(&png_bytes);
    let scale_factor = regions
        .first()
        .map(|r| r.scale_factor)
        .unwrap_or(1.0)
        .max(1.0);

    Ok(CroppedRegion {
        raw_base64,
        png_bytes,
        pixel_width,
        pixel_height,
        scale_factor,
    })
}

pub fn cropped_region_from_rgba(
    width: u32,
    height: u32,
    rgba_bytes: Vec<u8>,
    scale_factor: f32,
) -> Result<CroppedRegion, String> {
    if width < 4 || height < 4 {
        return Err("剪贴板图像尺寸过小，无法识别".to_string());
    }
    let img = RgbaImage::from_raw(width, height, rgba_bytes)
        .ok_or_else(|| "剪贴板图像像素数据格式异常".to_string())?;
    let png_bytes = encode_rgba_to_png_bytes(&img)?;
    let raw_base64 = BASE64.encode(&png_bytes);
    Ok(CroppedRegion {
        raw_base64,
        png_bytes,
        pixel_width: width,
        pixel_height: height,
        scale_factor: scale_factor.max(1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solid_crop(w: u32, h: u32, rgba: [u8; 4]) -> CroppedRegion {
        let img = RgbaImage::from_pixel(w, h, image::Rgba(rgba));
        let png_bytes = encode_rgba_to_png_bytes(&img).unwrap();
        let raw_base64 = BASE64.encode(&png_bytes);
        CroppedRegion {
            raw_base64,
            png_bytes,
            pixel_width: w,
            pixel_height: h,
            scale_factor: 1.0,
        }
    }

    #[test]
    fn stitch_empty_errors() {
        assert!(stitch_regions(&[], "v").is_err());
    }

    #[test]
    fn stitch_vertical_horizontal_and_grid_dimensions() {
        let a = make_solid_crop(40, 20, [255, 0, 0, 255]);
        let b = make_solid_crop(60, 30, [0, 255, 0, 255]);
        let c = make_solid_crop(30, 10, [0, 0, 255, 255]);

        let v = stitch_regions(&[a.clone(), b.clone()], "v").unwrap();
        // width = max(40,60) + 24 = 84, height = 20 + 30 + 12 + 24 = 86
        assert_eq!((v.pixel_width, v.pixel_height), (84, 86));

        let h = stitch_regions(&[a.clone(), b.clone()], "h").unwrap();
        // width = 40 + 60 + 12 + 24 = 136, height = max(20,30) + 24 = 54
        assert_eq!((h.pixel_width, h.pixel_height), (136, 54));

        let g = stitch_regions(&[a, b, c], "grid").unwrap();
        // 3 items -> 2 cols x 2 rows, cell 60x30 -> w = 2*60 + 12 + 24 = 156, h = 2*30 + 12 + 24 = 96
        assert_eq!((g.pixel_width, g.pixel_height), (156, 96));
    }

    #[test]
    fn cropped_region_from_rgba_validates_and_encodes() {
        let tiny = vec![0u8; 2 * 2 * 4];
        assert!(cropped_region_from_rgba(2, 2, tiny, 1.0).is_err());

        let rgba = vec![120u8; 8 * 6 * 4];
        let crop = cropped_region_from_rgba(8, 6, rgba, 1.5).unwrap();
        assert_eq!((crop.pixel_width, crop.pixel_height), (8, 6));
        assert!(!crop.png_bytes.is_empty());
        assert!(!crop.raw_base64.is_empty());
    }
}
