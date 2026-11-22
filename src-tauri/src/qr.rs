use image::imageops::FilterType;
use image::{DynamicImage, GrayImage, Luma};

fn decode_from_gray(gray: &GrayImage) -> Vec<String> {
    let mut prepared = rqrr::PreparedImage::prepare(gray.clone());
    let grids = prepared.detect_grids();
    let mut out = Vec::new();
    for grid in grids {
        if let Ok((_meta, content)) = grid.decode() {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() && !out.contains(&trimmed) {
                out.push(trimmed);
            }
        }
    }
    out
}

fn invert_gray(gray: &GrayImage) -> GrayImage {
    let mut inv = gray.clone();
    for p in inv.pixels_mut() {
        p[0] = 255u8.saturating_sub(p[0]);
    }
    inv
}

fn add_white_border(gray: &GrayImage, pad: u32) -> GrayImage {
    let (w, h) = gray.dimensions();
    let mut canvas = GrayImage::from_pixel(w + pad * 2, h + pad * 2, Luma([255]));
    for (x, y, p) in gray.enumerate_pixels() {
        canvas.put_pixel(x + pad, y + pad, *p);
    }
    canvas
}

fn push_unique(found: &mut Vec<String>, items: Vec<String>) {
    for item in items {
        if !found.contains(&item) {
            found.push(item);
        }
    }
}

/// Decode one or more QR codes from PNG bytes, trying native, padded, upscaled, and inverted variants.
pub fn decode_qr_from_png(png_bytes: &[u8]) -> Result<Vec<String>, String> {
    let dyn_img =
        image::load_from_memory(png_bytes).map_err(|e| format!("解析截图失败: {e}"))?;
    let gray = dyn_img.to_luma8();
    let (w, h) = gray.dimensions();

    let mut found = Vec::new();

    // 1. Native grayscale + quiet-zone padding (helps tight crops around QR edges)
    let padded = add_white_border(&gray, 24);
    push_unique(&mut found, decode_from_gray(&padded));
    if !found.is_empty() {
        return Ok(found);
    }

    // 2. Inverted grayscale (dark-mode QR codes)
    let inv_padded = add_white_border(&invert_gray(&gray), 24);
    push_unique(&mut found, decode_from_gray(&inv_padded));
    if !found.is_empty() {
        return Ok(found);
    }

    // 3. Upscaled 2x for small / dense QR codes
    if w < 1200 && h < 1200 {
        let up = DynamicImage::ImageLuma8(gray)
            .resize_exact((w * 2).max(1), (h * 2).max(1), FilterType::CatmullRom)
            .to_luma8();
        let up_padded = add_white_border(&up, 32);
        push_unique(&mut found, decode_from_gray(&up_padded));
        if !found.is_empty() {
            return Ok(found);
        }

        let up_inv = add_white_border(&invert_gray(&up), 32);
        push_unique(&mut found, decode_from_gray(&up_inv));
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::decode_qr_from_png;

    #[test]
    fn handles_blank_image_without_error() {
        let img = image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        let res = decode_qr_from_png(&buf).unwrap();
        assert!(res.is_empty());
    }
}
