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
        return Ok(());
    }

    // Use the magic-byte-detected format so a mislabeled file (e.g., a WebP
    // named *.jpg) still reports correct dimensions / color type instead of
    // the image crate failing on an extension-guessed format mismatch.
    let image_fmt = format
        .to_image_format()
        .expect("non-HEIC formats map to image::ImageFormat");

    let open_reader = || -> std::io::Result<_> {
        let file = std::fs::File::open(path)?;
        Ok(image::ImageReader::with_format(
            std::io::BufReader::new(file),
            image_fmt,
        ))
    };

    match open_reader().and_then(|r| r.into_dimensions().map_err(std::io::Error::other)) {
        Ok((w, h)) => println!("Dimensions: {w}x{h}"),
        Err(e) => println!("Dimensions: unknown ({e})"),
    }
    // Color type requires a full decode.
    match open_reader().and_then(|r| r.decode().map_err(std::io::Error::other)) {
        Ok(img) => println!("Color type: {:?}", img.color()),
        Err(_) => println!("Color type: unknown"),
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

    let mut camera: Vec<(&str, String)> = Vec::new();
    let mut exposure: Vec<(&str, String)> = Vec::new();
    let mut datetime: Vec<(&str, String)> = Vec::new();
    let mut image: Vec<(&str, String)> = Vec::new();
    let mut author: Vec<(&str, String)> = Vec::new();
    let mut gps: Vec<(&str, String)> = Vec::new();

    for tag in &metadata {
        match tag {
            // --- Camera ---
            ExifTag::Make(v) => push_str(&mut camera, "Make", v),
            ExifTag::Model(v) => push_str(&mut camera, "Model", v),
            ExifTag::LensMake(v) => push_str(&mut camera, "Lens Make", v),
            ExifTag::LensModel(v) => push_str(&mut camera, "Lens Model", v),
            ExifTag::LensSerialNumber(v) => push_str(&mut camera, "Lens Serial Number", v),
            ExifTag::SerialNumber(v) => push_str(&mut camera, "Serial Number", v),
            ExifTag::OwnerName(v) => push_str(&mut camera, "Owner", v),
            ExifTag::LensInfo(rationals) if !rationals.is_empty() => {
                camera.push(("Lens Info", format_lens_info(rationals)));
            }

            // --- Exposure ---
            ExifTag::ExposureTime(rationals) if !rationals.is_empty() => {
                exposure.push(("Exposure Time", format_exposure_time(&rationals[0])));
            }
            ExifTag::FNumber(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                exposure.push(("F-Number", format!("f/{val:.1}")));
            }
            ExifTag::ISO(vals) if !vals.is_empty() => {
                exposure.push(("ISO", vals[0].to_string()));
            }
            ExifTag::FocalLength(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                exposure.push(("Focal Length", format!("{val:.1} mm")));
            }
            ExifTag::FocalLengthIn35mmFormat(vals) if !vals.is_empty() => {
                exposure.push(("Focal Length (35mm)", format!("{} mm", vals[0])));
            }
            ExifTag::ExposureProgram(vals) if !vals.is_empty() => {
                exposure.push(("Exposure Program", format_exposure_program(vals[0])));
            }
            ExifTag::ExposureMode(vals) if !vals.is_empty() => {
                exposure.push(("Exposure Mode", format_exposure_mode(vals[0])));
            }
            ExifTag::ExposureCompensation(rationals) if !rationals.is_empty() => {
                let val = rationals[0].nominator as f64 / rationals[0].denominator.max(1) as f64;
                exposure.push(("Exposure Compensation", format!("{val:+.1} EV")));
            }
            ExifTag::MeteringMode(vals) if !vals.is_empty() => {
                exposure.push(("Metering Mode", format_metering_mode(vals[0])));
            }
            ExifTag::Flash(vals) if !vals.is_empty() => {
                exposure.push(("Flash", format_flash(vals[0])));
            }
            ExifTag::WhiteBalance(vals) if !vals.is_empty() => {
                exposure.push((
                    "White Balance",
                    if vals[0] == 0 {
                        "Auto".into()
                    } else {
                        "Manual".into()
                    },
                ));
            }
            ExifTag::LightSource(vals) if !vals.is_empty() => {
                exposure.push(("Light Source", format_light_source(vals[0])));
            }
            ExifTag::ShutterSpeedValue(rationals) if !rationals.is_empty() => {
                let apex =
                    rationals[0].nominator as f64 / rationals[0].denominator.max(1) as f64;
                let seconds = 2.0_f64.powf(-apex);
                if seconds < 1.0 && seconds > 0.0 {
                    exposure.push((
                        "Shutter Speed",
                        format!("1/{:.0} s", 1.0 / seconds),
                    ));
                } else {
                    exposure.push(("Shutter Speed", format!("{seconds:.1} s")));
                }
            }
            ExifTag::ApertureValue(rationals) if !rationals.is_empty() => {
                let apex = rational_to_f64(&rationals[0]);
                let fnum = 2.0_f64.powf(apex / 2.0);
                exposure.push(("Aperture Value", format!("f/{fnum:.1}")));
            }
            ExifTag::BrightnessValue(rationals) if !rationals.is_empty() => {
                let val =
                    rationals[0].nominator as f64 / rationals[0].denominator.max(1) as f64;
                exposure.push(("Brightness", format!("{val:.2} EV")));
            }
            ExifTag::MaxApertureValue(rationals) if !rationals.is_empty() => {
                let apex = rational_to_f64(&rationals[0]);
                let fnum = 2.0_f64.powf(apex / 2.0);
                exposure.push(("Max Aperture", format!("f/{fnum:.1}")));
            }
            ExifTag::SubjectDistance(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                exposure.push(("Subject Distance", format!("{val:.2} m")));
            }
            ExifTag::DigitalZoomRatio(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                exposure.push(("Digital Zoom", format!("{val:.1}x")));
            }
            ExifTag::SceneCaptureType(vals) if !vals.is_empty() => {
                exposure.push(("Scene Capture Type", format_scene_capture(vals[0])));
            }
            ExifTag::GainControl(vals) if !vals.is_empty() => {
                exposure.push(("Gain Control", format_gain_control(vals[0])));
            }
            ExifTag::Contrast(vals) if !vals.is_empty() => {
                exposure.push(("Contrast", format_low_normal_high(vals[0])));
            }
            ExifTag::Saturation(vals) if !vals.is_empty() => {
                exposure.push(("Saturation", format_low_normal_high(vals[0])));
            }
            ExifTag::Sharpness(vals) if !vals.is_empty() => {
                exposure.push(("Sharpness", format_low_normal_high(vals[0])));
            }
            ExifTag::SensingMethod(vals) if !vals.is_empty() => {
                exposure.push(("Sensing Method", format_sensing_method(vals[0])));
            }

            // --- Date/Time ---
            ExifTag::DateTimeOriginal(v) => push_str(&mut datetime, "Date Taken", v),
            ExifTag::CreateDate(v) => push_str(&mut datetime, "Date Created", v),
            ExifTag::ModifyDate(v) => push_str(&mut datetime, "Date Modified", v),
            ExifTag::OffsetTime(v) => push_str(&mut datetime, "Timezone", v),
            ExifTag::OffsetTimeOriginal(v) => push_str(&mut datetime, "Timezone (Original)", v),
            ExifTag::OffsetTimeDigitized(v) => {
                push_str(&mut datetime, "Timezone (Digitized)", v)
            }
            ExifTag::SubSecTime(v) => push_str(&mut datetime, "Sub-second Time", v),
            ExifTag::SubSecTimeOriginal(v) => {
                push_str(&mut datetime, "Sub-second (Original)", v)
            }
            ExifTag::SubSecTimeDigitized(v) => {
                push_str(&mut datetime, "Sub-second (Digitized)", v)
            }

            // --- Image ---
            ExifTag::ImageWidth(vals) if !vals.is_empty() => {
                image.push(("Width", vals[0].to_string()));
            }
            ExifTag::ImageHeight(vals) if !vals.is_empty() => {
                image.push(("Height", vals[0].to_string()));
            }
            ExifTag::ExifImageWidth(vals) if !vals.is_empty() => {
                image.push(("EXIF Width", vals[0].to_string()));
            }
            ExifTag::ExifImageHeight(vals) if !vals.is_empty() => {
                image.push(("EXIF Height", vals[0].to_string()));
            }
            ExifTag::Orientation(vals) if !vals.is_empty() => {
                image.push(("Orientation", format_orientation(vals[0])));
            }
            ExifTag::ColorSpace(vals) if !vals.is_empty() => {
                image.push(("Color Space", format_color_space(vals[0])));
            }
            ExifTag::XResolution(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                image.push(("X Resolution", format!("{val:.0}")));
            }
            ExifTag::YResolution(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                image.push(("Y Resolution", format!("{val:.0}")));
            }
            ExifTag::ResolutionUnit(vals) if !vals.is_empty() => {
                image.push(("Resolution Unit", format_resolution_unit(vals[0])));
            }
            ExifTag::BitsPerSample(vals) if !vals.is_empty() => {
                let parts: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
                image.push(("Bits Per Sample", parts.join(", ")));
            }
            ExifTag::Compression(vals) if !vals.is_empty() => {
                image.push(("Compression", format_compression(vals[0])));
            }
            ExifTag::CustomRendered(vals) if !vals.is_empty() => {
                image.push((
                    "Custom Rendered",
                    if vals[0] == 0 {
                        "Normal".into()
                    } else {
                        "Custom".into()
                    },
                ));
            }
            ExifTag::CompositeImage(vals) if !vals.is_empty() => {
                image.push((
                    "Composite Image",
                    match vals[0] {
                        0 => "Unknown".into(),
                        1 => "Not composite".into(),
                        2 => "General composite".into(),
                        3 => "Composite captured simultaneously".into(),
                        _ => format!("Unknown ({})", vals[0]),
                    },
                ));
            }

            // --- Author ---
            ExifTag::Artist(v) => push_str(&mut author, "Artist", v),
            ExifTag::Copyright(v) => push_str(&mut author, "Copyright", v),
            ExifTag::Software(v) => push_str(&mut author, "Software", v),
            ExifTag::ImageDescription(v) => push_str(&mut author, "Description", v),
            ExifTag::ImageUniqueID(v) => push_str(&mut author, "Unique ID", v),
            ExifTag::UserComment(bytes) if !bytes.is_empty() => {
                if let Some(comment) = decode_user_comment(bytes) {
                    author.push(("User Comment", comment));
                }
            }

            // --- GPS ---
            ExifTag::GPSLatitudeRef(_)
            | ExifTag::GPSLatitude(_)
            | ExifTag::GPSLongitudeRef(_)
            | ExifTag::GPSLongitude(_) => {
                // Handled below as a combined field
            }
            ExifTag::GPSAltitudeRef(vals) if !vals.is_empty() => {
                gps.push((
                    "Altitude Ref",
                    if vals[0] == 0 {
                        "Above sea level".into()
                    } else {
                        "Below sea level".into()
                    },
                ));
            }
            ExifTag::GPSAltitude(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                gps.push(("Altitude", format!("{val:.1} m")));
            }
            ExifTag::GPSTimeStamp(rationals) if rationals.len() == 3 => {
                let h = rationals[0].nominator / rationals[0].denominator.max(1);
                let m = rationals[1].nominator / rationals[1].denominator.max(1);
                let s = rational_to_f64(&rationals[2]);
                gps.push(("Time (UTC)", format!("{h:02}:{m:02}:{s:05.2}")));
            }
            ExifTag::GPSDateStamp(v) => push_str(&mut gps, "Date", v),
            ExifTag::GPSSpeed(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                gps.push(("Speed", format!("{val:.1}")));
            }
            ExifTag::GPSSpeedRef(v) => push_str(&mut gps, "Speed Ref", v),
            ExifTag::GPSTrack(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                gps.push(("Track", format!("{val:.1}")));
            }
            ExifTag::GPSImgDirection(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                gps.push(("Image Direction", format!("{val:.1}")));
            }
            ExifTag::GPSImgDirectionRef(v) => push_str(&mut gps, "Image Direction Ref", v),
            ExifTag::GPSDOP(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                gps.push(("DOP", format!("{val:.1}")));
            }
            ExifTag::GPSMapDatum(v) => push_str(&mut gps, "Map Datum", v),
            ExifTag::GPSSatellites(v) => push_str(&mut gps, "Satellites", v),
            ExifTag::GPSMeasureMode(v) => push_str(&mut gps, "Measure Mode", v),
            ExifTag::GPSStatus(v) => push_str(&mut gps, "Status", v),
            ExifTag::GPSDifferential(vals) if !vals.is_empty() => {
                gps.push((
                    "Differential",
                    if vals[0] == 0 {
                        "No correction".into()
                    } else {
                        "Differential corrected".into()
                    },
                ));
            }
            ExifTag::GPSHPositioningError(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                gps.push(("H Positioning Error", format!("{val:.2} m")));
            }

            // --- Environmental ---
            ExifTag::AmbientTemperature(rationals) if !rationals.is_empty() => {
                let val =
                    rationals[0].nominator as f64 / rationals[0].denominator.max(1) as f64;
                exposure.push(("Ambient Temperature", format!("{val:.1} C")));
            }
            ExifTag::Humidity(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                exposure.push(("Humidity", format!("{val:.1}%")));
            }
            ExifTag::Pressure(rationals) if !rationals.is_empty() => {
                let val = rational_to_f64(&rationals[0]);
                exposure.push(("Pressure", format!("{val:.1} hPa")));
            }

            _ => {}
        }
    }

    // Insert combined GPS coordinates at the top of the GPS section
    if let Some(coords) = format_gps_coordinates(&metadata) {
        gps.insert(0, ("Coordinates", coords));
    }

    let sections: &[(&str, &Vec<(&str, String)>)] = &[
        ("Camera", &camera),
        ("Exposure", &exposure),
        ("Date/Time", &datetime),
        ("Image", &image),
        ("Author", &author),
        ("GPS", &gps),
    ];

    for (title, fields) in sections {
        if fields.is_empty() {
            continue;
        }
        println!();
        println!("{title}:");
        let label_width = fields.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
        for (name, value) in *fields {
            println!("  {name:label_width$}  {value}");
        }
    }
}

