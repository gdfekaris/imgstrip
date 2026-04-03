use std::path::Path;

use crate::error::ImgstripError;
use crate::formats::{self, ImageFormat};

/// Display metadata summary for an image file.
pub fn display_info(path: &Path) -> Result<(), ImgstripError> {
    if !path.exists() {
        return Err(ImgstripError::IoError {
            path: path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        });
    }

    let format = formats::detect_format(path)?;
    let file_size = std::fs::metadata(path)
        .map_err(|e| ImgstripError::IoError {
            path: path.to_path_buf(),
            source: e,
        })?
        .len();

    println!("File:   {}", path.display());
    println!("Format: {format}");
    println!("Size:   {}", format_size(file_size));

    // Dimensions and color type
    print_image_dimensions(path, format)?;

    // Metadata presence summary
    let presence = detect_metadata_presence(path, format);
    println!();
    println!("Metadata:");
    println!(
        "  EXIF: {}",
        if presence.exif { "present" } else { "absent" }
    );
    println!(
        "  XMP:  {}",
        if presence.xmp { "present" } else { "absent" }
    );
    println!(
        "  ICC:  {}",
        if presence.icc { "present" } else { "absent" }
    );
    println!(
        "  IPTC: {}",
        if presence.iptc { "present" } else { "absent" }
    );

    // Key EXIF fields
    if presence.exif {
        print_exif_fields(path);
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn print_image_dimensions(path: &Path, format: ImageFormat) -> Result<(), ImgstripError> {
    if format == ImageFormat::Heic {
        let img = crate::heic::decode_heic(path)?;
        println!("Dimensions: {}x{}", img.width(), img.height());
        println!("Color type: {:?}", img.color());
    } else {
        match image::image_dimensions(path) {
            Ok((w, h)) => println!("Dimensions: {w}x{h}"),
            Err(e) => println!("Dimensions: unknown ({e})"),
        }
        // Color type requires a full decode, use the image reader for it
        match image::open(path) {
            Ok(img) => println!("Color type: {:?}", img.color()),
            Err(_) => println!("Color type: unknown"),
        }
    }
    Ok(())
}

struct MetadataPresence {
    exif: bool,
    xmp: bool,
    icc: bool,
    iptc: bool,
}

fn detect_metadata_presence(path: &Path, format: ImageFormat) -> MetadataPresence {
    match format {
        ImageFormat::Jpeg => detect_jpeg_metadata(path),
        ImageFormat::Png => detect_png_metadata(path),
        ImageFormat::WebP => detect_webp_metadata(path),
        _ => {
            // For TIFF/HEIC/BMP/GIF, use the extract bundle as a rough check
            let bundle = crate::metadata::extract(path).ok();
            MetadataPresence {
                exif: bundle.as_ref().is_some_and(|b| b.exif.is_some()),
                xmp: bundle.as_ref().is_some_and(|b| b.xmp.is_some()),
                icc: bundle.as_ref().is_some_and(|b| b.icc.is_some()),
                iptc: false,
            }
        }
    }
}

fn detect_jpeg_metadata(path: &Path) -> MetadataPresence {
    use img_parts::jpeg::{Jpeg, markers};
    use img_parts::{ImageEXIF, ImageICC};

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => {
            return MetadataPresence {
                exif: false,
                xmp: false,
                icc: false,
                iptc: false,
            };
        }
    };
    let jpeg = match Jpeg::from_bytes(data.into()) {
        Ok(j) => j,
        Err(_) => {
            return MetadataPresence {
                exif: false,
                xmp: false,
                icc: false,
                iptc: false,
            };
        }
    };

    let exif = jpeg.exif().is_some();
    let xmp = jpeg.segments().iter().any(|seg| {
        seg.marker() == markers::APP1
            && seg
                .contents()
                .starts_with(b"http://ns.adobe.com/xap/1.0/\0")
    });
    let icc = jpeg.icc_profile().is_some();
    let iptc = jpeg
        .segments()
        .iter()
        .any(|seg| seg.marker() == markers::APP13);

    MetadataPresence {
        exif,
        xmp,
        icc,
        iptc,
    }
}

fn detect_png_metadata(path: &Path) -> MetadataPresence {
    use img_parts::png::Png;
    use img_parts::{ImageEXIF, ImageICC};

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => {
            return MetadataPresence {
                exif: false,
                xmp: false,
                icc: false,
                iptc: false,
            };
        }
    };
    let png = match Png::from_bytes(data.into()) {
        Ok(p) => p,
        Err(_) => {
            return MetadataPresence {
                exif: false,
                xmp: false,
                icc: false,
                iptc: false,
            };
        }
    };

    MetadataPresence {
        exif: png.exif().is_some(),
        xmp: false,
        icc: png.icc_profile().is_some(),
        iptc: false,
    }
}

