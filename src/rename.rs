use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::ImgstripError;
use crate::formats;

/// A single planned rename operation.
#[derive(Debug, Clone)]
pub struct RenamePlan {
    pub source: PathBuf,
    pub target: PathBuf,
}

/// Result of a rename batch.
#[derive(Debug)]
pub struct RenameReport {
    pub succeeded: usize,
    pub failed: Vec<(PathBuf, ImgstripError)>,
}

/// Options controlling the rename operation.
pub struct RenameOptions {
    pub recursive: bool,
    pub dry_run: bool,
    pub output_dir: Option<PathBuf>,
    pub verbose: bool,
}

/// Validate a rename prefix.
fn validate_prefix(prefix: &str) -> Result<(), ImgstripError> {
    if prefix.is_empty() {
        return Err(ImgstripError::InvalidArgument(
            "prefix cannot be empty".to_string(),
        ));
    }
    if prefix.contains('/') || prefix.contains('\\') {
        return Err(ImgstripError::InvalidArgument(
            "prefix cannot contain path separators".to_string(),
        ));
    }
    Ok(())
}

/// Compute the zero-padding width for a given file count.
/// Minimum width is 2 (always at least one leading zero).
fn padding_width(count: usize) -> usize {
    if count == 0 {
        return 2;
    }
    let digits = format!("{count}").len();
    digits.max(2)
}

/// Collect image files from a single directory (non-recursive),
/// filter by supported extension, and sort alphabetically by filename.
fn collect_and_sort_images(dir: &Path) -> Result<Vec<PathBuf>, ImgstripError> {
    let entries = fs::read_dir(dir).map_err(|e| ImgstripError::IoError {
        path: dir.to_path_buf(),
        source: e,
    })?;

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| ImgstripError::IoError {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_file() && formats::has_supported_extension(&path) {
            files.push(path);
        }
    }

    files.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .cmp(b.file_name().unwrap_or_default())
    });

    Ok(files)
}

/// Build the rename plan for a single directory.
///
/// Given sorted files and a prefix, produces RenamePlan entries mapping each
/// source to `<prefix>-<padded_number>.<ext>` in the target directory.
pub fn plan_directory(
    files: &[PathBuf],
    prefix: &str,
    output_dir: Option<&Path>,
) -> Vec<RenamePlan> {
    if files.is_empty() {
        return Vec::new();
    }

    let width = padding_width(files.len());
    let mut plan = Vec::with_capacity(files.len());

    for (i, source) in files.iter().enumerate() {
        let number = i + 1;
        let ext = source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let new_name = if ext.is_empty() {
            format!("{prefix}-{number:0>width$}")
        } else {
            format!("{prefix}-{number:0>width$}.{ext}")
        };

        let target_dir = output_dir.unwrap_or_else(|| source.parent().unwrap_or(Path::new(".")));
        let target = target_dir.join(new_name);

        plan.push(RenamePlan {
            source: source.clone(),
            target,
        });
    }

    plan
}

/// Execute a rename plan.
///
/// For in-place renames (source and target in same directory), uses a two-phase
/// strategy to avoid conflicts:
///   Phase 1: rename all sources to temporary names
///   Phase 2: rename all temps to final target names
///
/// When output_dir differs from source, copies files directly (no conflicts possible).
pub fn execute_plan(plan: &[RenamePlan], dry_run: bool, verbose: bool) -> RenameReport {
    let mut report = RenameReport {
        succeeded: 0,
        failed: Vec::new(),
    };

    if plan.is_empty() {
        return report;
    }

    if dry_run {
        for entry in plan {
            println!(
                "Would rename {} -> {}",
                entry.source.display(),
                entry.target.display()
            );
            report.succeeded += 1;
        }
        return report;
    }

    // Determine if this is an out-of-place operation (copy, not rename)
    let is_copy = plan.first().map_or(false, |entry| {
        entry.source.parent() != entry.target.parent()
    });

    if is_copy {
        execute_copy(plan, verbose, &mut report);
    } else {
        execute_in_place(plan, verbose, &mut report);
    }

    report
}

