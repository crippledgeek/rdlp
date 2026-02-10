//! Error types for post-processing operations.

use rdlp_core::RdlpError;
use std::fmt;
use std::path::PathBuf;
use thiserror::Error;

/// Classification of input container corruption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptionKind {
    /// Matroska/WebM EBML structural errors:
    /// - "invalid as first byte of an EBML number"
    /// - "element exceeds containing master element"
    /// - "Duplicate element"
    MatroskaEbml,
}

impl fmt::Display for CorruptionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MatroskaEbml => write!(f, "matroska ebml"),
        }
    }
}

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

    /// Mux/write operation failed with diagnostic packet context
    #[error(
        "Mux write failed ({operation}): {message} [stream={stream_index}, pts={pts:?}, dts={dts:?}, size={packet_size}, tb={time_base_num}/{time_base_den}]"
    )]
    MuxWriteError {
        message: String,
        operation: String,
        stream_index: usize,
        pts: Option<i64>,
        dts: Option<i64>,
        packet_size: i32,
        time_base_num: i32,
        time_base_den: i32,
    },

    /// Post-processor not found in registry
    #[error("Post-processor not found: {name}")]
    ProcessorNotFound { name: String },

    /// Multiple streams require merge but merge is disabled
    #[error("Multiple streams detected but merging is disabled")]
    MergeRequired,

    /// Temporary file creation failed
    #[error("Failed to create temporary file: {message}")]
    TempFileError { message: String },

    /// Audio normalization failed
    #[error("Audio normalization failed: {message}")]
    NormalizationFailed { message: String },

    /// FFmpeg log capture failed
    #[error("FFmpeg log capture failed: {message}")]
    LogCaptureFailed { message: String },

    /// Input container is structurally corrupt (detected via demuxer log analysis).
    ///
    /// Matroska/WebM EBML errors indicate malformed container structure that
    /// produces invalid packets/timestamps, which cascade into muxer ENOMEM
    /// failures during transcoding.
    #[error("input corrupt ({kind}): {}", path.display())]
    InputCorrupt {
        kind: CorruptionKind,
        path: PathBuf,
        logs: Vec<String>,
    },

    /// Salvage remux failed — input too corrupt to recover.
    #[error("Input corrupt and salvage failed: {message}")]
    SalvageFailed { message: String },
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

    /// True if this is a mux write error (ENOMEM / ENOSPC from the muxer).
    #[must_use]
    pub fn is_mux_write_error(&self) -> bool {
        matches!(self, Self::MuxWriteError { .. })
    }

    /// True if this error may be recoverable by salvage-remuxing and retrying.
    ///
    /// D1: Triggers on mux write errors (ENOMEM/ENOSPC/EIO/EINVAL),
    /// stall watchdog aborts, and input container corruption.
    #[must_use]
    pub fn is_salvage_retryable(&self) -> bool {
        matches!(self, Self::MuxWriteError { .. } | Self::InputCorrupt { .. })
    }

    /// True if this error indicates memory exhaustion (ENOMEM/-12).
    #[must_use]
    pub fn is_enomem(&self) -> bool {
        match self {
            Self::MuxWriteError { message, .. } => message.contains("ret=-12"),
            _ => false,
        }
    }

    /// True if this error indicates an I/O error (EIO/-5).
    #[must_use]
    pub fn is_eio(&self) -> bool {
        match self {
            Self::MuxWriteError { message, .. } => message.contains("ret=-5"),
            _ => false,
        }
    }

    /// True if this error indicates a mux stall (watchdog abort).
    #[must_use]
    pub fn is_stall(&self) -> bool {
        match self {
            Self::MuxWriteError { operation, .. } => operation.contains("watchdog"),
            _ => false,
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
            | PostProcessError::FFmpegLibraryError { .. }
            | PostProcessError::MuxWriteError { .. } => RdlpError::FFmpeg(error.to_string()),

            PostProcessError::InputNotFound { .. }
            | PostProcessError::OutputExists { .. }
            | PostProcessError::IoError { .. }
            | PostProcessError::TempFileError { .. } => RdlpError::PostProcess(error.to_string()),

            PostProcessError::InputCorrupt { .. } | PostProcessError::SalvageFailed { .. } => {
                RdlpError::FFmpeg(error.to_string())
            }

            _ => RdlpError::PostProcess(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mux_write_error_display() {
        let err = PostProcessError::MuxWriteError {
            message: "Not enough space".into(),
            operation: "av_interleaved_write_frame".into(),
            stream_index: 0,
            pts: Some(12345),
            dts: Some(12340),
            packet_size: 1024,
            time_base_num: 1,
            time_base_den: 1000,
        };
        let display = err.to_string();
        assert!(display.contains("av_interleaved_write_frame"));
        assert!(display.contains("Not enough space"));
        assert!(display.contains("stream=0"));
        assert!(display.contains("pts=Some(12345)"));
        assert!(display.contains("dts=Some(12340)"));
        assert!(display.contains("size=1024"));
        assert!(display.contains("tb=1/1000"));
    }

    #[test]
    fn test_mux_write_error_display_none_timestamps() {
        let err = PostProcessError::MuxWriteError {
            message: "write failed".into(),
            operation: "av_write_frame".into(),
            stream_index: 1,
            pts: None,
            dts: None,
            packet_size: 512,
            time_base_num: 1,
            time_base_den: 48000,
        };
        let display = err.to_string();
        assert!(display.contains("av_write_frame"));
        assert!(display.contains("pts=None"));
        assert!(display.contains("dts=None"));
        assert!(display.contains("tb=1/48000"));
    }

    #[test]
    fn test_mux_write_error_direct_write_display() {
        let err = PostProcessError::MuxWriteError {
            message: "ret=-12 (Cannot allocate memory), dur=1024".into(),
            operation: "av_write_frame".into(),
            stream_index: 0,
            pts: Some(48000),
            dts: Some(48000),
            packet_size: 2048,
            time_base_num: 1,
            time_base_den: 48000,
        };
        let display = err.to_string();
        assert!(display.contains("av_write_frame"));
        assert!(display.contains("Cannot allocate memory"));
        assert!(display.contains("stream=0"));
        assert!(display.contains("size=2048"));
    }

    #[test]
    fn test_mux_write_error_converts_to_rdlp_ffmpeg() {
        let err = PostProcessError::MuxWriteError {
            message: "test".into(),
            operation: "av_interleaved_write_frame".into(),
            stream_index: 0,
            pts: None,
            dts: None,
            packet_size: 0,
            time_base_num: 1,
            time_base_den: 1000,
        };
        let rdlp_err: RdlpError = err.into();
        match rdlp_err {
            RdlpError::FFmpeg(msg) => assert!(msg.contains("Mux write failed")),
            other => panic!("expected RdlpError::FFmpeg, got: {other:?}"),
        }
    }

    #[test]
    fn test_is_mux_write_error() {
        let mux_err = PostProcessError::MuxWriteError {
            message: "ENOMEM".into(),
            operation: "av_write_frame".into(),
            stream_index: 0,
            pts: Some(100),
            dts: Some(100),
            packet_size: 1024,
            time_base_num: 1,
            time_base_den: 1000,
        };
        assert!(mux_err.is_mux_write_error());

        let lib_err = PostProcessError::FFmpegLibraryError {
            message: "something else".into(),
        };
        assert!(!lib_err.is_mux_write_error());
    }

    #[test]
    fn test_is_enomem() {
        let err = PostProcessError::MuxWriteError {
            message: "ret=-12 (Cannot allocate memory), dur=23, pb->pos=1024".into(),
            operation: "av_write_frame".into(),
            stream_index: 0,
            pts: Some(0),
            dts: Some(0),
            packet_size: 1024,
            time_base_num: 1,
            time_base_den: 1000,
        };
        assert!(err.is_enomem());
        assert!(err.is_mux_write_error());
        assert!(err.is_salvage_retryable());

        let non_enomem = PostProcessError::MuxWriteError {
            message: "ret=-28 (No space left on device)".into(),
            operation: "av_write_frame".into(),
            stream_index: 0,
            pts: None,
            dts: None,
            packet_size: 512,
            time_base_num: 1,
            time_base_den: 48000,
        };
        assert!(!non_enomem.is_enomem());

        let lib_err = PostProcessError::FFmpegLibraryError {
            message: "ret=-12".into(),
        };
        assert!(!lib_err.is_enomem());
    }

    #[test]
    fn test_is_salvage_retryable() {
        let mux_err = PostProcessError::MuxWriteError {
            message: "ENOMEM".into(),
            operation: "av_write_frame".into(),
            stream_index: 0,
            pts: None,
            dts: None,
            packet_size: 0,
            time_base_num: 1,
            time_base_den: 1000,
        };
        assert!(mux_err.is_salvage_retryable());

        let corrupt_err = PostProcessError::InputCorrupt {
            kind: CorruptionKind::MatroskaEbml,
            path: PathBuf::from("/tmp/test.mkv"),
            logs: vec![],
        };
        assert!(corrupt_err.is_salvage_retryable());

        let norm_err = PostProcessError::NormalizationFailed {
            message: "decoder failed".into(),
        };
        assert!(!norm_err.is_salvage_retryable());

        let no_audio = PostProcessError::NoAudioStream;
        assert!(!no_audio.is_salvage_retryable());
    }
}
