use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::ImgstripError;

/// All image formats the tool can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Bmp,
    Tiff,
    Gif,
    Heic,
}

/// Formats the tool can write (HEIC excluded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Bmp,
    Tiff,
    Gif,
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageFormat::Jpeg => write!(f, "JPEG"),
            ImageFormat::Png => write!(f, "PNG"),
            ImageFormat::WebP => write!(f, "WebP"),
            ImageFormat::Bmp => write!(f, "BMP"),
            ImageFormat::Tiff => write!(f, "TIFF"),
            ImageFormat::Gif => write!(f, "GIF"),
            ImageFormat::Heic => write!(f, "HEIC"),
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputFormat::Jpeg => write!(f, "JPEG"),
            OutputFormat::Png => write!(f, "PNG"),
            OutputFormat::WebP => write!(f, "WebP"),
            OutputFormat::Bmp => write!(f, "BMP"),
            OutputFormat::Tiff => write!(f, "TIFF"),
            OutputFormat::Gif => write!(f, "GIF"),
        }
    }
}

/// Detect the image format of a file by magic bytes first, then extension.
///
/// Magic bytes are authoritative — extensions can lie (scrapers rename files,
/// CMSs re-encode without updating the extension). We only fall back to the
/// extension when the file has no recognizable magic, which covers corrupt
/// headers and empty files (so callers still surface a useful format name
/// before downstream decode fails).
pub fn detect_format(path: &Path) -> Result<ImageFormat, ImgstripError> {
    match detect_by_magic_bytes(path) {
        Ok(format) => Ok(format),
        Err(ImgstripError::UnsupportedFormat(_)) => detect_by_extension(path)
            .ok_or_else(|| ImgstripError::UnsupportedFormat(format!("{}", path.display()))),
        Err(e) => Err(e),
    }
}

/// Returns true if the file has a recognized image extension.
pub fn has_supported_extension(path: &Path) -> bool {
    detect_by_extension(path).is_some()
}

/// Detect format from file extension (case-insensitive).
fn detect_by_extension(path: &Path) -> Option<ImageFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "png" => Some(ImageFormat::Png),
        "webp" => Some(ImageFormat::WebP),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "gif" => Some(ImageFormat::Gif),
        "heic" | "heif" => Some(ImageFormat::Heic),
        _ => None,
    }
}

/// Detect format by reading magic bytes from the file.
fn detect_by_magic_bytes(path: &Path) -> Result<ImageFormat, ImgstripError> {
    let mut file = File::open(path).map_err(|e| ImgstripError::IoError {
        path: path.to_path_buf(),
        source: e,
    })?;

    let mut buf = [0u8; 12];
    let bytes_read = file.read(&mut buf).map_err(|e| ImgstripError::IoError {
        path: path.to_path_buf(),
        source: e,
    })?;

    if bytes_read == 0 {
        return Err(ImgstripError::UnsupportedFormat("empty file".to_string()));
    }

    let buf = &buf[..bytes_read];

    // JPEG: FF D8 FF
    if buf.len() >= 3 && buf[0] == 0xFF && buf[1] == 0xD8 && buf[2] == 0xFF {
        return Ok(ImageFormat::Jpeg);
    }

    // PNG: 89 50 4E 47
    if buf.len() >= 4 && buf[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return Ok(ImageFormat::Png);
    }

    // GIF: 47 49 46 38
    if buf.len() >= 4 && buf[..4] == [0x47, 0x49, 0x46, 0x38] {
        return Ok(ImageFormat::Gif);
    }

    // BMP: 42 4D
    if buf.len() >= 2 && buf[..2] == [0x42, 0x4D] {
        return Ok(ImageFormat::Bmp);
    }

    // TIFF: 49 49 2A 00 (little-endian) or 4D 4D 00 2A (big-endian)
    if buf.len() >= 4
        && (buf[..4] == [0x49, 0x49, 0x2A, 0x00] || buf[..4] == [0x4D, 0x4D, 0x00, 0x2A])
    {
        return Ok(ImageFormat::Tiff);
    }

    // WebP: RIFF....WEBP
    if buf.len() >= 12
        && buf[..4] == [0x52, 0x49, 0x46, 0x46]
        && buf[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return Ok(ImageFormat::WebP);
    }

    // HEIC: check for ftyp box at offset 4, with brand heic/heix/mif1
    if buf.len() >= 12 && buf[4..8] == [0x66, 0x74, 0x79, 0x70] {
        let brand = &buf[8..12];
        if brand == b"heic" || brand == b"heix" || brand == b"mif1" {
            return Ok(ImageFormat::Heic);
        }
    }

    Err(ImgstripError::UnsupportedFormat(format!(
        "{}",
        path.display()
    )))
}