fn push_str(fields: &mut Vec<(&str, String)>, label: &'static str, value: &str) {
    let trimmed = value.trim_end_matches('\0').trim();
    if !trimmed.is_empty() {
        fields.push((label, trimmed.to_string()));
    }
}

fn rational_to_f64(r: &little_exif::rational::uR64) -> f64 {
    r.nominator as f64 / r.denominator.max(1) as f64
}

fn format_exposure_time(r: &little_exif::rational::uR64) -> String {
    if r.denominator == 0 {
        return "Unknown".into();
    }
    let val = r.nominator as f64 / r.denominator as f64;
    if val >= 1.0 {
        format!("{val:.1} s")
    } else if r.nominator == 1 {
        format!("1/{} s", r.denominator)
    } else {
        // Normalize to 1/x
        let denom = r.denominator as f64 / r.nominator as f64;
        format!("1/{denom:.0} s")
    }
}

fn format_lens_info(rationals: &[little_exif::rational::uR64]) -> String {
    if rationals.len() < 4 {
        return String::new();
    }
    let min_fl = rational_to_f64(&rationals[0]);
    let max_fl = rational_to_f64(&rationals[1]);
    let min_fn = rational_to_f64(&rationals[2]);
    let max_fn = rational_to_f64(&rationals[3]);

    if (min_fl - max_fl).abs() < 0.1 {
        format!("{min_fl:.0}mm f/{min_fn:.1}")
    } else {
        format!("{min_fl:.0}-{max_fl:.0}mm f/{min_fn:.1}-{max_fn:.1}")
    }
}

