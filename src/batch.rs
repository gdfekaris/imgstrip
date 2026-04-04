use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::convert;
use crate::error::ImgstripError;
use crate::formats::{self, OutputFormat};
use crate::metadata;
use crate::rename;

/// Result of a batch operation across multiple files.
pub struct BatchReport {
    pub succeeded: usize,
    pub failed: Vec<(PathBuf, ImgstripError)>,
}

/// What operation to perform on each file.
pub enum Operation {
    Convert {
        format: OutputFormat,
        quality: u8,
        overwrite: bool,
        strip_metadata: bool,
    },
    Strip,
}

/// Options controlling directory traversal and output.
pub struct BatchOptions {
    pub recursive: bool,
    pub dry_run: bool,
    pub output_dir: Option<PathBuf>,
    pub verbose: bool,
    pub rename_prefix: Option<String>,
}

/// Process all supported image files in a directory.
pub fn process_directory(
    input_dir: &Path,
    operation: &Operation,
    options: &BatchOptions,
) -> Result<BatchReport, ImgstripError> {
    let mut report = BatchReport {
        succeeded: 0,
        failed: Vec::new(),
    };

    // Track output file paths for post-process renaming
    let mut output_files: Vec<PathBuf> = Vec::new();

    // Configure walk depth
    let walker = if options.recursive {
        WalkDir::new(input_dir)
    } else {
        WalkDir::new(input_dir).max_depth(1)
    };

    // Create output directory if needed
    if let Some(ref out_dir) = options.output_dir
        && !options.dry_run
    {
        fs::create_dir_all(out_dir).map_err(|e| ImgstripError::IoError {
            path: out_dir.clone(),
            source: e,
        })?;
    }

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                let path = e.path().map(|p| p.to_path_buf()).unwrap_or_default();
                report.failed.push((
                    path,
                    ImgstripError::IoError {
                        path: input_dir.to_path_buf(),
                        source: e.into(),
                    },
                ));
                continue;
            }
        };

        // Skip directories themselves
        if entry.file_type().is_dir() {
            continue;
        }

        let path = entry.path();

        // Filter by supported extension
        if !formats::has_supported_extension(path) {
            if options.verbose {
                eprintln!("Skipping non-image file: {}", path.display());
            }
            continue;
        }

        if options.dry_run {
            match operation {
                Operation::Convert { format, .. } => {
                    let output = derive_batch_output(
                        path,
                        input_dir,
                        options.output_dir.as_deref(),
                        Some(*format),
                    );
                    println!("Would convert {} -> {}", path.display(), output.display());
                }
                Operation::Strip => {
                    if let Some(ref out_dir) = options.output_dir {
                        let output = derive_batch_output(path, input_dir, Some(out_dir), None);
                        println!(
                            "Would strip metadata: {} -> {}",
                            path.display(),
                            output.display()
                        );
                    } else {
                        println!("Would strip metadata: {} (in place)", path.display());
                    }
                }
            }
            report.succeeded += 1;
            continue;
        }

        // Execute the operation
        let (result, output_path) = match operation {
            Operation::Convert {
                format,
                quality,
                overwrite,
                strip_metadata,
            } => {
                let output = derive_batch_output(
                    path,
                    input_dir,
                    options.output_dir.as_deref(),
                    Some(*format),
                );
                if let Err(e) = ensure_parent_dir(&output) {
                    report.failed.push((path.to_path_buf(), e));
                    continue;
                }
                if options.verbose {
                    eprintln!("Converting {} -> {}", path.display(), output.display());
                }
                let out_clone = output.clone();
                let res = convert::convert_file(
                    path,
                    &output,
                    *format,
                    *quality,
                    *overwrite,
                    *strip_metadata,
                );
                (res, out_clone)
            }
            Operation::Strip => {
                let out = options.output_dir.as_ref().map(|_| {
                    derive_batch_output(path, input_dir, options.output_dir.as_deref(), None)
                });
                if let Some(ref out) = out
                    && let Err(e) = ensure_parent_dir(out)
                {
                    report.failed.push((path.to_path_buf(), e));
                    continue;
                }
                if options.verbose {
                    if let Some(ref out) = out {
                        eprintln!(
                            "Stripping metadata: {} -> {}",
                            path.display(),
                            out.display()
                        );
                    } else {
                        eprintln!("Stripping metadata: {}", path.display());
                    }
                }
                let effective_output = out.clone().unwrap_or_else(|| path.to_path_buf());
                let res = metadata::strip(path, out.as_deref());
                (res, effective_output)
            }
        };

        match result {
            Ok(()) => {
                report.succeeded += 1;
                if options.rename_prefix.is_some() {
                    output_files.push(output_path);
                }
            }
            Err(e) => report.failed.push((path.to_path_buf(), e)),
        }
    }

    // Post-process: rename output files if --rename was specified
    if let Some(ref prefix) = options.rename_prefix
        && !output_files.is_empty()
    {
        let rename_report =
            rename::rename_files_with_prefix(&output_files, prefix, options.dry_run, options.verbose);

        // The rename replaces the files we already counted as succeeded,
        // so only add failures (successes are already counted)
        for failure in rename_report.failed {
            // A rename failure doesn't un-do the convert/strip, but we should report it
            report.failed.push(failure);
        }
    }

    Ok(report)
}

