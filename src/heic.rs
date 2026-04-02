use std::path::Path;

use image::{DynamicImage, RgbImage, RgbaImage};
use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

use crate::error::ImgstripError;

/// Decode a HEIC/HEIF file into a DynamicImage.
pub fn decode_heic(path: &Path) -> Result<DynamicImage, ImgstripError> {
    let path_str = path.to_str().ok_or_else(|| {
        ImgstripError::HeicError(format!("invalid UTF-8 in path: {}", path.display()))
    })?;

    let ctx = HeifContext::read_from_file(path_str).map_err(|e| {
        ImgstripError::HeicError(format!("failed to read HEIC file: {e}"))
    })?;

    let handle = ctx.primary_image_handle().map_err(|e| {
        ImgstripError::HeicError(format!("failed to get primary image handle: {e}"))
    })?;

    let has_alpha = handle.has_alpha_channel();
    let width = handle.width();
    let height = handle.height();

    let lib_heif = LibHeif::new();

    // Decode to interleaved RGB or RGBA depending on alpha presence
    let color_space = if has_alpha {
        ColorSpace::Rgb(RgbChroma::Rgba)
    } else {
        ColorSpace::Rgb(RgbChroma::Rgb)
    };

    let image = lib_heif.decode(&handle, color_space, None).map_err(|e| {
        ImgstripError::HeicError(format!("failed to decode HEIC image: {e}"))
    })?;

    let planes = image.planes();
    let interleaved = planes.interleaved.ok_or_else(|| {
        ImgstripError::HeicError("no interleaved plane in decoded HEIC image".to_string())
    })?;

    let stride = interleaved.stride;
    let data = interleaved.data;

    if has_alpha {
        // RGBA: 4 bytes per pixel
        let bytes_per_row = (width as usize) * 4;
        let mut pixels = Vec::with_capacity((width * height) as usize * 4);
        for row in 0..height as usize {
            let row_start = row * stride;
            pixels.extend_from_slice(&data[row_start..row_start + bytes_per_row]);
        }
        let img = RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
            ImgstripError::HeicError("failed to construct RGBA image from decoded data".to_string())
        })?;
        Ok(DynamicImage::ImageRgba8(img))
    } else {
        // RGB: 3 bytes per pixel
        let bytes_per_row = (width as usize) * 3;
        let mut pixels = Vec::with_capacity((width * height) as usize * 3);
        for row in 0..height as usize {
            let row_start = row * stride;
            pixels.extend_from_slice(&data[row_start..row_start + bytes_per_row]);
        }
        let img = RgbImage::from_raw(width, height, pixels).ok_or_else(|| {
            ImgstripError::HeicError("failed to construct RGB image from decoded data".to_string())
        })?;
        Ok(DynamicImage::ImageRgb8(img))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const HEIC_FIXTURE: &str = "tests/fixtures/sample.heic";

    #[test]
    fn decode_heic_valid_file() {
        let img = decode_heic(Path::new(HEIC_FIXTURE)).unwrap();
        assert_eq!(img.width(), 64);
        assert_eq!(img.height(), 64);
    }

    #[test]
    fn decode_heic_nonexistent_file() {
        let err = decode_heic(Path::new("tests/fixtures/nonexistent.heic")).unwrap_err();
        assert!(matches!(err, ImgstripError::HeicError(_)));
    }

    #[test]
    fn decode_heic_not_a_heic_file() {
        // A JPEG file renamed to .heic — libheif should reject it
        let err = decode_heic(Path::new("tests/fixtures/sample.jpg")).unwrap_err();
        assert!(matches!(err, ImgstripError::HeicError(_)));
    }

    #[test]
    fn decode_heic_truncated_file() {
        // Create a truncated HEIC file (just the ftyp header)
        let tmp = tempfile::Builder::new()
            .suffix(".heic")
            .tempfile()
            .unwrap();
        std::fs::write(
            tmp.path(),
            &[
                0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, b'h', b'e', b'i', b'c',
            ],
        )
        .unwrap();
        let err = decode_heic(tmp.path()).unwrap_err();
        assert!(matches!(err, ImgstripError::HeicError(_)));
    }
}
