//! HLS (HTTP Live Streaming) size detection module
//!
//! This module provides functionality to detect the total file size of HLS streams
//! by parsing m3u8 playlists and aggregating segment sizes using parallel HTTP requests.
//!
//! # Architecture
//!
//! The size detection process follows these steps:
//! 1. Fetch the m3u8 playlist file
//! 2. Parse the playlist using m3u8-rs to extract segment URLs
//! 3. Make parallel HEAD requests to fetch each segment's Content-Length
//! 4. Sum all segment sizes to get the total file size
//!
//! # Performance
//!
//! - Concurrency: 8 parallel HTTP requests (configurable)
//! - Typical detection time: 2-5 seconds for 100-500 segment playlists
//! - Memory usage: O(n) where n = segment count (~20-200 KB)
//!
//! # Example
//!
//! ```no_run
//! use rdlp_extractor::hls::HlsSizeDetector;
//! use std::sync::Arc;
//!
//! # async fn example() -> rdlp_core::Result<()> {
//! let http_client = Arc::new(reqwest::Client::new());
//! let detector = HlsSizeDetector::new(http_client, false);
//!
//! let m3u8_url = "https://example.com/playlist.m3u8";
//! if let Some(size) = detector.detect_size(m3u8_url).await? {
//!     println!("Total size: {} MB", size / 1_000_000);
//! }
//! # Ok(())
//! # }
//! ```

mod detector;
mod format_detection;
mod segments;
mod types;
mod variants;

// Re-export public API unchanged
pub use detector::HlsSizeDetector;
pub use format_detection::{detect_format_sizes, detect_format_sizes_lazy};
pub use types::{HlsInfo, HlsStreamFlags, HlsVariantInfo};

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_detector_creation() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client.clone(), false);

        assert_eq!(detector.concurrent_requests, 8);
        assert!(!detector.verbose);
    }

    #[test]
    fn test_detector_with_concurrency() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client, false).with_concurrency(16);

        assert_eq!(detector.concurrent_requests, 16);
    }

    #[test]
    fn test_detector_concurrency_minimum() {
        let client = Arc::new(reqwest::Client::new());
        let detector = HlsSizeDetector::new(client, false).with_concurrency(0);

        // Should be clamped to minimum of 1
        assert_eq!(detector.concurrent_requests, 1);
    }

    // Note: Integration tests with real URLs are in the redtube extractor tests
    // because they require network access and are marked with #[ignore]

    use super::variants::infer_muxed_audio;

    #[test]
    fn test_infer_muxed_audio_video_only_no_audio_group() {
        // AV1 with no declared audio and no separate audio rendition
        // → should infer muxed AAC
        assert_eq!(infer_muxed_audio(Some("av1"), None, false), Some("aac"),);
    }

    #[test]
    fn test_infer_muxed_audio_declared_audio_preserved() {
        // h264+aac declared → should keep declared aac
        assert_eq!(
            infer_muxed_audio(Some("h264"), Some("aac"), false),
            Some("aac"),
        );
    }

    #[test]
    fn test_infer_muxed_audio_opus_preserved() {
        // AV1+opus declared → should keep declared opus
        assert_eq!(
            infer_muxed_audio(Some("av1"), Some("opus"), false),
            Some("opus"),
        );
    }

    #[test]
    fn test_infer_muxed_audio_separate_audio_group() {
        // Video only with separate AUDIO rendition → should NOT infer
        assert_eq!(infer_muxed_audio(Some("av1"), None, true), None,);
    }

    #[test]
    fn test_infer_muxed_audio_no_video() {
        // Audio-only stream (no video) → should NOT infer
        assert_eq!(infer_muxed_audio(None, Some("aac"), false), Some("aac"),);
    }

    #[test]
    fn test_infer_muxed_audio_no_codecs() {
        // No codecs declared at all → should NOT infer
        assert_eq!(infer_muxed_audio(None, None, false), None,);
    }

    /// End-to-end: parse a master playlist with AV1-only CODECS and verify
    /// the variant expansion infers muxed AAC.
    #[test]
    fn test_infer_muxed_audio_from_m3u8_parse() {
        let master_m3u8 = "\
            #EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,\
            CODECS=\"av01.0.08M.08\"\n\
            media_1080.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,\
            CODECS=\"avc1.640028,mp4a.40.2\"\n\
            media_720.m3u8\n";

        let playlist = m3u8_rs::parse_playlist_res(master_m3u8.as_bytes()).unwrap();
        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            _ => panic!("Expected master playlist"),
        };

        assert_eq!(master.variants.len(), 2);

        // AV1 variant: video only in CODECS, no AUDIO attribute
        let av1 = &master.variants[0];
        let (v, a) = rdlp_core::parse_hls_codecs(av1.codecs.as_deref().unwrap());
        let inferred = infer_muxed_audio(v, a, av1.audio.is_some());
        assert_eq!(v, Some("av1"));
        assert_eq!(a, None, "Raw CODECS should have no audio");
        assert_eq!(inferred, Some("aac"), "Should infer muxed AAC");

        // h264+aac variant: both declared
        let h264 = &master.variants[1];
        let (v2, a2) = rdlp_core::parse_hls_codecs(h264.codecs.as_deref().unwrap());
        let inferred2 = infer_muxed_audio(v2, a2, h264.audio.is_some());
        assert_eq!(v2, Some("h264"));
        assert_eq!(a2, Some("aac"));
        assert_eq!(inferred2, Some("aac"), "Declared audio preserved");
    }

    /// Parse a master playlist with separate AUDIO group and verify no inference.
    #[test]
    fn test_no_inference_with_audio_rendition() {
        let master_m3u8 = "\
            #EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"AAC\",\
            DEFAULT=YES,URI=\"audio.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,\
            CODECS=\"av01.0.08M.08\",AUDIO=\"audio\"\n\
            video_1080.m3u8\n";

        let playlist = m3u8_rs::parse_playlist_res(master_m3u8.as_bytes()).unwrap();
        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            _ => panic!("Expected master playlist"),
        };

        let variant = &master.variants[0];
        let (v, a) = rdlp_core::parse_hls_codecs(variant.codecs.as_deref().unwrap());
        let inferred = infer_muxed_audio(v, a, variant.audio.is_some());
        assert_eq!(v, Some("av1"));
        assert!(variant.audio.is_some(), "AUDIO attribute should be present");
        assert_eq!(
            inferred, None,
            "Should NOT infer audio when AUDIO group referenced"
        );
    }
}
