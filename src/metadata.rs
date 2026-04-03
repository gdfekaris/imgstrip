use std::fs;
use std::path::Path;

use crate::error::ImgstripError;
use crate::formats::{self, ImageFormat, OutputFormat};
use crate::heic;

/// Raw metadata bytes extracted from a source image.
/// Carries opaque bytes to avoid lossy round-tripping through parsed structures.
#[derive(Debug, Default, Clone)]
pub struct MetadataBundle {
    /// Raw EXIF bytes (TIFF-header-prefixed). No "Exif\0\0" wrapper.
    pub exif: Option<Vec<u8>>,
    /// Raw XMP bytes (the XML payload, no container prefix).
    pub xmp: Option<Vec<u8>>,
    /// Raw ICC profile bytes.
    pub icc: Option<Vec<u8>>,
}

impl MetadataBundle {
    /// Returns true if all metadata fields are empty.
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.xmp.is_none() && self.icc.is_none()
    }
}

/// Extract metadata from an image file into a MetadataBundle.
pub fn extract(input: &Path) -> Result<MetadataBundle, ImgstripError> {
    let format = formats::detect_format(input)?;

    match format {
        ImageFormat::Jpeg => extract_jpeg(input),
        ImageFormat::Png => extract_png(input),
        ImageFormat::WebP => extract_webp(input),
        ImageFormat::Tiff => extract_tiff(input),
        ImageFormat::Heic => heic::extract_metadata(input),
        ImageFormat::Bmp | ImageFormat::Gif => Ok(MetadataBundle::default()),
    }
}

fn extract_jpeg(input: &Path) -> Result<MetadataBundle, ImgstripError> {
    use img_parts::jpeg::{Jpeg, markers};
    use img_parts::{ImageEXIF, ImageICC};

    let data = read_file(input)?;
    let jpeg = Jpeg::from_bytes(data.into())
        .map_err(|e| ImgstripError::MetadataError(format!("failed to parse JPEG: {e}")))?;

    let exif = jpeg.exif().map(|b| b.to_vec());

    const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    let xmp = jpeg.segments().iter().find_map(|seg| {
        if seg.marker() == markers::APP1 && seg.contents().starts_with(XMP_PREFIX) {
            Some(seg.contents()[XMP_PREFIX.len()..].to_vec())
        } else {
            None
        }
    });

    let icc = jpeg.icc_profile().map(|b| b.to_vec());

    Ok(MetadataBundle { exif, xmp, icc })
}

fn extract_png(input: &Path) -> Result<MetadataBundle, ImgstripError> {
    use img_parts::png::Png;
    use img_parts::{ImageEXIF, ImageICC};

    let data = read_file(input)?;
    let png = Png::from_bytes(data.into())
        .map_err(|e| ImgstripError::MetadataError(format!("failed to parse PNG: {e}")))?;

    let exif = png.exif().map(|b| b.to_vec());
    let icc = png.icc_profile().map(|b| b.to_vec());

    Ok(MetadataBundle {
        exif,
        xmp: None,
        icc,
    })
}

fn extract_webp(input: &Path) -> Result<MetadataBundle, ImgstripError> {
    use img_parts::webp::{CHUNK_XMP, WebP};
    use img_parts::{ImageEXIF, ImageICC};

    let data = read_file(input)?;
    let webp = WebP::from_bytes(data.into())
        .map_err(|e| ImgstripError::MetadataError(format!("failed to parse WebP: {e}")))?;

    let exif = webp.exif().map(|b| b.to_vec());
    let icc = webp.icc_profile().map(|b| b.to_vec());
    let xmp = webp
        .chunk_by_id(CHUNK_XMP)
        .and_then(|c| c.content().data())
        .map(|b| b.to_vec());

    Ok(MetadataBundle { exif, xmp, icc })
}

/// Extract metadata from a TIFF file.
/// Limitation: little_exif only reads EXIF IFD entries from TIFF files.
/// XMP and ICC profiles in TIFF files are not extracted (img-parts has no TIFF support).
fn extract_tiff(input: &Path) -> Result<MetadataBundle, ImgstripError> {
    use little_exif::metadata::Metadata;

    let metadata = Metadata::new_from_path(input)
        .map_err(|e| ImgstripError::MetadataError(format!("failed to read TIFF metadata: {e}")))?;

    let exif = metadata.encode().ok().filter(|v| !v.is_empty());

    Ok(MetadataBundle {
        exif,
        xmp: None,
        icc: None,
    })
}

/// Inject metadata from a MetadataBundle into an already-encoded output file.
/// Handles JPEG, PNG, and WebP. BMP/GIF are no-ops.
pub fn inject(
    output: &Path,
    format: OutputFormat,
    bundle: &MetadataBundle,
) -> Result<(), ImgstripError> {
    if bundle.is_empty() {
        return Ok(());
    }

    match format {
        OutputFormat::Jpeg => inject_jpeg(output, bundle),
        OutputFormat::Png => inject_png(output, bundle),
        OutputFormat::WebP => inject_webp(output, bundle),
        OutputFormat::Bmp | OutputFormat::Gif | OutputFormat::Tiff => Ok(()),
    }
}

