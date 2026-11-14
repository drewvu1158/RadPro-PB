//! Writing composed strips and individual shots to disk.

use image::DynamicImage;
use std::io;
use std::path::{Path, PathBuf};

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn save_jpeg(image: &DynamicImage, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image
        .save_with_format(path, image::ImageFormat::Jpeg)
        .map_err(io::Error::other)
}

/// Save the composed strip; returns the path written.
pub fn save_strip(strip: &DynamicImage, dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(format!("strip-{}.jpg", timestamp()));
    save_jpeg(strip, &path)?;
    Ok(path)
}

/// Save one individual shot; returns the path written.
pub fn save_shot(shot: &DynamicImage, dir: &Path, index: u32) -> io::Result<PathBuf> {
    let path = dir.join(format!("shot-{}-{}.jpg", timestamp(), index + 1));
    save_jpeg(shot, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn save_strip_writes_jpeg() {
        let dir = std::env::temp_dir().join(format!(
            "sparkbooth-rust-test-{}-storage",
            std::process::id()
        ));
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(8, 8, Rgb([7, 7, 7])));
        let path = save_strip(&img, &dir).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] == 0xD8); // JPEG SOI
        let _ = std::fs::remove_dir_all(&dir);
    }
}
