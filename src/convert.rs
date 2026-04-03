use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;

use crate::error::ImgstripError;
use crate::formats::{self, ImageFormat, OutputFormat, default_extension};
use crate::heic;
use crate::metadata;

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
    strip_metadata: bool,
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

    // Extract metadata before conversion (if we need to preserve it)
    let bundle = if strip_metadata {
        None
    } else {
        match metadata::extract(input) {
            Ok(b) if b.is_empty() => None,
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("Warning: failed to extract metadata: {e}");
                None
            }
        }
    };

    // Decode — use libheif for HEIC, image crate for everything else
    let detected = formats::detect_format(input)?;
    let img = if detected == ImageFormat::Heic {
        heic::decode_heic(input)?
    } else {
        image::open(input).map_err(|e| ImgstripError::DecodeError(e.to_string()))?
    };

    // Encode to target format (each arm manages its own file handle)
    match format {
        OutputFormat::Jpeg => {
            let out_file = fs::File::create(output).map_err(|e| ImgstripError::IoError {
                path: output.to_path_buf(),
                source: e,
            })?;
            let writer = BufWriter::new(out_file);
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
            let out_file = fs::File::create(output).map_err(|e| ImgstripError::IoError {
                path: output.to_path_buf(),
                source: e,
            })?;
            let writer = BufWriter::new(out_file);
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

    // Inject metadata into the encoded output
    if let Some(ref bundle) = bundle {
        match format {
            OutputFormat::Bmp | OutputFormat::Gif => {
                // These formats cannot carry metadata
            }
            OutputFormat::Tiff => {
                // TIFF metadata injection is not supported: little_exif's write_to_file
                // overwrites the entire TIFF (including pixel data), and img-parts has
                // no TIFF support. Metadata is silently lost for TIFF output.
                eprintln!("Warning: metadata preservation for TIFF output is not yet supported");
            }
            _ => {
                if let Err(e) = metadata::inject(output, format, bundle) {
                    eprintln!("Warning: failed to inject metadata: {e}");
                }
            }
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
        convert_file(&input, &output, OutputFormat::Png, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn png_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn jpeg_to_webp() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.jpg", OutputFormat::Jpeg);
        let output = dir.path().join("photo.webp");
        convert_file(&input, &output, OutputFormat::WebP, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn png_to_bmp() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.bmp");
        convert_file(&input, &output, OutputFormat::Bmp, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn webp_to_tiff() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.webp", OutputFormat::WebP);
        let output = dir.path().join("photo.tiff");
        convert_file(&input, &output, OutputFormat::Tiff, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn same_format_jpeg_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.jpg", OutputFormat::Jpeg);
        let output = dir.path().join("photo_copy.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
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
        convert_file(&input, &out_low, OutputFormat::Jpeg, 10, false, false).unwrap();
        convert_file(&input, &out_high, OutputFormat::Jpeg, 95, false, false).unwrap();

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
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        // Second without overwrite fails
        let err = convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap_err();
        assert!(matches!(err, ImgstripError::OutputExists { .. }));
    }

    #[test]
    fn overwrite_allowed() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.png", OutputFormat::Png);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        // With overwrite flag, second conversion succeeds
        convert_file(&input, &output, OutputFormat::Jpeg, 90, true, false).unwrap();
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
        convert_file(&input, &output, OutputFormat::Gif, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn gif_to_png() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.gif", OutputFormat::Gif);
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn tiff_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "photo.tiff", OutputFormat::Tiff);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    // --- HEIC conversion tests ---

    const HEIC_FIXTURE: &str = "tests/fixtures/sample.heic";

    #[test]
    fn heic_to_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = PathBuf::from(HEIC_FIXTURE);
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        assert_valid_image(&output, 64, 64);
    }

    #[test]
    fn heic_to_png() {
        let dir = TempDir::new().unwrap();
        let input = PathBuf::from(HEIC_FIXTURE);
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false, false).unwrap();
        assert_valid_image(&output, 64, 64);
    }

    #[test]
    fn heic_to_webp() {
        let dir = TempDir::new().unwrap();
        let input = PathBuf::from(HEIC_FIXTURE);
        let output = dir.path().join("photo.webp");
        convert_file(&input, &output, OutputFormat::WebP, 90, false, false).unwrap();
        assert_valid_image(&output, 64, 64);
    }

    #[test]
    fn heic_to_bmp() {
        let dir = TempDir::new().unwrap();
        let input = PathBuf::from(HEIC_FIXTURE);
        let output = dir.path().join("photo.bmp");
        convert_file(&input, &output, OutputFormat::Bmp, 90, false, false).unwrap();
        assert_valid_image(&output, 64, 64);
    }

    #[test]
    fn heic_to_tiff() {
        let dir = TempDir::new().unwrap();
        let input = PathBuf::from(HEIC_FIXTURE);
        let output = dir.path().join("photo.tiff");
        convert_file(&input, &output, OutputFormat::Tiff, 90, false, false).unwrap();
        assert_valid_image(&output, 64, 64);
    }

    // --- Phase 6: Metadata preservation tests ---

    /// Create a JPEG with fake EXIF and ICC metadata for testing metadata preservation.
    fn create_jpeg_with_metadata(dir: &Path, name: &str) -> PathBuf {
        use img_parts::jpeg::{Jpeg, JpegSegment, markers};
        use img_parts::{Bytes, ImageEXIF, ImageICC};

        let path = create_test_image(dir, name, OutputFormat::Jpeg);

        let data = fs::read(&path).unwrap();
        let mut jpeg = Jpeg::from_bytes(data.into()).unwrap();

        // Add fake EXIF (TIFF-header-prefixed)
        let exif_payload: Vec<u8> = {
            let mut v = Vec::new();
            // TIFF header: big-endian, magic 0x002A, offset to IFD0 = 8
            v.extend_from_slice(b"MM");
            v.extend_from_slice(&[0x00, 0x2A]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x08]);
            // IFD0: 1 entry
            v.extend_from_slice(&[0x00, 0x01]);
            // Tag 0x010F (Make), type ASCII (2), count 5, value "Test\0"
            v.extend_from_slice(&[0x01, 0x0F, 0x00, 0x02, 0x00, 0x00, 0x00, 0x05]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x1A]); // offset to data
            // Next IFD offset: 0 (no more IFDs)
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
            // Data for Make tag at offset 0x1A
            v.extend_from_slice(b"Test\0");
            v
        };
        jpeg.set_exif(Some(Bytes::from(exif_payload)));

        // Add fake XMP
        let xmp_payload = {
            let mut v = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
            v.extend_from_slice(b"<x:xmpmeta>test-xmp</x:xmpmeta>");
            v
        };
        jpeg.segments_mut().insert(
            0,
            JpegSegment::new_with_contents(markers::APP1, Bytes::from(xmp_payload)),
        );

        // Add fake ICC profile
        let icc_payload = vec![0x00; 64]; // minimal fake ICC
        jpeg.set_icc_profile(Some(Bytes::from(icc_payload)));

        let file = fs::File::create(&path).unwrap();
        jpeg.encoder().write_to(file).unwrap();

        path
    }

    /// Create a PNG with fake EXIF metadata.
    fn create_png_with_exif(dir: &Path, name: &str) -> PathBuf {
        use img_parts::png::Png;
        use img_parts::{Bytes, ImageEXIF};

        let path = create_test_image(dir, name, OutputFormat::Png);

        let data = fs::read(&path).unwrap();
        let mut png = Png::from_bytes(data.into()).unwrap();

        // Same fake EXIF as JPEG
        let exif_payload: Vec<u8> = {
            let mut v = Vec::new();
            v.extend_from_slice(b"MM");
            v.extend_from_slice(&[0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]);
            v.extend_from_slice(&[0x00, 0x01]);
            v.extend_from_slice(&[0x01, 0x0F, 0x00, 0x02, 0x00, 0x00, 0x00, 0x05]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x1A]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
            v.extend_from_slice(b"Test\0");
            v
        };
        png.set_exif(Some(Bytes::from(exif_payload)));

        let file = fs::File::create(&path).unwrap();
        png.encoder().write_to(file).unwrap();

        path
    }

    fn has_exif(path: &Path, format: OutputFormat) -> bool {
        let data = fs::read(path).unwrap();
        match format {
            OutputFormat::Jpeg => {
                use img_parts::ImageEXIF;
                use img_parts::jpeg::Jpeg;
                let jpeg = Jpeg::from_bytes(data.into()).unwrap();
                jpeg.exif().is_some()
            }
            OutputFormat::Png => {
                use img_parts::ImageEXIF;
                use img_parts::png::Png;
                let png = Png::from_bytes(data.into()).unwrap();
                png.exif().is_some()
            }
            OutputFormat::WebP => {
                use img_parts::ImageEXIF;
                use img_parts::webp::WebP;
                let webp = WebP::from_bytes(data.into()).unwrap();
                webp.exif().is_some()
            }
            _ => false,
        }
    }

    fn has_icc(path: &Path, format: OutputFormat) -> bool {
        let data = fs::read(path).unwrap();
        match format {
            OutputFormat::Jpeg => {
                use img_parts::ImageICC;
                use img_parts::jpeg::Jpeg;
                let jpeg = Jpeg::from_bytes(data.into()).unwrap();
                jpeg.icc_profile().is_some()
            }
            OutputFormat::Png => {
                use img_parts::ImageICC;
                use img_parts::png::Png;
                let png = Png::from_bytes(data.into()).unwrap();
                png.icc_profile().is_some()
            }
            OutputFormat::WebP => {
                use img_parts::ImageICC;
                use img_parts::webp::WebP;
                let webp = WebP::from_bytes(data.into()).unwrap();
                webp.icc_profile().is_some()
            }
            _ => false,
        }
    }

    #[test]
    fn jpeg_to_png_preserves_exif() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
        assert!(
            has_exif(&output, OutputFormat::Png),
            "output PNG should have EXIF"
        );
    }

    #[test]
    fn jpeg_to_png_preserves_icc() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false, false).unwrap();
        assert!(
            has_icc(&output, OutputFormat::Png),
            "output PNG should have ICC"
        );
    }

    #[test]
    fn jpeg_to_webp_preserves_exif() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.webp");
        convert_file(&input, &output, OutputFormat::WebP, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
        assert!(
            has_exif(&output, OutputFormat::WebP),
            "output WebP should have EXIF"
        );
    }

    #[test]
    fn jpeg_to_webp_preserves_icc() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.webp");
        convert_file(&input, &output, OutputFormat::WebP, 90, false, false).unwrap();
        assert!(
            has_icc(&output, OutputFormat::WebP),
            "output WebP should have ICC"
        );
    }

    #[test]
    fn png_to_jpeg_preserves_exif() {
        let dir = TempDir::new().unwrap();
        let input = create_png_with_exif(dir.path(), "photo.png");
        let output = dir.path().join("photo.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
        assert!(
            has_exif(&output, OutputFormat::Jpeg),
            "output JPEG should have EXIF"
        );
    }

    #[test]
    fn jpeg_to_png_strip_metadata_drops_exif() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.png");
        convert_file(&input, &output, OutputFormat::Png, 90, false, true).unwrap();
        assert_valid_image(&output, 8, 8);
        assert!(
            !has_exif(&output, OutputFormat::Png),
            "output PNG should NOT have EXIF when strip_metadata is true"
        );
    }

    #[test]
    fn jpeg_to_jpeg_preserves_exif() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo_copy.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
        assert!(
            has_exif(&output, OutputFormat::Jpeg),
            "output JPEG should have EXIF"
        );
    }

    #[test]
    fn convert_bare_image_no_metadata_succeeds() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "bare.png", OutputFormat::Png);
        let output = dir.path().join("bare.jpg");
        convert_file(&input, &output, OutputFormat::Jpeg, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn jpeg_to_tiff_does_not_corrupt_with_metadata() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.tiff");
        // Should succeed and produce a valid image even though TIFF can't preserve metadata
        convert_file(&input, &output, OutputFormat::Tiff, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn jpeg_to_bmp_does_not_error_with_metadata() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.bmp");
        // Should succeed even though BMP can't carry metadata
        convert_file(&input, &output, OutputFormat::Bmp, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn jpeg_to_gif_does_not_error_with_metadata() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let output = dir.path().join("photo.gif");
        convert_file(&input, &output, OutputFormat::Gif, 90, false, false).unwrap();
        assert_valid_image(&output, 8, 8);
    }

    #[test]
    fn extract_jpeg_round_trips_exif_bytes() {
        let dir = TempDir::new().unwrap();
        let input = create_jpeg_with_metadata(dir.path(), "photo.jpg");
        let bundle = metadata::extract(&input).unwrap();
        assert!(bundle.exif.is_some(), "JPEG with metadata should have EXIF");
        assert!(bundle.xmp.is_some(), "JPEG with metadata should have XMP");
        assert!(bundle.icc.is_some(), "JPEG with metadata should have ICC");
    }

    #[test]
    fn extract_bare_image_returns_empty_bundle() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "bare.png", OutputFormat::Png);
        let bundle = metadata::extract(&input).unwrap();
        assert!(bundle.is_empty(), "bare image should produce empty bundle");
    }

    #[test]
    fn extract_bmp_returns_empty_bundle() {
        let dir = TempDir::new().unwrap();
        let input = create_test_image(dir.path(), "bare.bmp", OutputFormat::Bmp);
        let bundle = metadata::extract(&input).unwrap();
        assert!(bundle.is_empty(), "BMP should produce empty bundle");
    }
}