/// Parse a user-provided format string into an OutputFormat.
pub fn parse_output_format(s: &str) -> Result<OutputFormat, ImgstripError> {
    match s.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => Ok(OutputFormat::Jpeg),
        "png" => Ok(OutputFormat::Png),
        "webp" => Ok(OutputFormat::WebP),
        "bmp" => Ok(OutputFormat::Bmp),
        "tiff" | "tif" => Ok(OutputFormat::Tiff),
        "gif" => Ok(OutputFormat::Gif),
        _ => Err(ImgstripError::UnsupportedFormat(format!(
            "unsupported output format: {s}"
        ))),
    }
}

impl ImageFormat {
    /// Map to the `image` crate's `ImageFormat`. Returns `None` for HEIC,
    /// which the `image` crate does not support — callers must dispatch HEIC
    /// to `heic::decode_heic` before reaching for this.
    pub fn to_image_format(self) -> Option<image::ImageFormat> {
        match self {
            ImageFormat::Jpeg => Some(image::ImageFormat::Jpeg),
            ImageFormat::Png => Some(image::ImageFormat::Png),
            ImageFormat::WebP => Some(image::ImageFormat::WebP),
            ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
            ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
            ImageFormat::Gif => Some(image::ImageFormat::Gif),
            ImageFormat::Heic => None,
        }
    }
}

impl OutputFormat {
    /// Convert to the `image` crate's `ImageFormat`.
    pub fn to_image_format(self) -> image::ImageFormat {
        match self {
            OutputFormat::Jpeg => image::ImageFormat::Jpeg,
            OutputFormat::Png => image::ImageFormat::Png,
            OutputFormat::WebP => image::ImageFormat::WebP,
            OutputFormat::Bmp => image::ImageFormat::Bmp,
            OutputFormat::Tiff => image::ImageFormat::Tiff,
            OutputFormat::Gif => image::ImageFormat::Gif,
        }
    }
}

