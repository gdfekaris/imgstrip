# imgstrip

A lightweight command-line tool for image format conversion and metadata stripping, written in Rust.

imgstrip does two things well:

- **Converts** images between common formats (JPEG, PNG, WebP, BMP, TIFF, GIF), with HEIC/HEIF as an input format.
- **Strips** metadata (EXIF, XMP, IPTC, ICC profiles) from images without re-encoding the pixels — your image data stays untouched.

It also has an **info** command that lets you inspect what metadata an image contains.

By default, converting an image preserves its metadata. You can opt in to stripping metadata during conversion with the `--strip-metadata` flag. Stripping on its own never re-encodes the image — it only removes metadata.

## Installation

Requires Rust 1.85+ (2024 edition).

```bash
git clone <repo-url>
cd imgstrip
cargo build --release
```

The compiled binary will be at `target/release/imgstrip`. No shared libraries are needed at runtime — `libheif` is statically linked into the binary.

To make it available from anywhere, you can copy or symlink it into a directory on your PATH:

```bash
cp target/release/imgstrip ~/.local/bin/
```

## Quick start

```bash
# Convert a HEIC photo to JPEG
imgstrip convert photo.heic --format jpeg

# Strip all metadata from a JPEG (modifies the file in place)
imgstrip strip photo.jpg

# See what metadata an image contains
imgstrip info photo.jpg
```

## Commands

imgstrip has three commands: `convert`, `strip`, and `info`. Each is described in detail below.

---

### `convert` — Convert images to a different format

Converts one image (or a whole directory of images) to a target format. Metadata is preserved by default.

```bash
imgstrip convert <INPUT> --format <FORMAT> [OPTIONS]
```

`<INPUT>` is a path to an image file or a directory containing images.

#### Options

| Option | Short | Description |
|--------|-------|-------------|
| `--format <FORMAT>` | `-f` | **(Required)** Target format. One of: `jpeg`, `png`, `webp`, `bmp`, `tiff`, `gif`. |
| `--output <PATH>` | `-o` | Where to write the output. For a single file this is the output file path; for a directory this is the output directory. If omitted, the converted file is written next to the original with the new extension (e.g. `photo.png` becomes `photo.jpg`). |
| `--strip-metadata` | `-s` | Remove all metadata from the converted output. Without this flag, metadata is carried over from the source image. |
| `--quality <1-100>` | | JPEG quality setting (1-100). Default is 90. Ignored for all other formats, including WebP (see note below). |
| `--recursive` | `-r` | When the input is a directory, also process images in subdirectories. |
| `--overwrite` | | Allow overwriting existing output files. Without this flag, imgstrip will stop with an error if the output file already exists. |
| `--dry-run` | | Show what would be done without actually writing any files. Useful for previewing a batch operation. |

#### Examples

**Convert a single file:**

```bash
# Convert a HEIC photo to JPEG (output: photo.jpg in the same directory)
imgstrip convert photo.heic --format jpeg

# Convert to WebP and specify an output path
imgstrip convert photo.png --format webp -o converted/photo.webp

# Convert with lower JPEG quality to reduce file size
imgstrip convert photo.png --format jpeg --quality 60

# Convert and strip metadata in one step
imgstrip convert photo.jpg --format png --strip-metadata
```

**Convert a directory of images:**

```bash
# Convert all images in a folder to PNG, writing results to a new folder
imgstrip convert ./photos --format png -o ./converted

# Same, but also process subfolders
imgstrip convert ./photos --format jpeg --recursive -o ./converted

# Preview what would happen without writing anything
imgstrip convert ./photos --format png --dry-run
```

---

### `strip` — Remove metadata from images

Strips all EXIF, XMP, IPTC, and ICC metadata from one image or a directory of images. The image pixels are not re-encoded — for JPEG and PNG this is fully lossless at the byte level.

```bash
imgstrip strip <INPUT> [OPTIONS]
```

`<INPUT>` is a path to an image file or a directory containing images.