fn inject_jpeg(output: &Path, bundle: &MetadataBundle) -> Result<(), ImgstripError> {
    use img_parts::jpeg::{Jpeg, JpegSegment, markers};
    use img_parts::{Bytes, ImageEXIF, ImageICC};

    let data = read_file(output)?;
    let mut jpeg = Jpeg::from_bytes(data.into()).map_err(|e| {
        ImgstripError::MetadataError(format!("failed to parse JPEG for injection: {e}"))
    })?;

    if let Some(ref exif) = bundle.exif {
        jpeg.set_exif(Some(Bytes::from(exif.clone())));
    }

    if let Some(ref xmp) = bundle.xmp {
        const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
        let mut payload = XMP_PREFIX.to_vec();
        payload.extend_from_slice(xmp);
        jpeg.segments_mut().insert(
            0,
            JpegSegment::new_with_contents(markers::APP1, Bytes::from(payload)),
        );
    }

    if let Some(ref icc) = bundle.icc {
        jpeg.set_icc_profile(Some(Bytes::from(icc.clone())));
    }

    let file = fs::File::create(output).map_err(|e| ImgstripError::IoError {
        path: output.to_path_buf(),
        source: e,
    })?;
    jpeg.encoder()
        .write_to(file)
        .map_err(|e| ImgstripError::IoError {
            path: output.to_path_buf(),
            source: e,
        })?;

    Ok(())
}

fn inject_png(output: &Path, bundle: &MetadataBundle) -> Result<(), ImgstripError> {
    use img_parts::png::Png;
    use img_parts::{Bytes, ImageEXIF, ImageICC};

    let data = read_file(output)?;
    let mut png = Png::from_bytes(data.into()).map_err(|e| {
        ImgstripError::MetadataError(format!("failed to parse PNG for injection: {e}"))
    })?;

    if let Some(ref exif) = bundle.exif {
        png.set_exif(Some(Bytes::from(exif.clone())));
    }

    if let Some(ref icc) = bundle.icc {
        png.set_icc_profile(Some(Bytes::from(icc.clone())));
    }

    let file = fs::File::create(output).map_err(|e| ImgstripError::IoError {
        path: output.to_path_buf(),
        source: e,
    })?;
    png.encoder()
        .write_to(file)
        .map_err(|e| ImgstripError::IoError {
            path: output.to_path_buf(),
            source: e,
        })?;

    Ok(())
}