/// Copy files to a new directory with new names. Originals are untouched.
fn execute_copy(plan: &[RenamePlan], verbose: bool, report: &mut RenameReport) {
    for entry in plan {
        // Ensure parent directory exists
        if let Some(parent) = entry.target.parent()
            && !parent.exists()
        {
            if let Err(e) = fs::create_dir_all(parent) {
                report.failed.push((
                    entry.source.clone(),
                    ImgstripError::IoError {
                        path: parent.to_path_buf(),
                        source: e,
                    },
                ));
                continue;
            }
        }

        match fs::copy(&entry.source, &entry.target) {
            Ok(_) => {
                report.succeeded += 1;
                if verbose {
                    eprintln!(
                        "Copied {} -> {}",
                        entry.source.display(),
                        entry.target.display()
                    );
                }
            }
            Err(e) => {
                report.failed.push((
                    entry.source.clone(),
                    ImgstripError::IoError {
                        path: entry.target.clone(),
                        source: e,
                    },
                ));
            }
        }
    }
}

/// Rename files in place using two-phase strategy to avoid conflicts.
fn execute_in_place(plan: &[RenamePlan], verbose: bool, report: &mut RenameReport) {
    // Phase 1: rename all sources to temp names
    let mut temp_mappings: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // (original, temp, target)

    for (i, entry) in plan.iter().enumerate() {
        let dir = entry.source.parent().unwrap_or(Path::new("."));
        let ext = entry
            .source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let temp_name = if ext.is_empty() {
            format!(".imgstrip-rename-{i:06}")
        } else {
            format!(".imgstrip-rename-{i:06}.{ext}")
        };
        let temp_path = dir.join(temp_name);

        match fs::rename(&entry.source, &temp_path) {
            Ok(()) => {
                temp_mappings.push((entry.source.clone(), temp_path, entry.target.clone()));
            }
            Err(e) => {
                // Phase 1 failure: roll back all successful renames so far
                for (orig, tmp, _) in temp_mappings.iter().rev() {
                    let _ = fs::rename(tmp, orig);
                }
                report.failed.push((
                    entry.source.clone(),
                    ImgstripError::IoError {
                        path: entry.source.clone(),
                        source: e,
                    },
                ));
                return;
            }
        }
    }

    // Phase 2: rename all temps to final target names
    for (original, temp, target) in &temp_mappings {
        match fs::rename(temp, target) {
            Ok(()) => {
                report.succeeded += 1;
                if verbose {
                    eprintln!(
                        "Renamed {} -> {}",
                        original.display(),
                        target.display()
                    );
                }
            }
            Err(e) => {
                // Try to restore from temp to original
                let _ = fs::rename(temp, original);
                report.failed.push((
                    original.clone(),
                    ImgstripError::IoError {
                        path: target.clone(),
                        source: e,
                    },
                ));
            }
        }
    }
}

/// Top-level entry point: rename images in a directory (and optionally subdirectories).
///
/// Numbering resets per subdirectory.
pub fn rename_directory(
    input_dir: &Path,
    prefix: &str,
    options: &RenameOptions,
) -> Result<RenameReport, ImgstripError> {
    validate_prefix(prefix)?;

    let mut aggregate = RenameReport {
        succeeded: 0,
        failed: Vec::new(),
    };

    // Collect directories to process
    let dirs_to_process = if options.recursive {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for entry in WalkDir::new(input_dir) {
            let entry = entry.map_err(|e| {
                let path = e.path().map(|p| p.to_path_buf()).unwrap_or_default();
                ImgstripError::IoError {
                    path,
                    source: e.into(),
                }
            })?;
            if entry.file_type().is_dir() {
                dirs.push(entry.path().to_path_buf());
            }
        }
        dirs.sort();
        dirs
    } else {
        vec![input_dir.to_path_buf()]
    };

    for dir in &dirs_to_process {
        let files = collect_and_sort_images(dir)?;
        if files.is_empty() {
            continue;
        }

        // Compute the output directory for this subdirectory
        let out_dir = options.output_dir.as_ref().map(|out| {
            let relative = dir.strip_prefix(input_dir).unwrap_or(dir);
            out.join(relative)
        });

        let plan = plan_directory(&files, prefix, out_dir.as_deref());
        let sub_report = execute_plan(&plan, options.dry_run, options.verbose);

        aggregate.succeeded += sub_report.succeeded;
        aggregate.failed.extend(sub_report.failed);
    }

    Ok(aggregate)
}