fn detect_webp_metadata(path: &Path) -> MetadataPresence {
    use img_parts::webp::{CHUNK_XMP, WebP};
    use img_parts::{ImageEXIF, ImageICC};

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => {
            return MetadataPresence {
                exif: false,
                xmp: false,
                icc: false,
                iptc: false,
            };
        }
    };
    let webp = match WebP::from_bytes(data.into()) {
        Ok(w) => w,
        Err(_) => {
            return MetadataPresence {
                exif: false,
                xmp: false,
                icc: false,
                iptc: false,
            };
        }
    };

    MetadataPresence {
        exif: webp.exif().is_some(),
        xmp: webp.chunk_by_id(CHUNK_XMP).is_some(),
        icc: webp.icc_profile().is_some(),
        iptc: false,
    }
}

fn print_exif_fields(path: &Path) {
    use little_exif::exif_tag::ExifTag;
    use little_exif::metadata::Metadata;

    let metadata = match Metadata::new_from_path(path) {
        Ok(m) => m,
        Err(_) => return,
    };

    let mut fields: Vec<(&str, String)> = Vec::new();

    if let Some(ExifTag::Make(v)) = metadata.get_tag(&ExifTag::Make(String::new())).next() {
        fields.push(("Make", v.trim_end_matches('\0').to_string()));
    }
    if let Some(ExifTag::Model(v)) = metadata.get_tag(&ExifTag::Model(String::new())).next() {
        fields.push(("Model", v.trim_end_matches('\0').to_string()));
    }
    if let Some(ExifTag::DateTimeOriginal(v)) = metadata
        .get_tag(&ExifTag::DateTimeOriginal(String::new()))
        .next()
    {
        fields.push(("DateTimeOriginal", v.trim_end_matches('\0').to_string()));
    }
    if let Some(ExifTag::Software(v)) = metadata.get_tag(&ExifTag::Software(String::new())).next() {
        fields.push(("Software", v.trim_end_matches('\0').to_string()));
    }
    if let Some(ExifTag::ImageDescription(v)) = metadata
        .get_tag(&ExifTag::ImageDescription(String::new()))
        .next()
    {
        fields.push(("ImageDescription", v.trim_end_matches('\0').to_string()));
    }

    // GPS
    let gps = format_gps(&metadata);
    if let Some(coords) = gps {
        fields.push(("GPS", coords));
    }

    if !fields.is_empty() {
        println!();
        println!("EXIF fields:");
        for (name, value) in &fields {
            println!("  {name}: {value}");
        }
    }
}

