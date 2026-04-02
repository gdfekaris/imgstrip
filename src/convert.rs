use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;

use crate::error::ImgstripError;
use crate::formats::{OutputFormat, default_extension};

/// Derive the output path by replacing the input's extension with the target format's extension.
pub fn derive_output_path(input: &Path, format: OutputFormat) -> PathBuf {
    let mut output = input.to_path_buf();
    output.set_extension(default_extension(format));
    output
}

/// Convert a single image file to the specified output format.
pub fn convert_file(
    input: &Path,
    output: &Path,
    format: OutputFormat,
    quality: u8,
    overwrite: bool,
) -> Result<(), ImgstripError> {
    // Check input exists
    if !input.exists() {
        return Err(ImgstripError::IoError {
            path: input.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        });
    }

    // Check overwrite
    if output.exists() && !overwrite {
        return Err(ImgstripError::OutputExists {
            path: output.to_path_buf(),
        });
    }

    // Decode
    let img = image::open(input).map_err(|e| ImgstripError::DecodeError(e.to_string()))?;

    // Encode to target format
    let out_file = fs::File::create(output).map_err(|e| ImgstripError::IoError {
        path: output.to_path_buf(),
        source: e,
    })?;
    let writer = BufWriter::new(out_file);

    match format {
        OutputFormat::Jpeg => {
            let encoder = JpegEncoder::new_with_quality(writer, quality);
            encoder
                .write_image(
                    img.as_bytes(),
                    img.width(),
                    img.height(),
                    img.color().into(),
                )
                .map_err(|e| ImgstripError::EncodeError {
                    format: format.to_string(),
                    source: Box::new(e),
                })?;
        }
        OutputFormat::WebP => {
            // image 0.25 only supports lossless WebP encoding; quality parameter is ignored
            let rgba = img.to_rgba8();
            let encoder = WebPEncoder::new_lossless(writer);
            encoder
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| ImgstripError::EncodeError {
                    format: format.to_string(),
                    source: Box::new(e),
                })?;
        }
        _ => {
            img.save_with_format(output, format.to_image_format())
                .map_err(|e| ImgstripError::EncodeError {
                    format: format.to_string(),
                    source: Box::new(e),
                })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::parse_output_format;
    use image::{DynamicImage, RgbImage};
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Create a small 8x8 test image in the given format inside `dir`.
    fn create_test_image(dir: &Path, name: &str, format: OutputFormat) -> PathBuf {
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        let path = dir.join(name);
        match format {
            OutputFormat::Jpeg => {
                let file = fs::File::create(&path).unwrap();
                let writer = BufWriter::new(file);
                let encoder = JpegEncoder::new_with_quality(writer, 90);
                encoder
                    .write_image(
                        img.as_bytes(),
                        img.width(),
                        img.height(),
                        img.color().into(),
                    )
                    .unwrap();
            }
            _ => {
                img.save_with_format(&path, format.to_image_format())
                    .unwrap();
            }
        }
        path
    }

    fn assert_valid_image(path: &Path, expected_width: u32, expected_height: u32) {
        let img = image::open(path).expect("output should be a valid image");
        assert_eq!(img.width(), expected_width);
        assert_eq!(img.height(), expected_height);
    }

    #[test]
    fn jpeg_to_png() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.jpg", OutputFormat::Jpeg);
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn png_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn jpeg_to_webp() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.jpg", OutputFormat::Jpeg);
        let output = dir.path().join("photo.webp");
        convert_file(&input, &output, OutputFormat::WebP, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn png_to_bmp() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.bmp");
        convert_file(&input, &output, OutputFormat::Bmp, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn webp_to_tiff() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.webp", OutputFormat::WebP);
        let output = dir.path().join("photo.tiff");
        convert_file(&input, &output, OutputFormat::Tiff, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn same_format_jpeg_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.jpg", OutputFormat::Jpeg);
        let output = dir.path().join("photo_copy.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn quality_affects_file_size() {
        let dir = TempDir::new().unwrap();
        // Use a larger image so quality difference is measurable
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(100, 100, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 2) as u8, 128])
        }));
        let input = dir.path().join("big.png");
        img.save_with_format(&input, image::ImageFormat::Png)
            .unwrap();

        let out_low = dir.path().join("low.jpg");
        let out_high = dir.path().join("high.jpg");
        convert_file(&input, &out_low, OutputFormat::Jpeg, 10, false).unwrap();
        convert_file(&input, &out_high, OutputFormat::Jpeg, 95, false).unwrap();

        let size_low = fs::metadata(&out_low).unwrap().len();
        let size_high = fs::metadata(&out_high).unwrap().len();
        assert!(
            size_low < size_high,
            "quality 10 ({size_low} bytes) should be smaller than quality 95 ({size_high} bytes)"
        );
    }

    #[test]
    fn output_path_derivation() {
        let derived = derive_output_path(Path::new("photos/photo.jpg"), OutputFormat::Png);
        assert_eq!(derived, PathBuf::from("photos/photo.png"));

        let derived = derive_output_path(Path::new("image.png"), OutputFormat::Jpeg);
        assert_eq!(derived, PathBuf::from("image.jpg"));
    }

    #[test]
    fn overwrite_protection() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.jpg");
        // First conversion succeeds
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false).unwrap();
        // Second without overwrite fails
        let err = convert_file(&input, &output, OutputFormat::Jpeg, 90, false).unwrap_err();
        assert!(matches!(err, ImgstripError::OutputExists { .. }));
    }

    #[test]
    fn overwrite_allowed() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false).unwrap();
        // With overwrite flag, second conversion succeeds
        convert_file(&input, &output, OutputFormat::Jpeg, 90, true).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn invalid_output_format_string() {
        let err = parse_output_format("heic").unwrap_err();
        assert!(matches!(err, ImgstripError::UnsupportedFormat(_)));
    }

    #[test]
    fn nonexistent_input_returns_error() {
        let dir = TempDir::new().unwrap();
        let err = convert_file(
            &dir.path().join("nope.jpg"),
            &dir.path().join("out.png"),
            OutputFormat::Png,
            90,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, ImgstripError::IoError { .. }));
    }

    #[test]
    fn bmp_to_gif() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.bmp", OutputFormat::Bmp);
        let output = dir.path().join("photo.gif");
        convert_file(&input, &output, OutputFormat::Gif, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn gif_to_png() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.gif", OutputFormat::Gif);
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn tiff_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.tiff", OutputFormat::Tiff);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }
}
