use std::fs;
use std::path::Path;

use crate::error::ImgstripError;
use crate::formats::{self, ImageFormat};

/// Strip all metadata from an image file.
/// If `output` is `None`, strip in place. Otherwise write stripped copy to `output`.
pub fn strip(input: &Path, output: Option<&Path>) -> Result<(), ImgstripError> {
    let format = formats::detect_format(input)?;

    match format {
        ImageFormat::Jpeg => strip_jpeg(input, output),
        ImageFormat::Png => strip_png(input, output),
        ImageFormat::WebP | ImageFormat::Tiff | ImageFormat::Heic => {
            strip_with_little_exif(input, output, format)
        }
        ImageFormat::Bmp => {
            eprintln!("Warning: BMP files do not contain metadata to strip");
            if let Some(out) = output {
                copy_file(input, out)?;
            }
            Ok(())
        }
        ImageFormat::Gif => {
            eprintln!("Warning: GIF files do not contain EXIF metadata to strip");
            if let Some(out) = output {
                copy_file(input, out)?;
            }
            Ok(())
        }
    }
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), ImgstripError> {
    fs::copy(src, dst).map_err(|e| ImgstripError::IoError {
        path: dst.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Lossless JPEG metadata stripping via img-parts.
/// Removes EXIF, XMP, ICC, IPTC, and comment segments.
fn strip_jpeg(input: &Path, output: Option<&Path>) -> Result<(), ImgstripError> {
    use img_parts::jpeg::{markers, Jpeg};
    use img_parts::{ImageEXIF, ImageICC};

    let data = read_file(input)?;
    let mut jpeg = Jpeg::from_bytes(data.into())
        .map_err(|e| ImgstripError::MetadataError(format!("failed to parse JPEG: {e}")))?;

    // Remove EXIF (APP1 segments with "Exif\0\0" prefix)
    jpeg.set_exif(None);

    // Remove XMP (APP1 segments with XMP namespace prefix)
    const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    jpeg.segments_mut()
        .retain(|seg| !(seg.marker() == markers::APP1 && seg.contents().starts_with(XMP_PREFIX)));

    // Remove ICC profiles (APP2 segments with "ICC_PROFILE\0" prefix)
    jpeg.set_icc_profile(None);

    // Remove IPTC (APP13 segments)
    jpeg.remove_segments_by_marker(markers::APP13);

    // Remove comment segments
    jpeg.remove_segments_by_marker(markers::COM);

    let target = output.unwrap_or(input);
    let file = fs::File::create(target).map_err(|e| ImgstripError::IoError {
        path: target.to_path_buf(),
        source: e,
    })?;
    jpeg.encoder()
        .write_to(file)
        .map_err(|e| ImgstripError::IoError {
            path: target.to_path_buf(),
            source: e,
        })?;

    Ok(())
}

/// Lossless PNG metadata stripping via img-parts.
/// Removes eXIf, iCCP, tEXt, iTXt, and zTXt chunks.
fn strip_png(input: &Path, output: Option<&Path>) -> Result<(), ImgstripError> {
    use img_parts::png::Png;
    use img_parts::{ImageEXIF, ImageICC};

    let data = read_file(input)?;
    let mut png = Png::from_bytes(data.into())
        .map_err(|e| ImgstripError::MetadataError(format!("failed to parse PNG: {e}")))?;

    // Remove EXIF chunk
    png.set_exif(None);

    // Remove ICC profile chunk
    png.set_icc_profile(None);

    // Remove text metadata chunks
    png.remove_chunks_by_type(*b"tEXt");
    png.remove_chunks_by_type(*b"iTXt");
    png.remove_chunks_by_type(*b"zTXt");

    let target = output.unwrap_or(input);
    let file = fs::File::create(target).map_err(|e| ImgstripError::IoError {
        path: target.to_path_buf(),
        source: e,
    })?;
    png.encoder()
        .write_to(file)
        .map_err(|e| ImgstripError::IoError {
            path: target.to_path_buf(),
            source: e,
        })?;

    Ok(())
}

/// Metadata stripping for WebP, TIFF, and HEIC via little_exif.
fn strip_with_little_exif(
    input: &Path,
    output: Option<&Path>,
    format: ImageFormat,
) -> Result<(), ImgstripError> {
    use little_exif::filetype::FileExtension;
    use little_exif::metadata::Metadata;

    let file_type = match format {
        ImageFormat::WebP => FileExtension::WEBP,
        ImageFormat::Tiff => FileExtension::TIFF,
        ImageFormat::Heic => FileExtension::HEIF,
        _ => unreachable!(),
    };

    let mut data = read_file(input)?;

    Metadata::clear_metadata(&mut data, file_type).map_err(|e| {
        ImgstripError::MetadataError(format!("failed to strip metadata: {e}"))
    })?;

    let target = output.unwrap_or(input);
    fs::write(target, &data).map_err(|e| ImgstripError::IoError {
        path: target.to_path_buf(),
        source: e,
    })?;

    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>, ImgstripError> {
    fs::read(path).map_err(|e| ImgstripError::IoError {
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use img_parts::jpeg::{markers, Jpeg, JpegSegment};
    use img_parts::png::{Png, PngChunk};
    use img_parts::{Bytes, ImageEXIF};
    use std::io::BufWriter;
    use tempfile::TempDir;

    /// Create a minimal JPEG file and inject fake EXIF + IPTC + comment metadata.
    fn create_jpeg_with_metadata(path: &Path) {
        // First create a bare JPEG with the image crate
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        let file = fs::File::create(path).unwrap();
        let writer = BufWriter::new(file);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, 90);
        image::ImageEncoder::write_image(
            encoder,
            img.as_bytes(),
            img.width(),
            img.height(),
            img.color().into(),
        )
        .unwrap();

        // Now inject metadata segments using img-parts
        let data = fs::read(path).unwrap();
        let mut jpeg = Jpeg::from_bytes(data.into()).unwrap();

        // Add fake EXIF APP1 segment
        let exif_payload = {
            let mut v = b"Exif\0\0".to_vec();
            v.extend_from_slice(b"fake-exif-data-camera-gps-timestamps");
            v
        };
        jpeg.segments_mut()
            .insert(0, JpegSegment::new_with_contents(markers::APP1, Bytes::from(exif_payload)));

        // Add fake XMP APP1 segment
        let xmp_payload = {
            let mut v = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
            v.extend_from_slice(b"<x:xmpmeta>fake-xmp</x:xmpmeta>");
            v
        };
        jpeg.segments_mut()
            .insert(1, JpegSegment::new_with_contents(markers::APP1, Bytes::from(xmp_payload)));

        // Add fake IPTC APP13 segment
        let iptc_payload = {
            let mut v = b"Photoshop 3.0\0".to_vec();
            v.extend_from_slice(b"fake-iptc-data");
            v
        };
        jpeg.segments_mut()
            .insert(2, JpegSegment::new_with_contents(markers::APP13, Bytes::from(iptc_payload)));

        // Add a comment segment
        jpeg.segments_mut().insert(
            3,
            JpegSegment::new_with_contents(markers::COM, Bytes::from_static(b"test comment")),
        );

        let file = fs::File::create(path).unwrap();
        jpeg.encoder().write_to(file).unwrap();
    }

    /// Create a minimal PNG file and inject tEXt metadata.
    fn create_png_with_metadata(path: &Path) {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        img.save_with_format(path, image::ImageFormat::Png).unwrap();

        let data = fs::read(path).unwrap();
        let mut png = Png::from_bytes(data.into()).unwrap();

        // Add a tEXt chunk: keyword\0value
        let text_payload = b"Comment\0This is a test comment with metadata";
        let chunk = PngChunk::new(*b"tEXt", Bytes::from(&text_payload[..]));
        // Insert before the IEND chunk (last chunk)
        let len = png.chunks().len();
        png.chunks_mut().insert(len.saturating_sub(1), chunk);

        // Add a fake eXIf chunk
        let exif_data = b"Exif\0\0fake-exif-data";
        png.set_exif(Some(Bytes::from(&exif_data[..])));

        let file = fs::File::create(path).unwrap();
        png.encoder().write_to(file).unwrap();
    }

    fn has_jpeg_exif(path: &Path) -> bool {
        let data = fs::read(path).unwrap();
        let jpeg = Jpeg::from_bytes(data.into()).unwrap();
        jpeg.exif().is_some()
    }

    fn has_jpeg_xmp(path: &Path) -> bool {
        let data = fs::read(path).unwrap();
        let jpeg = Jpeg::from_bytes(data.into()).unwrap();
        const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
        jpeg.segments()
            .iter()
            .any(|seg| seg.marker() == markers::APP1 && seg.contents().starts_with(XMP_PREFIX))
    }

    fn has_jpeg_iptc(path: &Path) -> bool {
        let data = fs::read(path).unwrap();
        let jpeg = Jpeg::from_bytes(data.into()).unwrap();
        jpeg.segments()
            .iter()
            .any(|seg| seg.marker() == markers::APP13)
    }

    fn has_jpeg_comment(path: &Path) -> bool {
        let data = fs::read(path).unwrap();
        let jpeg = Jpeg::from_bytes(data.into()).unwrap();
        jpeg.segments()
            .iter()
            .any(|seg| seg.marker() == markers::COM)
    }

    fn has_png_text(path: &Path) -> bool {
        let data = fs::read(path).unwrap();
        let png = Png::from_bytes(data.into()).unwrap();
        png.chunks()
            .iter()
            .any(|c| matches!(&c.kind(), b"tEXt" | b"iTXt" | b"zTXt"))
    }

    fn has_png_exif(path: &Path) -> bool {
        let data = fs::read(path).unwrap();
        let png = Png::from_bytes(data.into()).unwrap();
        png.exif().is_some()
    }

    // --- JPEG stripping tests ---

    #[test]
    fn strip_jpeg_removes_exif() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        assert!(has_jpeg_exif(&path), "precondition: EXIF should be present");
        strip(&path, None).unwrap();
        assert!(
            !has_jpeg_exif(&path),
            "EXIF should be removed after stripping"
        );
    }

    #[test]
    fn strip_jpeg_removes_xmp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        assert!(has_jpeg_xmp(&path), "precondition: XMP should be present");
        strip(&path, None).unwrap();
        assert!(
            !has_jpeg_xmp(&path),
            "XMP should be removed after stripping"
        );
    }

    #[test]
    fn strip_jpeg_removes_iptc() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        assert!(
            has_jpeg_iptc(&path),
            "precondition: IPTC should be present"
        );
        strip(&path, None).unwrap();
        assert!(
            !has_jpeg_iptc(&path),
            "IPTC should be removed after stripping"
        );
    }

    #[test]
    fn strip_jpeg_removes_comments() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        assert!(
            has_jpeg_comment(&path),
            "precondition: comment should be present"
        );
        strip(&path, None).unwrap();
        assert!(
            !has_jpeg_comment(&path),
            "comments should be removed after stripping"
        );
    }

    #[test]
    fn strip_jpeg_file_size_decreases() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        let size_before = fs::metadata(&path).unwrap().len();
        strip(&path, None).unwrap();
        let size_after = fs::metadata(&path).unwrap().len();

        assert!(
            size_after < size_before,
            "file should shrink after metadata removal ({size_before} -> {size_after})"
        );
    }

    #[test]
    fn strip_jpeg_image_still_valid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        strip(&path, None).unwrap();

        let img = image::open(&path).expect("stripped JPEG should still be decodable");
        assert_eq!(img.width(), 8);
        assert_eq!(img.height(), 8);
    }

    // --- PNG stripping tests ---

    #[test]
    fn strip_png_removes_text_chunks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        create_png_with_metadata(&path);

        assert!(
            has_png_text(&path),
            "precondition: tEXt chunk should be present"
        );
        strip(&path, None).unwrap();
        assert!(
            !has_png_text(&path),
            "tEXt chunks should be removed after stripping"
        );
    }

    #[test]
    fn strip_png_removes_exif() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        create_png_with_metadata(&path);

        assert!(
            has_png_exif(&path),
            "precondition: eXIf chunk should be present"
        );
        strip(&path, None).unwrap();
        assert!(
            !has_png_exif(&path),
            "eXIf chunk should be removed after stripping"
        );
    }

    #[test]
    fn strip_png_image_still_valid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        create_png_with_metadata(&path);

        strip(&path, None).unwrap();

        let img = image::open(&path).expect("stripped PNG should still be decodable");
        assert_eq!(img.width(), 8);
        assert_eq!(img.height(), 8);
    }

    // --- Output mode tests ---

    #[test]
    fn strip_jpeg_output_mode_preserves_original() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("photo.jpg");
        let output = dir.path().join("stripped.jpg");
        create_jpeg_with_metadata(&input);

        let original_data = fs::read(&input).unwrap();
        strip(&input, Some(&output)).unwrap();

        // Original should be untouched
        assert_eq!(fs::read(&input).unwrap(), original_data);
        // Output should exist and have no EXIF
        assert!(output.exists());
        assert!(!has_jpeg_exif(&output));
    }

    #[test]
    fn strip_png_output_mode_preserves_original() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("photo.png");
        let output = dir.path().join("stripped.png");
        create_png_with_metadata(&input);

        let original_data = fs::read(&input).unwrap();
        strip(&input, Some(&output)).unwrap();

        assert_eq!(fs::read(&input).unwrap(), original_data);
        assert!(output.exists());
        assert!(!has_png_exif(&output));
    }

    // --- BMP / GIF no-op tests ---

    #[test]
    fn strip_bmp_succeeds_with_no_op() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.bmp");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        img.save_with_format(&path, image::ImageFormat::Bmp)
            .unwrap();

        let data_before = fs::read(&path).unwrap();
        strip(&path, None).unwrap();
        let data_after = fs::read(&path).unwrap();

        // File should be untouched (no-op)
        assert_eq!(data_before, data_after);
    }

    #[test]
    fn strip_bmp_output_mode_copies_file() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("photo.bmp");
        let output = dir.path().join("stripped.bmp");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |_, _| {
            image::Rgb([128, 128, 128])
        }));
        img.save_with_format(&input, image::ImageFormat::Bmp)
            .unwrap();

        strip(&input, Some(&output)).unwrap();
        assert!(output.exists());
        assert_eq!(fs::read(&input).unwrap(), fs::read(&output).unwrap());
    }

    #[test]
    fn strip_gif_succeeds_with_no_op() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.gif");
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([(x * 30) as u8, (y * 30) as u8, 128, 255])
        }));
        img.save_with_format(&path, image::ImageFormat::Gif)
            .unwrap();

        let data_before = fs::read(&path).unwrap();
        strip(&path, None).unwrap();
        let data_after = fs::read(&path).unwrap();

        assert_eq!(data_before, data_after);
    }

    // --- WebP stripping test ---

    #[test]
    fn strip_webp_succeeds() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.webp");
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([(x * 30) as u8, (y * 30) as u8, 128, 255])
        }));
        // WebP encoding via image crate (lossless)
        let file = fs::File::create(&path).unwrap();
        let writer = BufWriter::new(file);
        let encoder = image::codecs::webp::WebPEncoder::new_lossless(writer);
        image::ImageEncoder::write_image(
            encoder,
            img.to_rgba8().as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();

        strip(&path, None).unwrap();

        // File should still be a valid WebP
        let decoded = image::open(&path).expect("stripped WebP should still be decodable");
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 8);
    }

    // --- Error case ---

    #[test]
    fn strip_nonexistent_file_returns_error() {
        let err = strip(Path::new("/tmp/does_not_exist_imgstrip.jpg"), None).unwrap_err();
        assert!(matches!(
            err,
            ImgstripError::IoError { .. } | ImgstripError::MetadataError(_)
        ));
    }
}
