//! HLS master/media playlist expansion into pre-resolved `Format` entries
//! with `Format.fragments` populated.

use std::sync::Arc;

use rdlp_types::Format;

/// Refusal/failure modes for [`expand_hls_url`].
///
/// On any error the caller should fall back to the legacy `Format` row
/// (manifest URL with `fragments = None`) so the downloader re-parses the
/// playlist at download time.
#[derive(Debug, thiserror::Error)]
pub enum HlsExpandError {
    /// Master playlist parsed cleanly but contained zero variants.
    #[error("master playlist has no variants")]
    NoVariants,
    /// Media playlist parsed cleanly but contained zero segments.
    #[error("variant playlist has no segments")]
    NoSegments,
    /// Media playlist segment count exceeds the configured cap.
    #[error("variant playlist has too many segments: {count} (max {max})")]
    TooManySegments {
        /// Observed segment count.
        count: usize,
        /// Configured maximum.
        max: usize,
    },
    /// EXT-X-KEY indicated an encryption method other than `NONE`.
    #[error("HLS encryption not supported in pre-resolved path: {0}")]
    Encrypted(String),
    /// Media playlist lacks EXT-X-ENDLIST (live / event stream).
    #[error("HLS live streams not supported in pre-resolved path")]
    LiveStream,
    /// More than one distinct EXT-X-MAP URI seen in the same media playlist.
    #[error("multiple distinct EXT-X-MAP URIs not supported in pre-resolved path")]
    MultipleInitSegments,
    /// EXT-X-MAP carried a BYTERANGE attribute (init-segment subrange).
    #[error("byte-ranged EXT-X-MAP not supported in pre-resolved path")]
    ByteRangedInit,
    /// HTTP fetch of master or media playlist failed.
    #[error("network: {0}")]
    Network(String),
    /// `m3u8_rs` could not parse the playlist body.
    #[error("parse: {0}")]
    Parse(String),
}

/// Expand an HLS URL (master or media playlist) into pre-resolved `Format`
/// entries.
///
/// Auto-detects master vs media via `m3u8_rs::Playlist`. Master playlists
/// expand to one `Format` per variant; media playlists return a single-element
/// `Vec<Format>`.
///
/// Each output `Format` inherits codec/height/format_id/format_note metadata
/// from `seed`, then has `fragments = Some(...)` populated with absolute
/// fragment URLs (resolved against the *media* playlist URL per RFC 8216 §4.1).
///
/// # Errors
///
/// Returns `HlsExpandError` for refusal cases (encrypted, live, multi-init,
/// byte-ranged init, empty variants/segments, network/parse failures). The
/// caller should treat any error as a signal to keep the original `Format`
/// row (legacy fallback).
pub async fn expand_hls_url(
    _seed: &Format,
    _http: Arc<wreq::Client>,
) -> Result<Vec<Format>, HlsExpandError> {
    unimplemented!("filled in by subsequent tasks")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hls_expand_error_displays() {
        let e = HlsExpandError::Encrypted("AES-128".into());
        assert_eq!(
            e.to_string(),
            "HLS encryption not supported in pre-resolved path: AES-128"
        );
    }
}
