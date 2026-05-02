//! Typed errors for the DASH downloader.

use rdlp_core::RdlpError;

/// Errors raised during DASH manifest parsing, segment fetch, and mux.
#[derive(Debug, thiserror::Error)]
pub enum DashError {
    /// MPD XML failed to parse or violated structural expectations.
    #[error("Failed to parse MPD: {0}")]
    Parse(String),
    /// MPD declares dynamic / live presentation; not yet supported.
    #[error("Dynamic/live MPD is not yet supported")]
    DynamicMpd,
    /// MPD contains DRM protection (ContentProtection elements).
    #[error("DRM-protected MPD is not supported")]
    DrmProtected,
    /// No video representation usable by this downloader was found.
    #[error("MPD has no usable video representation")]
    NoVideoRepresentation,
    /// No audio representation usable by this downloader was found.
    #[error("MPD has no usable audio representation")]
    NoAudioRepresentation,
    /// A segment HTTP fetch failed past the retry budget.
    #[error("Segment fetch failed: {0}")]
    SegmentFetch(String),
    /// FFmpeg mux of the resolved video + audio streams failed.
    #[error("Mux failed: {0}")]
    Mux(String),
}

impl From<DashError> for RdlpError {
    fn from(e: DashError) -> Self {
        match e {
            DashError::DynamicMpd
            | DashError::DrmProtected
            | DashError::NoVideoRepresentation
            | DashError::NoAudioRepresentation => RdlpError::Extraction {
                message: e.to_string(),
                url: None,
            },
            DashError::Parse(_) | DashError::SegmentFetch(_) | DashError::Mux(_) => {
                RdlpError::Download {
                    message: e.to_string(),
                    url: None,
                }
            }
        }
    }
}