/// Ensure the parent directory of a path exists, creating it if needed.
fn ensure_parent_dir(path: &Path) -> Result<(), ImgstripError> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| ImgstripError::IoError {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    Ok(())
}

/// Derive the output path for a file in a batch operation.
///
/// If `output_dir` is set, the file is placed under that directory preserving
/// its relative path from `input_dir`. Otherwise, it stays next to the original.
/// If `format` is provided, the extension is changed to match.
fn derive_batch_output(
    file: &Path,
    input_dir: &Path,
    output_dir: Option<&Path>,
    format: Option<OutputFormat>,
) -> PathBuf {
    let relative = file.strip_prefix(input_dir).unwrap_or(file);

    let mut output = if let Some(out_dir) = output_dir {
        out_dir.join(relative)
    } else {
        file.to_path_buf()
    };

    if let Some(fmt) = format {
        output.set_extension(formats::default_extension(fmt));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};
    use std::io::BufWriter;
    use tempfile::TempDir;

    /// Create a small test image at the given path.
    fn create_test_image(path: &Path, format: OutputFormat) {
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        match format {
            OutputFormat::Jpeg => {
                use image::ImageEncoder;
                use image::codecs::jpeg::JpegEncoder;
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
            _ => {
                img.save_with_format(path, format.to_image_format())
                    .unwrap();
            }
        }
    }

    fn assert_valid_image(path: &Path) {
        let img = image::open(path).expect("should be a valid image");
        assert_eq!(img.width(), 8);
        assert_eq!(img.height(), 8);
    }

    /// Create a directory with 5 mixed-format images.
    fn create_mixed_dir(dir: &Path) {
        create_test_image(&dir.join("a.jpg"), OutputFormat::Jpeg);
        create_test_image(&dir.join("b.png"), OutputFormat::Png);
        create_test_image(&dir.join("c.bmp"), OutputFormat::Bmp);
        create_test_image(&dir.join("d.gif"), OutputFormat::Gif);
        create_test_image(&dir.join("e.tiff"), OutputFormat::Tiff);
    }

    fn default_options() -> BatchOptions {
        BatchOptions {
            recursive: false,
            dry_run: false,
            output_dir: None,
            verbose: false,
            rename_prefix: None,
        }
    }

    // --- Batch convert tests ---

    #[test]
    fn batch_convert_mixed_formats_to_png() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        create_mixed_dir(&input);

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 5);
        assert!(report.failed.is_empty());

        for name in ["a.png", "b.png", "c.png", "d.png", "e.png"] {
            assert_valid_image(&output.join(name));
        }
    }

    #[test]
    fn batch_strip_jpegs() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        fs::create_dir_all(&input).unwrap();

        for name in ["1.jpg", "2.jpg", "3.jpg", "4.jpg", "5.jpg"] {
            // Create JPEGs with metadata
            let path = input.join(name);
            create_test_image(&path, OutputFormat::Jpeg);

            // Inject fake EXIF
            use img_parts::jpeg::Jpeg;
            use img_parts::{Bytes, ImageEXIF};
            let data = fs::read(&path).unwrap();
            let mut jpeg = Jpeg::from_bytes(data.into()).unwrap();
            jpeg.set_exif(Some(Bytes::from_static(
                b"MM\x00\x2A\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00",
            )));
            let file = fs::File::create(&path).unwrap();
            jpeg.encoder().write_to(file).unwrap();
        }

        let op = Operation::Strip;
        let report = process_directory(&input, &op, &default_options()).unwrap();
        assert_eq!(report.succeeded, 5);
        assert!(report.failed.is_empty());

        // Verify metadata was stripped
        for name in ["1.jpg", "2.jpg", "3.jpg", "4.jpg", "5.jpg"] {
            use img_parts::ImageEXIF;
            use img_parts::jpeg::Jpeg;
            let data = fs::read(input.join(name)).unwrap();
            let jpeg = Jpeg::from_bytes(data.into()).unwrap();
            assert!(
                jpeg.exif().is_none(),
                "{name} should have no EXIF after strip"
            );
        }
    }

    #[test]
    fn recursive_processes_subdirectories() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        create_test_image(&input.join("top.jpg"), OutputFormat::Jpeg);
        create_test_image(&input.join("sub/nested.png"), OutputFormat::Png);

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            recursive: true,
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 2);
        assert_valid_image(&output.join("top.png"));
        assert_valid_image(&output.join("sub/nested.png"));
    }

    #[test]
    fn non_recursive_ignores_subdirectories() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        create_test_image(&input.join("top.jpg"), OutputFormat::Jpeg);
        create_test_image(&input.join("sub/nested.png"), OutputFormat::Png);

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            recursive: false,
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 1);
        assert_valid_image(&output.join("top.png"));
        assert!(!output.join("sub/nested.png").exists());
    }

    #[test]
    fn mixed_directory_skips_non_images() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        fs::create_dir_all(&input).unwrap();
        create_test_image(&input.join("photo.jpg"), OutputFormat::Jpeg);
        fs::write(input.join("readme.txt"), "not an image").unwrap();
        fs::write(input.join("data.csv"), "1,2,3").unwrap();

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            output_dir: Some(dir.path().join("output")),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 1);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn dry_run_creates_no_files() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        create_mixed_dir(&input);

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            dry_run: true,
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 5);
        // Output directory should not be created in dry-run mode
        assert!(!output.exists());
    }

    #[test]
    fn output_dir_created_if_missing() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("new/nested/output");
        fs::create_dir_all(&input).unwrap();
        create_test_image(&input.join("photo.jpg"), OutputFormat::Jpeg);

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 1);
        assert!(output.exists());
        assert_valid_image(&output.join("photo.png"));
    }

    #[test]
    fn subdirectory_structure_preserved() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        create_test_image(&input.join("a/b/deep.jpg"), OutputFormat::Jpeg);
        create_test_image(&input.join("a/shallow.png"), OutputFormat::Png);

        let op = Operation::Convert {
            format: OutputFormat::WebP,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            recursive: true,
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 2);
        assert_valid_image(&output.join("a/b/deep.webp"));
        assert_valid_image(&output.join("a/shallow.webp"));
    }

    #[test]
    fn corrupted_file_does_not_stop_batch() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fs::create_dir_all(&input).unwrap();

        // One good file, one corrupted
        create_test_image(&input.join("good.jpg"), OutputFormat::Jpeg);
        fs::write(input.join("bad.jpg"), b"not a real jpeg").unwrap();

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            output_dir: Some(output.clone()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed.len(), 1);
        assert_valid_image(&output.join("good.png"));
    }

    #[test]
    fn empty_directory() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("empty");
        fs::create_dir_all(&input).unwrap();

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let report = process_directory(&input, &op, &default_options()).unwrap();
        assert_eq!(report.succeeded, 0);
        assert!(report.failed.is_empty());
    }

    // --- Composability: --rename with convert/strip ---

    #[test]
    fn batch_convert_with_rename() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fs::create_dir_all(&input).unwrap();

        create_test_image(&input.join("cat.jpg"), OutputFormat::Jpeg);
        create_test_image(&input.join("apple.png"), OutputFormat::Png);
        create_test_image(&input.join("bee.bmp"), OutputFormat::Bmp);

        let op = Operation::Convert {
            format: OutputFormat::Png,
            quality: 90,
            overwrite: false,
            strip_metadata: false,
        };
        let opts = BatchOptions {
            output_dir: Some(output.clone()),
            rename_prefix: Some("vacation".to_string()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 3);
        assert!(report.failed.is_empty());

        // Files should be renamed with prefix
        assert!(output.join("vacation-01.png").exists());
        assert!(output.join("vacation-02.png").exists());
        assert!(output.join("vacation-03.png").exists());

        // Original convert output names should not exist
        assert!(!output.join("apple.png").exists());
        assert!(!output.join("bee.png").exists());
        assert!(!output.join("cat.png").exists());
    }

    #[test]
    fn batch_strip_with_rename() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fs::create_dir_all(&input).unwrap();

        create_test_image(&input.join("x.jpg"), OutputFormat::Jpeg);
        create_test_image(&input.join("a.jpg"), OutputFormat::Jpeg);

        let op = Operation::Strip;
        let opts = BatchOptions {
            output_dir: Some(output.clone()),
            rename_prefix: Some("photo".to_string()),
            ..default_options()
        };

        let report = process_directory(&input, &op, &opts).unwrap();
        assert_eq!(report.succeeded, 2);
        assert!(report.failed.is_empty());

        assert!(output.join("photo-01.jpg").exists());
        assert!(output.join("photo-02.jpg").exists());

        // Original names should not exist in output
        assert!(!output.join("a.jpg").exists());
        assert!(!output.join("x.jpg").exists());
    }
}
