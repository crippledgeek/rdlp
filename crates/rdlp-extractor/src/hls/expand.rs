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

/// Build per-media-playlist `Format` from parsed bytes + media-playlist URL.
///
/// Internal helper — used by `expand_hls_url` after fetching. Separated out
/// so refusal logic can be unit-tested without HTTP mocking.
#[allow(dead_code)] // wired into expand_hls_url in Task 7
fn expand_media_playlist(
    seed: &Format,
    media_playlist_url: &str,
    bytes: &[u8],
) -> Result<Format, HlsExpandError> {
    let playlist = m3u8_rs::parse_media_playlist_res(bytes)
        .map_err(|e| HlsExpandError::Parse(format!("media playlist: {e:?}")))?;

    // Encryption refusal — check every segment's `key` field.
    for seg in &playlist.segments {
        if let Some(key) = &seg.key {
            match &key.method {
                m3u8_rs::KeyMethod::None => {}
                m3u8_rs::KeyMethod::AES128 => {
                    return Err(HlsExpandError::Encrypted("AES-128".into()));
                }
                m3u8_rs::KeyMethod::SampleAES => {
                    return Err(HlsExpandError::Encrypted("SAMPLE-AES".into()));
                }
                m3u8_rs::KeyMethod::Other(s) => {
                    return Err(HlsExpandError::Encrypted(s.clone()));
                }
            }
        }
    }

    // Subsequent tasks fill in the rest. For now, error so we don't return
    // a half-built Format.
    let _ = (seed, media_playlist_url);
    Err(HlsExpandError::Parse(
        "expand_media_playlist not yet complete".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::DownloadProtocol;

    #[test]
    fn hls_expand_error_displays() {
        let e = HlsExpandError::Encrypted("AES-128".into());
        assert_eq!(
            e.to_string(),
            "HLS encryption not supported in pre-resolved path: AES-128"
        );
    }

    fn seed() -> Format {
        Format::new(
            "hls",
            "https://h.com/master.m3u8",
            "m3u8",
            DownloadProtocol::M3u8,
        )
    }

    const MEDIA_AES128: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=AES-128,URI=\"https://h.com/key\"
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    const MEDIA_SAMPLE_AES: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://h.com/key\"
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    const MEDIA_KEY_NONE: &[u8] = b"\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-KEY:METHOD=NONE
#EXTINF:6.0,
seg-1.ts
#EXT-X-ENDLIST
";

    #[test]
    fn refuses_aes128_encryption() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_AES128)
            .expect_err("must refuse AES-128");
        assert!(matches!(err, HlsExpandError::Encrypted(ref s) if s == "AES-128"));
    }

    #[test]
    fn refuses_sample_aes_encryption() {
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_SAMPLE_AES)
            .expect_err("must refuse SAMPLE-AES");
        assert!(matches!(err, HlsExpandError::Encrypted(ref s) if s == "SAMPLE-AES"));
    }

    #[test]
    fn key_method_none_is_not_encrypted() {
        // Not actually encrypted — the stub still errors with Parse for now,
        // but the encryption check is bypassed. Task 7 makes this assert
        // success once the full body is in place.
        let err = expand_media_playlist(&seed(), "https://h.com/v.m3u8", MEDIA_KEY_NONE)
            .expect_err("stub still errors");
        assert!(!matches!(err, HlsExpandError::Encrypted(_)));
    }
}