/// Rename a list of already-existing files in place using a prefix.
///
/// Used by the batch module to rename output files after convert/strip operations.
/// Files are grouped by parent directory, sorted alphabetically, and renamed
/// with numbering that resets per directory.
pub fn rename_files_with_prefix(
    files: &[PathBuf],
    prefix: &str,
    dry_run: bool,
    verbose: bool,
) -> RenameReport {
    let mut aggregate = RenameReport {
        succeeded: 0,
        failed: Vec::new(),
    };

    if files.is_empty() {
        return aggregate;
    }

    // Group files by parent directory
    let mut by_dir: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for file in files {
        let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        by_dir.entry(dir).or_default().push(file.clone());
    }

    for (_dir, mut dir_files) in by_dir {
        dir_files.sort_by(|a, b| {
            a.file_name()
                .unwrap_or_default()
                .cmp(b.file_name().unwrap_or_default())
        });

        // Rename in-place (files are already in their final directory)
        let plan = plan_directory(&dir_files, prefix, None);
        let sub_report = execute_plan(&plan, dry_run, verbose);

        aggregate.succeeded += sub_report.succeeded;
        aggregate.failed.extend(sub_report.failed);
    }

    aggregate
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};
    use std::io::BufWriter;
    use tempfile::TempDir;

    /// Create a small test image at the given path.
    fn create_test_image(path: &Path) {
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(8, 8, |x, y| {
            image::Rgb([(x * 30) as u8, (y * 30) as u8, 128])
        }));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        if path.extension().and_then(|e| e.to_str()) == Some("jpg")
            || path.extension().and_then(|e| e.to_str()) == Some("jpeg")
        {
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
        } else {
            let fmt = match path.extension().and_then(|e| e.to_str()) {
                Some("png") => image::ImageFormat::Png,
                Some("webp") => image::ImageFormat::WebP,
                Some("bmp") => image::ImageFormat::Bmp,
                Some("tiff") | Some("tif") => image::ImageFormat::Tiff,
                Some("gif") => image::ImageFormat::Gif,
                _ => image::ImageFormat::Png,
            };
            img.save_with_format(path, fmt).unwrap();
        }
    }

    // --- padding_width tests ---

    #[test]
    fn padding_width_minimum_is_two() {
        assert_eq!(padding_width(0), 2);
        assert_eq!(padding_width(1), 2);
        assert_eq!(padding_width(9), 2);
        assert_eq!(padding_width(10), 2);
        assert_eq!(padding_width(99), 2);
    }

    #[test]
    fn padding_width_scales_up() {
        assert_eq!(padding_width(100), 3);
        assert_eq!(padding_width(999), 3);
        assert_eq!(padding_width(1000), 4);
        assert_eq!(padding_width(9999), 4);
    }

    // --- validate_prefix tests ---

    #[test]
    fn validate_prefix_rejects_empty() {
        let err = validate_prefix("").unwrap_err();
        assert!(matches!(err, ImgstripError::InvalidArgument(_)));
    }

    #[test]
    fn validate_prefix_rejects_path_separators() {
        let err = validate_prefix("foo/bar").unwrap_err();
        assert!(matches!(err, ImgstripError::InvalidArgument(_)));

        let err = validate_prefix("foo\\bar").unwrap_err();
        assert!(matches!(err, ImgstripError::InvalidArgument(_)));
    }

    #[test]
    fn validate_prefix_accepts_valid() {
        validate_prefix("vacation").unwrap();
        validate_prefix("my-photos").unwrap();
        validate_prefix("photo_set").unwrap();
        validate_prefix("2024-trip").unwrap();
    }

    // --- collect_and_sort_images tests ---

    #[test]
    fn collect_and_sort_alphabetical() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("cat.jpg"));
        create_test_image(&dir.path().join("apple.png"));
        create_test_image(&dir.path().join("bee.webp"));

        let files = collect_and_sort_images(dir.path()).unwrap();
        let names: Vec<&str> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["apple.png", "bee.webp", "cat.jpg"]);
    }

    #[test]
    fn collect_and_sort_ignores_non_images() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("photo.jpg"));
        fs::write(dir.path().join("readme.txt"), "not an image").unwrap();
        fs::write(dir.path().join("data.csv"), "1,2,3").unwrap();

        let files = collect_and_sort_images(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().unwrap().to_str().unwrap(),
            "photo.jpg"
        );
    }

    #[test]
    fn collect_and_sort_ignores_subdirectories() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("photo.jpg"));
        create_test_image(&dir.path().join("subdir/nested.png"));

        let files = collect_and_sort_images(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn collect_and_sort_empty_directory() {
        let dir = TempDir::new().unwrap();
        let files = collect_and_sort_images(dir.path()).unwrap();
        assert!(files.is_empty());
    }

    // --- plan_directory tests ---

    #[test]
    fn plan_basic() {
        let files = vec![
            PathBuf::from("/photos/apple.png"),
            PathBuf::from("/photos/bee.webp"),
            PathBuf::from("/photos/cat.jpg"),
        ];

        let plan = plan_directory(&files, "vacation", None);
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0].target, PathBuf::from("/photos/vacation-01.png"));
        assert_eq!(plan[1].target, PathBuf::from("/photos/vacation-02.webp"));
        assert_eq!(plan[2].target, PathBuf::from("/photos/vacation-03.jpg"));
    }

    #[test]
    fn plan_preserves_extensions() {
        let files = vec![
            PathBuf::from("/dir/a.jpeg"),
            PathBuf::from("/dir/b.tiff"),
            PathBuf::from("/dir/c.gif"),
        ];

        let plan = plan_directory(&files, "img", None);
        assert_eq!(plan[0].target, PathBuf::from("/dir/img-01.jpeg"));
        assert_eq!(plan[1].target, PathBuf::from("/dir/img-02.tiff"));
        assert_eq!(plan[2].target, PathBuf::from("/dir/img-03.gif"));
    }

    #[test]
    fn plan_with_output_dir() {
        let files = vec![
            PathBuf::from("/input/apple.png"),
            PathBuf::from("/input/bee.jpg"),
        ];

        let plan = plan_directory(&files, "photo", Some(Path::new("/output")));
        assert_eq!(plan[0].target, PathBuf::from("/output/photo-01.png"));
        assert_eq!(plan[1].target, PathBuf::from("/output/photo-02.jpg"));
    }

    #[test]
    fn plan_padding_scales_with_count() {
        let files: Vec<PathBuf> = (0..150)
            .map(|i| PathBuf::from(format!("/dir/img{i:04}.jpg")))
            .collect();

        let plan = plan_directory(&files, "pic", None);
        assert_eq!(plan[0].target, PathBuf::from("/dir/pic-001.jpg"));
        assert_eq!(plan[9].target, PathBuf::from("/dir/pic-010.jpg"));
        assert_eq!(plan[99].target, PathBuf::from("/dir/pic-100.jpg"));
        assert_eq!(plan[149].target, PathBuf::from("/dir/pic-150.jpg"));
    }

    #[test]
    fn plan_single_file() {
        let files = vec![PathBuf::from("/dir/photo.jpg")];
        let plan = plan_directory(&files, "solo", None);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].target, PathBuf::from("/dir/solo-01.jpg"));
    }

    #[test]
    fn plan_empty() {
        let plan = plan_directory(&[], "prefix", None);
        assert!(plan.is_empty());
    }

    // --- execute_plan tests ---

    #[test]
    fn execute_in_place_basic() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("cat.jpg"));
        create_test_image(&dir.path().join("apple.png"));
        create_test_image(&dir.path().join("bee.bmp"));

        let files = collect_and_sort_images(dir.path()).unwrap();
        let plan = plan_directory(&files, "photo", None);
        let report = execute_plan(&plan, false, false);

        assert_eq!(report.succeeded, 3);
        assert!(report.failed.is_empty());
        assert!(dir.path().join("photo-01.png").exists());
        assert!(dir.path().join("photo-02.bmp").exists());
        assert!(dir.path().join("photo-03.jpg").exists());
        assert!(!dir.path().join("apple.png").exists());
        assert!(!dir.path().join("bee.bmp").exists());
        assert!(!dir.path().join("cat.jpg").exists());
    }

    #[test]
    fn execute_in_place_conflict_resolution() {
        let dir = TempDir::new().unwrap();
        // Create files where target names collide with source names
        create_test_image(&dir.path().join("a.jpg"));
        create_test_image(&dir.path().join("photo-01.jpg"));
        create_test_image(&dir.path().join("photo-02.jpg"));

        let files = collect_and_sort_images(dir.path()).unwrap();
        let plan = plan_directory(&files, "photo", None);
        let report = execute_plan(&plan, false, false);

        assert_eq!(report.succeeded, 3);
        assert!(report.failed.is_empty());
        assert!(dir.path().join("photo-01.jpg").exists());
        assert!(dir.path().join("photo-02.jpg").exists());
        assert!(dir.path().join("photo-03.jpg").exists());
        assert!(!dir.path().join("a.jpg").exists());
    }

    #[test]
    fn execute_with_output_dir_copies() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");
        fs::create_dir_all(&input).unwrap();

        create_test_image(&input.join("apple.jpg"));
        create_test_image(&input.join("bee.png"));

        let files = collect_and_sort_images(&input).unwrap();
        let plan = plan_directory(&files, "pic", Some(&output));
        let report = execute_plan(&plan, false, false);

        assert_eq!(report.succeeded, 2);
        assert!(report.failed.is_empty());
        // Originals still exist
        assert!(input.join("apple.jpg").exists());
        assert!(input.join("bee.png").exists());
        // Copies exist with new names
        assert!(output.join("pic-01.jpg").exists());
        assert!(output.join("pic-02.png").exists());
    }

    #[test]
    fn execute_dry_run_changes_nothing() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("cat.jpg"));
        create_test_image(&dir.path().join("apple.png"));

        let files = collect_and_sort_images(dir.path()).unwrap();
        let plan = plan_directory(&files, "photo", None);
        let report = execute_plan(&plan, true, false);

        assert_eq!(report.succeeded, 2);
        // Original files unchanged
        assert!(dir.path().join("cat.jpg").exists());
        assert!(dir.path().join("apple.png").exists());
        // No new files
        assert!(!dir.path().join("photo-01.png").exists());
        assert!(!dir.path().join("photo-02.jpg").exists());
    }

    // --- rename_directory tests ---

    #[test]
    fn rename_directory_basic() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("cat.jpg"));
        create_test_image(&dir.path().join("apple.png"));
        create_test_image(&dir.path().join("bee.bmp"));

        let options = RenameOptions {
            recursive: false,
            dry_run: false,
            output_dir: None,
            verbose: false,
        };

        let report = rename_directory(dir.path(), "vacation", &options).unwrap();
        assert_eq!(report.succeeded, 3);
        assert!(report.failed.is_empty());
        assert!(dir.path().join("vacation-01.png").exists());
        assert!(dir.path().join("vacation-02.bmp").exists());
        assert!(dir.path().join("vacation-03.jpg").exists());
    }

    #[test]
    fn rename_directory_recursive_resets_numbering() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("photos");

        create_test_image(&root.join("c.jpg"));
        create_test_image(&root.join("a.png"));
        create_test_image(&root.join("sub/x.jpg"));
        create_test_image(&root.join("sub/y.png"));
        create_test_image(&root.join("sub/z.bmp"));

        let options = RenameOptions {
            recursive: true,
            dry_run: false,
            output_dir: None,
            verbose: false,
        };

        let report = rename_directory(&root, "img", &options).unwrap();
        assert_eq!(report.succeeded, 5);
        assert!(report.failed.is_empty());

        // Root directory: 2 files, numbering 01-02
        assert!(root.join("img-01.png").exists());
        assert!(root.join("img-02.jpg").exists());

        // Subdirectory: 3 files sorted alphabetically (x.jpg, y.png, z.bmp), numbering resets to 01-03
        assert!(root.join("sub/img-01.jpg").exists());
        assert!(root.join("sub/img-02.png").exists());
        assert!(root.join("sub/img-03.bmp").exists());
    }

    #[test]
    fn rename_directory_with_output_dir() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");

        create_test_image(&input.join("bee.jpg"));
        create_test_image(&input.join("apple.png"));

        let options = RenameOptions {
            recursive: false,
            dry_run: false,
            output_dir: Some(output.clone()),
            verbose: false,
        };

        let report = rename_directory(&input, "pic", &options).unwrap();
        assert_eq!(report.succeeded, 2);

        // Originals still exist
        assert!(input.join("apple.png").exists());
        assert!(input.join("bee.jpg").exists());

        // Copies in output
        assert!(output.join("pic-01.png").exists());
        assert!(output.join("pic-02.jpg").exists());
    }

    #[test]
    fn rename_directory_recursive_with_output_preserves_structure() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("input");
        let output = dir.path().join("output");

        create_test_image(&input.join("top.jpg"));
        create_test_image(&input.join("sub/deep.png"));

        let options = RenameOptions {
            recursive: true,
            dry_run: false,
            output_dir: Some(output.clone()),
            verbose: false,
        };

        let report = rename_directory(&input, "img", &options).unwrap();
        assert_eq!(report.succeeded, 2);

        assert!(output.join("img-01.jpg").exists());
        assert!(output.join("sub/img-01.png").exists());
    }

    #[test]
    fn rename_empty_directory() {
        let dir = TempDir::new().unwrap();

        let options = RenameOptions {
            recursive: false,
            dry_run: false,
            output_dir: None,
            verbose: false,
        };

        let report = rename_directory(dir.path(), "photo", &options).unwrap();
        assert_eq!(report.succeeded, 0);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn rename_single_file_directory() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("lonely.jpg"));

        let options = RenameOptions {
            recursive: false,
            dry_run: false,
            output_dir: None,
            verbose: false,
        };

        let report = rename_directory(dir.path(), "solo", &options).unwrap();
        assert_eq!(report.succeeded, 1);
        assert!(dir.path().join("solo-01.jpg").exists());
        assert!(!dir.path().join("lonely.jpg").exists());
    }

    #[test]
    fn rename_invalid_prefix_empty() {
        let dir = TempDir::new().unwrap();
        let options = RenameOptions {
            recursive: false,
            dry_run: false,
            output_dir: None,
            verbose: false,
        };

        let err = rename_directory(dir.path(), "", &options).unwrap_err();
        assert!(matches!(err, ImgstripError::InvalidArgument(_)));
    }

    #[test]
    fn rename_invalid_prefix_with_separator() {
        let dir = TempDir::new().unwrap();
        let options = RenameOptions {
            recursive: false,
            dry_run: false,
            output_dir: None,
            verbose: false,
        };

        let err = rename_directory(dir.path(), "foo/bar", &options).unwrap_err();
        assert!(matches!(err, ImgstripError::InvalidArgument(_)));
    }

    // --- rename_files_with_prefix tests (used by batch composability) ---

    #[test]
    fn rename_files_with_prefix_basic() {
        let dir = TempDir::new().unwrap();
        create_test_image(&dir.path().join("c.jpg"));
        create_test_image(&dir.path().join("a.png"));
        create_test_image(&dir.path().join("b.bmp"));

        let files = vec![
            dir.path().join("a.png"),
            dir.path().join("b.bmp"),
            dir.path().join("c.jpg"),
        ];

        let report = rename_files_with_prefix(&files, "out", false, false);
        assert_eq!(report.succeeded, 3);
        assert!(report.failed.is_empty());
        assert!(dir.path().join("out-01.png").exists());
        assert!(dir.path().join("out-02.bmp").exists());
        assert!(dir.path().join("out-03.jpg").exists());
    }

    #[test]
    fn rename_files_with_prefix_groups_by_directory() {
        let dir = TempDir::new().unwrap();
        let dir_a = dir.path().join("a");
        let dir_b = dir.path().join("b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        create_test_image(&dir_a.join("x.jpg"));
        create_test_image(&dir_a.join("y.jpg"));
        create_test_image(&dir_b.join("z.png"));

        let files = vec![
            dir_a.join("x.jpg"),
            dir_a.join("y.jpg"),
            dir_b.join("z.png"),
        ];

        let report = rename_files_with_prefix(&files, "pic", false, false);
        assert_eq!(report.succeeded, 3);

        // dir_a: numbering 01-02
        assert!(dir_a.join("pic-01.jpg").exists());
        assert!(dir_a.join("pic-02.jpg").exists());
        // dir_b: numbering resets to 01
        assert!(dir_b.join("pic-01.png").exists());
    }
}