fn format_gps(metadata: &little_exif::metadata::Metadata) -> Option<String> {
    use little_exif::exif_tag::ExifTag;

    let lat_ref = match metadata
        .get_tag(&ExifTag::GPSLatitudeRef(String::new()))
        .next()
    {
        Some(ExifTag::GPSLatitudeRef(v)) => v.trim_end_matches('\0').to_string(),
        _ => return None,
    };
    let lon_ref = match metadata
        .get_tag(&ExifTag::GPSLongitudeRef(String::new()))
        .next()
    {
        Some(ExifTag::GPSLongitudeRef(v)) => v.trim_end_matches('\0').to_string(),
        _ => return None,
    };
    let lat = match metadata.get_tag(&ExifTag::GPSLatitude(vec![])).next() {
        Some(ExifTag::GPSLatitude(rationals)) if rationals.len() == 3 => {
            let d = rationals[0].nominator as f64 / rationals[0].denominator.max(1) as f64;
            let m = rationals[1].nominator as f64 / rationals[1].denominator.max(1) as f64;
            let s = rationals[2].nominator as f64 / rationals[2].denominator.max(1) as f64;
            d + m / 60.0 + s / 3600.0
        }
        _ => return None,
    };
    let lon = match metadata.get_tag(&ExifTag::GPSLongitude(vec![])).next() {
        Some(ExifTag::GPSLongitude(rationals)) if rationals.len() == 3 => {
            let d = rationals[0].nominator as f64 / rationals[0].denominator.max(1) as f64;
            let m = rationals[1].nominator as f64 / rationals[1].denominator.max(1) as f64;
            let s = rationals[2].nominator as f64 / rationals[2].denominator.max(1) as f64;
            d + m / 60.0 + s / 3600.0
        }
        _ => return None,
    };

    let lat_sign = if lat_ref == "S" { -1.0 } else { 1.0 };
    let lon_sign = if lon_ref == "W" { -1.0 } else { 1.0 };

    Some(format!("{:.6}, {:.6}", lat * lat_sign, lon * lon_sign))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_bare_jpeg(path: &Path) {
        use image::codecs::jpeg::JpegEncoder;
        use image::{DynamicImage, ImageEncoder, RgbImage};
        use std::io::BufWriter;
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        let file = fs::File::create(path).unwrap();
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

    fn create_jpeg_with_exif(path: &Path) {
        use img_parts::jpeg::Jpeg;
        use img_parts::{Bytes, ImageEXIF};

        create_bare_jpeg(path);

        let data = fs::read(path).unwrap();
        let mut jpeg = Jpeg::from_bytes(data.into()).unwrap();

        // Valid TIFF-header EXIF with a Make tag
        let exif_payload: Vec<u8> = {
            let mut v = Vec::new();
            v.extend_from_slice(b"MM");
            v.extend_from_slice(&[0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]);
            v.extend_from_slice(&[0x00, 0x01]); // 1 entry
            // Tag 0x010F (Make), type ASCII (2), count 5, value "Test\0"
            v.extend_from_slice(&[0x01, 0x0F, 0x00, 0x02, 0x00, 0x00, 0x00, 0x05]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x1A]); // offset
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // next IFD
            v.extend_from_slice(b"Test\0");
            v
        };
        jpeg.set_exif(Some(Bytes::from(exif_payload)));

        let file = fs::File::create(path).unwrap();
        jpeg.encoder().write_to(file).unwrap();
    }

    #[test]
    fn info_jpeg_with_exif() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.jpg");
        create_jpeg_with_exif(&path);
        // Should succeed without error
        display_info(&path).unwrap();
    }

    #[test]
    fn info_stripped_jpeg() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bare.jpg");
        create_bare_jpeg(&path);
        display_info(&path).unwrap();
    }

    #[test]
    fn info_png() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(16, 16, |x, y| {
            image::Rgb([(x * 15) as u8, (y * 15) as u8, 128])
        }));
        img.save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        display_info(&path).unwrap();
    }

    #[test]
    fn info_webp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.webp");
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(8, 8, |x, y| {
            image::Rgba([(x * 30) as u8, (y * 30) as u8, 128, 255])
        }));
        // WebP via image crate requires lossless encoder
        use image::ImageEncoder;
        use image::codecs::webp::WebPEncoder;
        let file = fs::File::create(&path).unwrap();
        let encoder = WebPEncoder::new_lossless(std::io::BufWriter::new(file));
        encoder
            .write_image(
                img.to_rgba8().as_raw(),
                img.width(),
                img.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        display_info(&path).unwrap();
    }

    #[test]
    fn info_heic() {
        display_info(Path::new("tests/fixtures/sample.heic")).unwrap();
    }

    #[test]
    fn info_nonexistent_file() {
        let err = display_info(Path::new("/tmp/no_such_file_imgstrip.jpg")).unwrap_err();
        assert!(matches!(err, ImgstripError::IoError { .. }));
    }

    #[test]
    fn info_non_image_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("readme.txt");
        fs::write(&path, "not an image").unwrap();
        let err = display_info(&path).unwrap_err();
        assert!(matches!(err, ImgstripError::UnsupportedFormat(_)));
    }
}