/// Return the default file extension for an output format.
pub fn default_extension(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Jpeg => "jpg",
        OutputFormat::Png => "png",
        OutputFormat::WebP => "webp",
        OutputFormat::Bmp => "bmp",
        OutputFormat::Tiff => "tiff",
        OutputFormat::Gif => "gif",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_file(ext: &str, contents: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .unwrap();
        f.write_all(contents).unwrap();
        f.flush().unwrap();
        f
    }

    // --- Extension-based detection ---

    #[test]
    fn detect_jpeg_by_extension() {
        let f = write_temp_file("jpg", &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn detect_jpeg_ext_uppercase() {
        let f = write_temp_file("JPG", &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn detect_jpeg_ext_mixed_case() {
        let f = write_temp_file("Jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn detect_png_by_extension() {
        let f = write_temp_file("png", &[0x89, 0x50, 0x4E, 0x47]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn detect_png_ext_uppercase() {
        let f = write_temp_file("Png", &[0x89, 0x50, 0x4E, 0x47]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn detect_webp_by_extension() {
        let f = write_temp_file("webp", b"RIFF\x00\x00\x00\x00WEBP");
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::WebP);
    }

    #[test]
    fn detect_bmp_by_extension() {
        let f = write_temp_file("bmp", &[0x42, 0x4D, 0x00, 0x00]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Bmp);
    }

    #[test]
    fn detect_tiff_by_extension() {
        let f = write_temp_file("tiff", &[0x49, 0x49, 0x2A, 0x00]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Tiff);
    }

    #[test]
    fn detect_tif_by_extension() {
        let f = write_temp_file("tif", &[0x49, 0x49, 0x2A, 0x00]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Tiff);
    }

    #[test]
    fn detect_gif_by_extension() {
        let f = write_temp_file("gif", &[0x47, 0x49, 0x46, 0x38]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Gif);
    }

    #[test]
    fn detect_heic_by_extension() {
        // ftyp box with heic brand
        let f = write_temp_file(
            "heic",
            &[
                0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, b'h', b'e', b'i', b'c',
            ],
        );
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Heic);
    }

    #[test]
    fn detect_heif_by_extension() {
        let f = write_temp_file(
            "heif",
            &[
                0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, b'h', b'e', b'i', b'c',
            ],
        );
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Heic);
    }

    // --- Magic-byte detection (wrong extension) ---

    #[test]
    fn detect_jpeg_by_magic_wrong_ext() {
        let f = write_temp_file("dat", &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn detect_png_by_magic_wrong_ext() {
        let f = write_temp_file("dat", &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn detect_webp_by_magic_wrong_ext() {
        let f = write_temp_file("dat", b"RIFF\x00\x00\x00\x00WEBP");
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::WebP);
    }

    #[test]
    fn detect_bmp_by_magic_wrong_ext() {
        let f = write_temp_file("dat", &[0x42, 0x4D, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Bmp);
    }

    #[test]
    fn detect_tiff_le_by_magic_wrong_ext() {
        let f = write_temp_file("dat", &[0x49, 0x49, 0x2A, 0x00]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Tiff);
    }

    #[test]
    fn detect_tiff_be_by_magic_wrong_ext() {
        let f = write_temp_file("dat", &[0x4D, 0x4D, 0x00, 0x2A]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Tiff);
    }

    #[test]
    fn detect_gif_by_magic_wrong_ext() {
        let f = write_temp_file("dat", &[0x47, 0x49, 0x46, 0x38, 0x39, 0x61]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Gif);
    }

    #[test]
    fn detect_heic_by_magic_wrong_ext() {
        let f = write_temp_file(
            "dat",
            &[
                0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, b'h', b'e', b'i', b'c',
            ],
        );
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Heic);
    }

    #[test]
    fn detect_heic_mif1_brand() {
        let f = write_temp_file(
            "dat",
            &[
                0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, b'm', b'i', b'f', b'1',
            ],
        );
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Heic);
    }

    // --- Disagreement: magic bytes win over extension ---

    #[test]
    fn magic_bytes_override_misleading_jpg_extension() {
        // Regression: WebP payload with a .jpg extension (seen in the wild from
        // image scrapers). Before magic-bytes-first, this routed to the JPEG
        // decoder and failed with "Illegal start bytes:5249".
        let f = write_temp_file("jpg", b"RIFF\x00\x00\x00\x00WEBP");
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::WebP);
    }

    #[test]
    fn magic_bytes_override_misleading_png_extension() {
        let f = write_temp_file("png", &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn extension_used_when_magic_unrecognized() {
        // Corrupt/short header — fall back to extension so downstream decode
        // still runs and produces a format-specific error.
        let f = write_temp_file("jpg", b"garbage");
        assert_eq!(detect_format(f.path()).unwrap(), ImageFormat::Jpeg);
    }

    // --- Error cases ---

    #[test]
    fn unsupported_format_unknown_ext_and_bytes() {
        let f = write_temp_file("dat", b"not an image at all");
        let err = detect_format(f.path()).unwrap_err();
        assert!(matches!(err, ImgstripError::UnsupportedFormat(_)));
    }

    #[test]
    fn empty_file_returns_error() {
        let f = write_temp_file("dat", b"");
        let err = detect_format(f.path()).unwrap_err();
        assert!(matches!(err, ImgstripError::UnsupportedFormat(_)));
    }

    #[test]
    fn nonexistent_file_returns_io_error() {
        let err = detect_format(Path::new("/tmp/does_not_exist_imgstrip_test.dat")).unwrap_err();
        assert!(matches!(err, ImgstripError::IoError { .. }));
    }

    // --- parse_output_format ---

    #[test]
    fn parse_output_jpeg() {
        assert_eq!(parse_output_format("jpeg").unwrap(), OutputFormat::Jpeg);
        assert_eq!(parse_output_format("jpg").unwrap(), OutputFormat::Jpeg);
        assert_eq!(parse_output_format("JPEG").unwrap(), OutputFormat::Jpeg);
    }

    #[test]
    fn parse_output_png() {
        assert_eq!(parse_output_format("png").unwrap(), OutputFormat::Png);
        assert_eq!(parse_output_format("PNG").unwrap(), OutputFormat::Png);
    }

    #[test]
    fn parse_output_webp() {
        assert_eq!(parse_output_format("webp").unwrap(), OutputFormat::WebP);
    }

    #[test]
    fn parse_output_bmp() {
        assert_eq!(parse_output_format("bmp").unwrap(), OutputFormat::Bmp);
    }

    #[test]
    fn parse_output_tiff() {
        assert_eq!(parse_output_format("tiff").unwrap(), OutputFormat::Tiff);
        assert_eq!(parse_output_format("tif").unwrap(), OutputFormat::Tiff);
    }

    #[test]
    fn parse_output_gif() {
        assert_eq!(parse_output_format("gif").unwrap(), OutputFormat::Gif);
    }

    #[test]
    fn parse_output_invalid() {
        let err = parse_output_format("heic").unwrap_err();
        assert!(matches!(err, ImgstripError::UnsupportedFormat(_)));

        let err = parse_output_format("pdf").unwrap_err();
        assert!(matches!(err, ImgstripError::UnsupportedFormat(_)));
    }

    // --- default_extension ---

    #[test]
    fn default_extensions() {
        assert_eq!(default_extension(OutputFormat::Jpeg), "jpg");
        assert_eq!(default_extension(OutputFormat::Png), "png");
        assert_eq!(default_extension(OutputFormat::WebP), "webp");
        assert_eq!(default_extension(OutputFormat::Bmp), "bmp");
        assert_eq!(default_extension(OutputFormat::Tiff), "tiff");
        assert_eq!(default_extension(OutputFormat::Gif), "gif");
    }
}
