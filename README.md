# imgstrip

A lightweight command-line tool for image format conversion and metadata stripping, written in Rust.

## Features

- **Convert** images between JPEG, PNG, WebP, BMP, TIFF, and GIF
- **Strip** EXIF, XMP, IPTC, and ICC metadata without re-encoding pixels
- **HEIC/HEIF input** support (statically linked, no runtime dependencies)
- **Batch processing** of entire directories, with recursive traversal
- **Metadata preservation** by default during conversion
- **Info** command for inspecting image metadata

Convert and strip are independent operations. Converting preserves metadata by default. Stripping does not re-encode pixels.

## Installation

Requires Rust 1.85+ (2024 edition).

```bash
git clone <repo-url>
cd imgstrip
cargo build --release
```

The binary is at `target/release/imgstrip`. No shared libraries required — `libheif` is statically linked.

## Usage

### Convert

```bash
# Single file
imgstrip convert photo.heic --format jpeg
imgstrip convert photo.png --format webp -o output.webp

# Directory
imgstrip convert ./photos --format png -o ./converted
imgstrip convert ./photos --format jpeg --recursive -o ./converted

# With options
imgstrip convert photo.jpg --format jpeg --quality 75
imgstrip convert photo.jpg --format png --strip-metadata
imgstrip convert ./photos --format png --dry-run
```

### Strip metadata

```bash
# In place
imgstrip strip photo.jpg

# To a new file
imgstrip strip photo.jpg -o stripped.jpg

# Directory
imgstrip strip ./photos
imgstrip strip ./photos --recursive -o ./stripped
```

### Inspect metadata

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

EXIF fields are grouped by category (Camera, Exposure, Date/Time, Image, Author, GPS) and formatted for readability — integer codes are shown as human-readable labels, rationals as standard notation (e.g. `1/125 s`, `f/1.8`), and GPS coordinates as decimal degrees. Empty sections are omitted.

## Supported Formats

| Format | Input | Output | Metadata strip | Metadata preserve on convert |
|--------|-------|--------|----------------|------------------------------|
| JPEG   | Yes   | Yes    | Yes (lossless) | Yes                          |
| PNG    | Yes   | Yes    | Yes (lossless) | Yes                          |
| WebP   | Yes   | Yes*   | Yes            | Yes                          |
| BMP    | Yes   | Yes    | N/A            | N/A                          |
| TIFF   | Yes   | Yes    | Yes            | No**                         |
| GIF    | Yes   | Yes    | N/A            | N/A                          |
| HEIC   | Yes   | No     | Yes            | N/A (input only)             |

\* WebP output is lossless only (`--quality` is ignored for WebP).
\** TIFF metadata preservation is not yet supported due to format constraints.

## Global Options

```
-v, --verbose    Print detailed progress information
-q, --quiet      Suppress all output except errors
-h, --help       Print help
-V, --version    Print version
```

## License

See LICENSE file for details.
