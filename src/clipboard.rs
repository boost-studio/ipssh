use anyhow::{Context, Result};
use arboard::Clipboard;
use image::{ImageBuffer, ImageFormat, Rgba};
use std::borrow::Cow;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    Image(ClipboardImage),
    Text(String),
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl ClipboardImage {
    pub fn save_png(&self, path: &Path) -> Result<()> {
        let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(self.width as u32, self.height as u32, self.rgba.clone())
                .context("clipboard image buffer dimensions do not match image data")?;
        buffer
            .save_with_format(path, ImageFormat::Png)
            .with_context(|| format!("failed to save PNG {}", path.display()))
    }
}

pub trait ClipboardReader {
    fn read(&mut self) -> Result<ClipboardContent>;
}

pub struct SystemClipboard {
    inner: Clipboard,
}

impl SystemClipboard {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: Clipboard::new().context("failed to open Windows clipboard")?,
        })
    }
}

impl ClipboardReader for SystemClipboard {
    fn read(&mut self) -> Result<ClipboardContent> {
        if let Ok(image) = self.inner.get_image() {
            let bytes: Cow<'_, [u8]> = image.bytes;
            return Ok(ClipboardContent::Image(ClipboardImage {
                width: image.width,
                height: image.height,
                rgba: bytes.into_owned(),
            }));
        }

        if let Ok(text) = self.inner.get_text() {
            if !text.is_empty() {
                return Ok(ClipboardContent::Text(text));
            }
        }

        Ok(ClipboardContent::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn saves_png_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("image.png");
        let image = ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        };

        image.save_png(&path).unwrap();
        assert!(path.exists());
        assert!(std::fs::metadata(path).unwrap().len() > 0);
    }
}
