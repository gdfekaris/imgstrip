use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ImgstripError {
    #[error("Unsupported input format: {0}")]
    UnsupportedFormat(String),

    #[error("Failed to decode image: {0}")]
    DecodeError(String),

    #[error("Failed to encode to {format}: {source}")]
    EncodeError {
        format: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Metadata operation failed: {0}")]
    MetadataError(String),

    #[error("File I/O error on {path}: {source}")]
    IoError {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("HEIC decoding error: {0}")]
    HeicError(String),

    #[error("Output file already exists: {path} (use --overwrite to replace)")]
    OutputExists { path: PathBuf },
}