**Important:** By default, `strip` modifies the file in place. Use `-o` to write the stripped version to a different location and leave the original untouched.

#### Options

| Option | Short | Description |
|--------|-------|-------------|
| `--output <PATH>` | `-o` | Write the stripped file to a new path instead of overwriting the original. For a directory, this is the output directory. |
| `--recursive` | `-r` | When the input is a directory, also process images in subdirectories. |
| `--dry-run` | | Show what would be done without actually writing any files. |

#### Examples

```bash
# Strip metadata from a single file (modifies the file in place)
imgstrip strip photo.jpg

# Strip metadata but write to a new file, keeping the original intact
imgstrip strip photo.jpg -o photo-clean.jpg

# Strip metadata from all images in a directory
imgstrip strip ./photos

# Strip metadata from a directory tree, writing results to a new folder
imgstrip strip ./photos --recursive -o ./stripped

# Preview what would be stripped
imgstrip strip ./photos --dry-run
```

---

### `info` — Inspect image metadata

Displays a summary of an image file's metadata, including format, dimensions, and all EXIF fields grouped into readable categories.

```bash
imgstrip info <FILE>
```

`<FILE>` is a path to a single image file.

#### Example

```bash
imgstrip info photo.jpg
```

Output:

```
File:   photo.jpg
Format: JPEG
Size:   2.4 MB
Dimensions: 4032x3024
Color type: Rgb8

Metadata:
  EXIF: present
  XMP:  present
  ICC:  present
  IPTC: absent

Camera:
  Make        Apple
  Model       iPhone 15 Pro
  Lens Model  iPhone 15 Pro back triple camera 6.765mm f/1.78

Exposure:
  Exposure Time     1/125 s
  F-Number          f/1.8
  ISO               50
  Focal Length      6.8 mm
  Exposure Program  Program AE
  Metering Mode     Multi-segment
  Flash             Did not fire (auto mode)

Date/Time:
  Date Taken    2024:12:25 10:30:00
  Date Created  2024:12:25 10:30:00

Image:
  Orientation  Horizontal (normal)
  Color Space  sRGB

Author:
  Software  17.2

GPS:
  Coordinates  37.774929, -122.419416
  Altitude     12.3 m
```

EXIF fields are grouped by category and formatted for readability — numeric codes are shown as human-readable labels, exposure values use standard notation (e.g. `1/125 s`, `f/1.8`), and GPS coordinates are displayed as decimal degrees. Sections with no data are omitted.

## Global options

These options can be used with any command, and can be placed before or after the command name:

```bash
imgstrip [GLOBAL OPTIONS] <COMMAND> ...
imgstrip <COMMAND> [GLOBAL OPTIONS] ...
```

| Option | Short | Description |
|--------|-------|-------------|
| `--verbose` | `-v` | Print detailed progress information (what files are being processed, what operations are being performed). |
| `--quiet` | `-q` | Suppress all output except errors. |
| `--help` | `-h` | Print help information. You can also use `--help` after a command name (e.g. `imgstrip convert --help`) to see help for that specific command. |
| `--version` | `-V` | Print the version number. |

## Supported formats

| Format | Input | Output | Metadata strip | Metadata preserved on convert |
|--------|-------|--------|----------------|-------------------------------|
| JPEG   | Yes   | Yes    | Yes (lossless) | Yes                           |
| PNG    | Yes   | Yes    | Yes (lossless) | Yes                           |
| WebP   | Yes   | Yes*   | Yes            | Yes                           |
| BMP    | Yes   | Yes    | N/A            | N/A                           |
| TIFF   | Yes   | Yes    | Yes            | No**                          |
| GIF    | Yes   | Yes    | N/A            | N/A                           |
| HEIC   | Yes   | No     | Yes            | N/A (input only)              |

\* WebP output is lossless only. The `--quality` flag has no effect when converting to WebP.

\** TIFF metadata preservation during conversion is not currently supported due to format constraints. Metadata will be silently dropped when converting to TIFF.

## License

See LICENSE file for details.
