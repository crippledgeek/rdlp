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

    /// Lock in that `EXT-X-MEDIA TYPE=AUDIO` rendition groups are surfaced
    /// as dedicated audio-only entries in the parsed master — the structural
    /// precondition for `detect_hls_variants` to emit paired video-only +
    /// audio-only `HlsVariantInfo` rows for bv+ba auto-pairing.
    ///
    /// Regression guard for the XHamster AV1 case documented in
    /// `project_format-selection-path-one.md`: before this fix the expander
    /// ignored `master.alternatives` entirely, dropping every audio rendition.
    #[test]
    fn test_ext_x_media_audio_surfaced_in_master() {
        let master_m3u8 = "\
            #EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio-aac\",NAME=\"English\",\
            DEFAULT=YES,AUTOSELECT=YES,LANGUAGE=\"en\",URI=\"audio-en.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"English\",\
            DEFAULT=NO,LANGUAGE=\"en\",URI=\"subs-en.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,\
            CODECS=\"av01.0.08M.08,mp4a.40.2\",AUDIO=\"audio-aac\"\n\
            video-1080-av1.m3u8\n";

        let playlist = m3u8_rs::parse_playlist_res(master_m3u8.as_bytes()).unwrap();
        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            _ => panic!("Expected master playlist"),
        };

        let audio_renditions: Vec<_> = master
            .alternatives
            .iter()
            .filter(|m| m.media_type == m3u8_rs::AlternativeMediaType::Audio)
            .collect();
        assert_eq!(audio_renditions.len(), 1, "one audio rendition expected");

        let audio = audio_renditions[0];
        assert_eq!(audio.group_id, "audio-aac");
        assert_eq!(audio.language.as_deref(), Some("en"));
        assert_eq!(audio.uri.as_deref(), Some("audio-en.m3u8"));
        assert_eq!(audio.name, "English");
    }

    /// After the HLS expander fix, an XHamster-shaped master with AV1
    /// video-only + separate AAC audio group yields BOTH a video-only
    /// `HlsVariantInfo` AND an audio-only `HlsVariantInfo`. Before the
    /// fix only the video-only entry was emitted (audio rendition was
    /// dropped because `master.alternatives` was never read).
    ///
    /// Tested against the pure `expand_master_variants` helper so no
    /// HTTP / SSRF gate is in the path.
    #[test]
    fn test_expand_master_variants_emits_audio_only_from_media_group() {
        use super::variants::expand_master_variants;

        let master_m3u8 = "\
            #EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"Stereo\",\
            DEFAULT=YES,AUTOSELECT=YES,URI=\"audio.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,\
            CODECS=\"av01.0.08M.08,mp4a.40.2\",AUDIO=\"aac\"\n\
            video-1080-av1.m3u8\n";

        let playlist = m3u8_rs::parse_playlist_res(master_m3u8.as_bytes()).unwrap();
        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            _ => panic!("Expected master playlist"),
        };
        let base = url::Url::parse("https://cdn.example.com/p/master.m3u8").unwrap();

        let variants = expand_master_variants(&master, &base);
        let video_only: Vec<_> = variants.iter().filter(|v| !v.is_audio_only).collect();
        let audio_only: Vec<_> = variants.iter().filter(|v| v.is_audio_only).collect();

        assert_eq!(video_only.len(), 1, "expect one video-only variant");
        assert_eq!(audio_only.len(), 1, "expect one audio-only rendition");

        let v = video_only[0];
        assert_eq!(v.video_codec.as_deref(), Some("av1"));
        assert_eq!(v.audio_group_id.as_deref(), Some("aac"));
        assert_eq!(v.resolution, Some((1920, 1080)));
        assert_eq!(
            v.media_playlist_url,
            "https://cdn.example.com/p/video-1080-av1.m3u8"
        );

        let a = audio_only[0];
        assert!(a.video_codec.is_none());
        assert_eq!(a.audio_codec.as_deref(), Some("aac"));
        assert_eq!(a.audio_group_id.as_deref(), Some("aac"));
        assert_eq!(a.rendition_name.as_deref(), Some("Stereo"));
        assert!(a.resolution.is_none());
        assert_eq!(
            a.media_playlist_url,
            "https://cdn.example.com/p/audio.m3u8"
        );
    }

    /// Masters without EXT-X-MEDIA (typical XVideos / XNXX / older
    /// XHamster case) must continue to emit only muxed video+audio
    /// entries — the fix is strictly additive and must not break
    /// single-manifest sites.
    #[test]
    fn test_expand_master_variants_muxed_only_unchanged() {
        use super::variants::expand_master_variants;

        let master_m3u8 = "\
            #EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,\
            CODECS=\"avc1.640028,mp4a.40.2\"\n\
            v-720.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,\
            CODECS=\"avc1.640028,mp4a.40.2\"\n\
            v-1080.m3u8\n";

        let playlist = m3u8_rs::parse_playlist_res(master_m3u8.as_bytes()).unwrap();
        let master = match playlist {
            m3u8_rs::Playlist::MasterPlaylist(m) => m,
            _ => panic!("Expected master playlist"),
        };
        let base = url::Url::parse("https://cdn.example.com/master.m3u8").unwrap();

        let variants = expand_master_variants(&master, &base);
        assert_eq!(variants.len(), 2, "two muxed variants, no audio rendition");
        assert!(
            variants.iter().all(|v| !v.is_audio_only),
            "no audio-only rows when EXT-X-MEDIA is absent"
        );
        assert!(
            variants
                .iter()
                .all(|v| v.audio_codec.as_deref() == Some("aac")),
            "muxed AAC is preserved on both variants"
        );
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