fn inject_webp(output: &Path, bundle: &MetadataBundle) -> Result<(), ImgstripError> {
    use img_parts::riff::{RiffChunk, RiffContent};
    use img_parts::webp::{CHUNK_XMP, WebP};
    use img_parts::{Bytes, ImageEXIF, ImageICC};

    let data = read_file(output)?;
    let mut webp = WebP::from_bytes(data.into()).map_err(|e| {
        ImgstripError::MetadataError(format!("failed to parse WebP for injection: {e}"))
    })?;

    if let Some(ref exif) = bundle.exif {
        webp.set_exif(Some(Bytes::from(exif.clone())));
    }

    if let Some(ref icc) = bundle.icc {
        webp.set_icc_profile(Some(Bytes::from(icc.clone())));
    }

    if let Some(ref xmp) = bundle.xmp {
        webp.remove_chunks_by_id(CHUNK_XMP);
        let chunk = RiffChunk::new(CHUNK_XMP, RiffContent::Data(Bytes::from(xmp.clone())));
        webp.chunks_mut().push(chunk);
    }

    let file = fs::File::create(output).map_err(|e| ImgstripError::IoError {
        path: output.to_path_buf(),
        source: e,
    })?;
    webp.encoder()
        .write_to(file)
        .map_err(|e| ImgstripError::IoError {
            path: output.to_path_buf(),
            source: e,
        })?;

    Ok(())
}

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
    use img_parts::jpeg::{Jpeg, markers};
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

    Metadata::clear_metadata(&mut data, file_type)
        .map_err(|e| ImgstripError::MetadataError(format!("failed to strip metadata: {e}")))?;

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
    use img_parts::jpeg::{Jpeg, JpegSegment, markers};
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
        jpeg.segments_mut().insert(
            0,
            JpegSegment::new_with_contents(markers::APP1, Bytes::from(exif_payload)),
        );

        // Add fake XMP APP1 segment
        let xmp_payload = {
            let mut v = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
            v.extend_from_slice(b"<x:xmpmeta>fake-xmp</x:xmpmeta>");
            v
        };
        jpeg.segments_mut().insert(
            1,
            JpegSegment::new_with_contents(markers::APP1, Bytes::from(xmp_payload)),
        );

        // Add fake IPTC APP13 segment
        let iptc_payload = {
            let mut v = b"Photoshop 3.0\0".to_vec();
            v.extend_from_slice(b"fake-iptc-data");
            v
        };
        jpeg.segments_mut().insert(
            2,
            JpegSegment::new_with_contents(markers::APP13, Bytes::from(iptc_payload)),
        );

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

        assert!(has_jpeg_iptc(&path), "precondition: IPTC should be present");
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

    // --- MetadataBundle tests ---

    #[test]
    fn metadata_bundle_default_is_empty() {
        let bundle = MetadataBundle::default();
        assert!(bundle.is_empty());
    }

    #[test]
    fn metadata_bundle_with_exif_is_not_empty() {
        let bundle = MetadataBundle {
            exif: Some(vec![0x4D, 0x4D]),
            xmp: None,
            icc: None,
        };
        assert!(!bundle.is_empty());
    }

    #[test]
    fn metadata_bundle_with_only_icc_is_not_empty() {
        let bundle = MetadataBundle {
            exif: None,
            xmp: None,
            icc: Some(vec![0x00]),
        };
        assert!(!bundle.is_empty());
    }

    // --- Extract tests ---

    #[test]
    fn extract_jpeg_with_metadata() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_metadata(&path);

        let bundle = extract(&path).unwrap();
        assert!(bundle.exif.is_some(), "should extract EXIF from JPEG");
        assert!(bundle.xmp.is_some(), "should extract XMP from JPEG");
    }

    #[test]
    fn extract_jpeg_no_metadata() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bare.jpg");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        let file = fs::File::create(&path).unwrap();
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

        let bundle = extract(&path).unwrap();
        assert!(bundle.exif.is_none(), "bare JPEG should have no EXIF");
        assert!(bundle.xmp.is_none(), "bare JPEG should have no XMP");
    }

    #[test]
    fn extract_png_with_exif() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        create_png_with_metadata(&path);

        let bundle = extract(&path).unwrap();
        assert!(bundle.exif.is_some(), "should extract EXIF from PNG");
    }

    #[test]
    fn extract_bmp_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.bmp");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        img.save_with_format(&path, image::ImageFormat::Bmp)
            .unwrap();

        let bundle = extract(&path).unwrap();
        assert!(bundle.is_empty(), "BMP should return empty bundle");
    }

    #[test]
    fn extract_gif_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.gif");
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([(x * 30) as u8, (y * 30) as u8, 128, 255])
        }));
        img.save_with_format(&path, image::ImageFormat::Gif)
            .unwrap();

        let bundle = extract(&path).unwrap();
        assert!(bundle.is_empty(), "GIF should return empty bundle");
    }

    // --- Inject tests ---

    #[test]
    fn inject_jpeg_exif_and_read_back() {
        use img_parts::ImageICC;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        // Create bare JPEG
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        let file = fs::File::create(&path).unwrap();
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

        // Inject fake metadata
        let bundle = MetadataBundle {
            exif: Some(b"MM\x00\x2A\x00\x00\x00\x08\x00\x00".to_vec()),
            xmp: Some(b"<x:xmpmeta>test</x:xmpmeta>".to_vec()),
            icc: Some(vec![0x00; 32]),
        };
        inject(&path, OutputFormat::Jpeg, &bundle).unwrap();

        // Verify
        let data = fs::read(&path).unwrap();
        let jpeg = Jpeg::from_bytes(data.into()).unwrap();
        assert!(
            jpeg.exif().is_some(),
            "EXIF should be present after injection"
        );
        assert!(
            jpeg.icc_profile().is_some(),
            "ICC should be present after injection"
        );

        // Verify image is still valid
        let decoded = image::open(&path).expect("JPEG should still be decodable after injection");
        assert_eq!(decoded.width(), 8);
    }

    #[test]
    fn inject_png_exif_and_read_back() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        img.save_with_format(&path, image::ImageFormat::Png)
            .unwrap();

        let bundle = MetadataBundle {
            exif: Some(b"MM\x00\x2A\x00\x00\x00\x08\x00\x00".to_vec()),
            xmp: None,
            icc: None,
        };
        inject(&path, OutputFormat::Png, &bundle).unwrap();

        let data = fs::read(&path).unwrap();
        let png = Png::from_bytes(data.into()).unwrap();
        assert!(
            png.exif().is_some(),
            "EXIF should be present in PNG after injection"
        );

        let decoded = image::open(&path).expect("PNG should still be decodable after injection");
        assert_eq!(decoded.width(), 8);
    }

    #[test]
    fn inject_empty_bundle_is_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        let file = fs::File::create(&path).unwrap();
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

        let data_before = fs::read(&path).unwrap();
        inject(&path, OutputFormat::Jpeg, &MetadataBundle::default()).unwrap();
        let data_after = fs::read(&path).unwrap();
        assert_eq!(
            data_before, data_after,
            "empty bundle should not modify file"
        );
    }

    #[test]
    fn inject_bmp_is_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.bmp");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        img.save_with_format(&path, image::ImageFormat::Bmp)
            .unwrap();

        let data_before = fs::read(&path).unwrap();
        let bundle = MetadataBundle {
            exif: Some(vec![0x4D, 0x4D]),
            xmp: None,
            icc: None,
        };
        inject(&path, OutputFormat::Bmp, &bundle).unwrap();
        let data_after = fs::read(&path).unwrap();
        assert_eq!(data_before, data_after, "BMP injection should be no-op");
    }
}
