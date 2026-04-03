use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "imgstrip",
    version,
    about = "Lightweight image format conversion and metadata stripping"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Print detailed progress information
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Convert image(s) to a different format
    Convert(ConvertArgs),

    /// Strip metadata from image(s) in place
    Strip(StripArgs),

    /// Display metadata summary for an image
    Info(InfoArgs),
}

#[derive(Parser)]
pub struct ConvertArgs {
    /// Path to an image file or directory
    pub input: PathBuf,

    /// Target format: jpeg, png, webp, bmp, tiff, gif
    #[arg(short, long)]
    pub format: String,

    /// Output file or directory (default: same location, new extension)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Also strip metadata from the converted output
    #[arg(short, long)]
    pub strip_metadata: bool,

    /// Process directories recursively
    #[arg(short, long)]
    pub recursive: bool,

    /// Overwrite existing output files
    #[arg(long)]
    pub overwrite: bool,

    /// Show what would be done without writing files
    #[arg(long)]
    pub dry_run: bool,

    /// JPEG quality (1-100, default: 90). Ignored for other formats.
    #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u8).range(1..=100))]
    pub quality: u8,
}

#[derive(Parser)]
pub struct StripArgs {
    /// Path to an image file or directory
    pub input: PathBuf,

    /// Write stripped file to a new path (default: overwrite in place)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Process directories recursively
    #[arg(short, long)]
    pub recursive: bool,

    /// Show what would be done without writing files
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser)]
pub struct InfoArgs {
    /// Path to an image file
    pub file: PathBuf,
}
