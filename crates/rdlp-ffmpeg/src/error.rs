//! Error types for post-processing operations.

use rdlp_core::RdlpError;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during post-processing.
#[derive(Debug, Error)]
#[allow(missing_docs)] // Field names are self-explanatory, variant docs describe them
pub enum PostProcessError {
    /// FFmpeg operation failed
    #[error("FFmpeg failed: {message}")]
    FFmpegFailed {
        message: String,
        #[source]
        source: Option<std::io::Error>,
    },

    /// Input file not found
    #[error("Input file not found: {}", path.display())]
    InputNotFound { path: PathBuf },

    /// Output file already exists
    #[error("Output file already exists: {}", path.display())]
    OutputExists { path: PathBuf },

    /// Unsupported format for operation
    #[error("Unsupported format '{format}' for {operation}")]
    UnsupportedFormat { format: String, operation: String },

    /// No audio stream found in input
    #[error("No audio stream found in input file")]
    NoAudioStream,

    /// No video stream found in input
    #[error("No video stream found in input file")]
    NoVideoStream,

    /// Codec not supported
    #[error("Codec '{codec}' is not supported for {operation}")]
    UnsupportedCodec { codec: String, operation: String },

    /// Invalid quality specification
    #[error("Invalid quality specification: {spec}")]
    InvalidQuality { spec: String },

    /// Metadata embedding failed
    #[error("Failed to embed metadata: {message}")]
    MetadataError { message: String },

    /// Thumbnail embedding failed
    #[error("Failed to embed thumbnail: {message}")]
    ThumbnailError { message: String },

    /// Chapter embedding failed
    #[error("Failed to embed chapters: {message}")]
    ChapterError { message: String },

    /// I/O error during post-processing
    #[error("I/O error: {message}")]
    IoError {
        message: String,
        #[source]
        source: std::io::Error,
    },

    /// FFmpeg library initialization failed
    #[error("FFmpeg library initialization failed: {message}")]
    FFmpegInitFailed { message: String },

    /// FFmpeg library operation failed
    #[error("FFmpeg library error: {message}")]
    FFmpegLibraryError { message: String },

    /// Post-processor not found in registry
    #[error("Post-processor not found: {name}")]
    ProcessorNotFound { name: String },

    /// Multiple streams require merge but merge is disabled
    #[error("Multiple streams detected but merging is disabled")]
    MergeRequired,

    /// Temporary file creation failed
    #[error("Failed to create temporary file: {message}")]
    TempFileError { message: String },
}

impl PostProcessError {
    /// Create an FFmpeg failed error
    pub fn ffmpeg_failed(message: impl Into<String>) -> Self {
        Self::FFmpegFailed {
            message: message.into(),
            source: None,
        }
    }

    /// Create an I/O error
    pub fn io_error(message: impl Into<String>, source: std::io::Error) -> Self {
        Self::IoError {
            message: message.into(),
            source,
        }
    }
}

/// Result type for post-processing operations.
pub type Result<T> = std::result::Result<T, PostProcessError>;

/// Convert ffmpeg-the-third errors into PostProcessError.
impl From<ffmpeg_the_third::Error> for PostProcessError {
    fn from(e: ffmpeg_the_third::Error) -> Self {
        PostProcessError::FFmpegLibraryError {
            message: e.to_string(),
        }
    }
}

/// Convert PostProcessError to RdlpError for use at trait boundaries.
impl From<PostProcessError> for RdlpError {
    fn from(error: PostProcessError) -> Self {
        match error {
            PostProcessError::FFmpegFailed { .. }
            | PostProcessError::FFmpegInitFailed { .. }
            | PostProcessError::FFmpegLibraryError { .. } => RdlpError::FFmpeg(error.to_string()),

            PostProcessError::InputNotFound { .. }
            | PostProcessError::OutputExists { .. }
            | PostProcessError::IoError { .. }
            | PostProcessError::TempFileError { .. } => RdlpError::PostProcess(error.to_string()),

            _ => RdlpError::PostProcess(error.to_string()),
        }
    }
}