fn format_orientation(v: u16) -> String {
    match v {
        1 => "Horizontal (normal)".into(),
        2 => "Mirror horizontal".into(),
        3 => "Rotate 180".into(),
        4 => "Mirror vertical".into(),
        5 => "Mirror horizontal, rotate 270 CW".into(),
        6 => "Rotate 90 CW".into(),
        7 => "Mirror horizontal, rotate 90 CW".into(),
        8 => "Rotate 270 CW".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_exposure_program(v: u16) -> String {
    match v {
        0 => "Not defined".into(),
        1 => "Manual".into(),
        2 => "Program AE".into(),
        3 => "Aperture priority".into(),
        4 => "Shutter priority".into(),
        5 => "Creative (slow speed)".into(),
        6 => "Action (high speed)".into(),
        7 => "Portrait".into(),
        8 => "Landscape".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_exposure_mode(v: u16) -> String {
    match v {
        0 => "Auto".into(),
        1 => "Manual".into(),
        2 => "Auto bracket".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_metering_mode(v: u16) -> String {
    match v {
        0 => "Unknown".into(),
        1 => "Average".into(),
        2 => "Center-weighted average".into(),
        3 => "Spot".into(),
        4 => "Multi-spot".into(),
        5 => "Multi-segment".into(),
        6 => "Partial".into(),
        255 => "Other".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_flash(v: u16) -> String {
    let fired = v & 0x01 != 0;
    let ret = (v >> 1) & 0x03;
    let mode = (v >> 3) & 0x03;
    let function = (v >> 5) & 0x01 != 0;
    let red_eye = (v >> 6) & 0x01 != 0;

    let mut parts = Vec::new();

    if fired {
        parts.push("Fired");
    } else {
        parts.push("Did not fire");
    }

    match ret {
        2 => parts.push("strobe return not detected"),
        3 => parts.push("strobe return detected"),
        _ => {}
    }

    match mode {
        1 => parts.push("compulsory firing"),
        2 => parts.push("compulsory suppression"),
        3 => parts.push("auto mode"),
        _ => {}
    }

    if function {
        parts.push("no flash function");
    }
    if red_eye {
        parts.push("red-eye reduction");
    }

    let mut result = parts[0].to_string();
    if parts.len() > 1 {
        result.push_str(" (");
        result.push_str(&parts[1..].join(", "));
        result.push(')');
    }
    result
}

fn format_light_source(v: u16) -> String {
    match v {
        0 => "Unknown".into(),
        1 => "Daylight".into(),
        2 => "Fluorescent".into(),
        3 => "Tungsten (incandescent)".into(),
        4 => "Flash".into(),
        9 => "Fine weather".into(),
        10 => "Cloudy".into(),
        11 => "Shade".into(),
        12 => "Daylight fluorescent (D 5700-7100K)".into(),
        13 => "Day white fluorescent (N 4600-5500K)".into(),
        14 => "Cool white fluorescent (W 3800-4500K)".into(),
        15 => "White fluorescent (WW 3250-3800K)".into(),
        16 => "Warm white fluorescent (L 2600-3250K)".into(),
        17 => "Standard light A".into(),
        18 => "Standard light B".into(),
        19 => "Standard light C".into(),
        20 => "D55".into(),
        21 => "D65".into(),
        22 => "D75".into(),
        23 => "D50".into(),
        24 => "ISO studio tungsten".into(),
        255 => "Other".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_color_space(v: u16) -> String {
    match v {
        1 => "sRGB".into(),
        0xFFFF => "Uncalibrated".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_resolution_unit(v: u16) -> String {
    match v {
        1 => "No unit".into(),
        2 => "inches".into(),
        3 => "centimeters".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_compression(v: u16) -> String {
    match v {
        1 => "Uncompressed".into(),
        6 => "JPEG".into(),
        _ => format!("{v}"),
    }
}

fn format_scene_capture(v: u16) -> String {
    match v {
        0 => "Standard".into(),
        1 => "Landscape".into(),
        2 => "Portrait".into(),
        3 => "Night".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_gain_control(v: u16) -> String {
    match v {
        0 => "None".into(),
        1 => "Low gain up".into(),
        2 => "High gain up".into(),
        3 => "Low gain down".into(),
        4 => "High gain down".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_low_normal_high(v: u16) -> String {
    match v {
        0 => "Normal".into(),
        1 => "Low".into(),
        2 => "High".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn format_sensing_method(v: u16) -> String {
    match v {
        1 => "Not defined".into(),
        2 => "One-chip color area".into(),
        3 => "Two-chip color area".into(),
        4 => "Three-chip color area".into(),
        5 => "Color sequential area".into(),
        7 => "Trilinear".into(),
        8 => "Color sequential linear".into(),
        _ => format!("Unknown ({v})"),
    }
}

fn decode_user_comment(bytes: &[u8]) -> Option<String> {
    // First 8 bytes are the character code identifier
    if bytes.len() <= 8 {
        return None;
    }
    let payload = &bytes[8..];
    let text = String::from_utf8_lossy(payload)
        .trim_end_matches('\0')
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn format_gps_coordinates(metadata: &little_exif::metadata::Metadata) -> Option<String> {
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

    #[test]
    fn info_jpeg_with_rich_exif() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::metadata::Metadata;
        use little_exif::rational::uR64;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rich.jpg");
        create_bare_jpeg(&path);

        let mut metadata = Metadata::new();
        metadata.set_tag(ExifTag::Make("Canon".to_string()));
        metadata.set_tag(ExifTag::Model("EOS R5".to_string()));
        metadata.set_tag(ExifTag::Software("Adobe Lightroom 6.0".to_string()));
        metadata.set_tag(ExifTag::Artist("John Doe".to_string()));
        metadata.set_tag(ExifTag::Copyright("2024 John Doe".to_string()));
        metadata.set_tag(ExifTag::DateTimeOriginal("2024:06:15 14:32:01".to_string()));
        metadata.set_tag(ExifTag::CreateDate("2024:06:15 14:32:01".to_string()));
        metadata.set_tag(ExifTag::ISO(vec![100]));
        metadata.set_tag(ExifTag::ExposureTime(vec![uR64 {
            nominator: 1,
            denominator: 250,
        }]));
        metadata.set_tag(ExifTag::FNumber(vec![uR64 {
            nominator: 28,
            denominator: 10,
        }]));
        metadata.set_tag(ExifTag::FocalLength(vec![uR64 {
            nominator: 50,
            denominator: 1,
        }]));
        metadata.set_tag(ExifTag::ExposureProgram(vec![2])); // Program AE
        metadata.set_tag(ExifTag::MeteringMode(vec![5])); // Multi-segment
        metadata.set_tag(ExifTag::Flash(vec![0x10])); // Did not fire, compulsory suppression
        metadata.set_tag(ExifTag::Orientation(vec![1])); // Horizontal
        metadata.set_tag(ExifTag::ColorSpace(vec![1])); // sRGB
        metadata.set_tag(ExifTag::LensModel("RF 50mm F1.2L USM".to_string()));
        metadata.write_to_file(&path).unwrap();

        // Should succeed and print grouped output
        display_info(&path).unwrap();
    }
}
