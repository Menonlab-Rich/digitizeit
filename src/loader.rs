use std::io::Read;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;

use anyhow::Result;
use image::{DynamicImage, RgbaImage};

pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "gif", "tga", "webp", "tif", "tiff", "pnm", "qoi", "dds", "ff",
    "hdr", "exr", "ico",
];

pub fn load_from_path(path: &Path) -> Result<RgbaImage> {
    let img = image::open(path)?;
    Ok(rgba_opaque(img))
}

pub fn rgba_opaque(img: DynamicImage) -> RgbaImage {
    composite_alpha(img.to_rgba8())
}

pub fn composite_alpha(mut rgba: RgbaImage) -> RgbaImage {
    let has_alpha = rgba.pixels().any(|p| p.0[3] < 255);
    if !has_alpha {
        return rgba;
    }
    for p in rgba.pixels_mut() {
        let a = p.0[3] as u32;
        if a == 255 {
            continue;
        }
        for i in 0..3 {
            let c = p.0[i] as u32;
            p.0[i] = ((c * a + 255 * (255 - a) + 127) / 255) as u8;
        }
        p.0[3] = 255;
    }
    rgba
}

pub fn load_from_clipboard() -> Result<RgbaImage> {
    let mut cb = arboard::Clipboard::new()?;
    let img = cb.get_image()?;
    let (width, height, data) = (img.width, img.height, img.into_owned_bytes());
    RgbaImage::from_raw(width as u32, height as u32, data.to_vec())
        .map(composite_alpha)
        .ok_or_else(|| anyhow::anyhow!("clipboard image size mismatch"))
}

pub fn spawn_url_download(url: String, tx: Sender<Result<RgbaImage, String>>) {
    thread::spawn(move || {
        let result = (|| -> Result<RgbaImage, String> {
            let resp = ureq::get(&url).call().map_err(|e| e.to_string())?;
            let mut body = resp.into_body();
            let bytes = body.read_to_vec().map_err(|e| e.to_string())?;
            let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
            Ok(rgba_opaque(img))
        })();
        let _ = tx.send(result);
    });
}

pub fn dropped_file_path(path: &Path) -> Result<RgbaImage> {
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let img = image::load_from_memory(&bytes)?;
    Ok(rgba_opaque(img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn opaque_image_untouched() {
        let mut img = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
        img.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
        let out = rgba_opaque(DynamicImage::ImageRgba8(img));
        assert_eq!(out.get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn alpha_composited_over_white() {
        let mut img = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 0]));
        img.put_pixel(0, 0, Rgba([0, 0, 0, 128]));
        img.put_pixel(1, 1, Rgba([255, 0, 0, 64]));
        let out = rgba_opaque(DynamicImage::ImageRgba8(img));
        assert_eq!(out.get_pixel(1, 1).0[3], 255);
        assert_eq!(out.get_pixel(0, 0), &Rgba([127, 127, 127, 255]));
        assert!(out.get_pixel(1, 1).0[0] > 200);
    }
}
