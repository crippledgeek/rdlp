//! End-to-end test: HLS Formats with a separate audio rendition →
//! `FormatSelector` picks `bv*+ba` → orchestrator dispatches two
//! `download_format` calls via `download_merge_pair` → both stream
//! intermediates written to disk with the correct bytes.
//!
//! # Scope reduction
//!
//! This test does NOT exercise the `MergeStage` / `FFmpeg` mux. The segment
//! bodies are placeholder bytes (not valid MPEG-TS or AAC), so the pipeline
//! would fail deterministically. What this test uniquely covers:
//!
//!   - `select_format` returns `DownloadPlan::Merge` with the **1080p** video
//!     variant (higher bandwidth wins) and the single audio rendition when
//!     given the explicit selector `bv*+ba`.
//!   - `download_merge_pair` fetches each stream's pre-resolved fragments and
//!     writes the concatenated segment bytes to the expected intermediate files.
//!   - The 720p variant's mock endpoints are never called (wrong variant
//!     filtered out by selector).
//!
//! `FFmpeg` mux correctness is separately covered by the HLS downloader tests.

// Test scope: large fixture-driven happy-path test exceeds the workspace's
// strict per-fn line cap and holds a `mockito::Server` across the body so
// `expect(0)` mocks fire on drop at end of scope.
#![allow(clippy::too_many_lines, clippy::significant_drop_tightening)]

use crate::events::Event;
use crate::handle::DownloadId;
use crate::orchestrator::{DownloadPlan, Orchestrator};
use rdlp_types::{Codec, Config, DownloadProtocol, Format, Fragment, InfoDict};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn orchestrator_with_config(config: Config) -> Orchestrator {
    let (tx, _rx) = mpsc::channel::<Event>(64);
    Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    )
}

fn make_hls_video_variant(
    id: &str,
    height: u32,
    bandwidth_kbps: u32,
    segment_urls: Vec<String>,
) -> Format {
    let mut f = Format::new(
        id,
        format!("http://mock-ignored/{id}.m3u8"),
        "ts",
        DownloadProtocol::M3u8Native,
    );
    f.vcodec = Codec::from("avc1.640028".to_string());
    f.acodec = Codec::Absent;
    f.height = Some(height);
    f.width = Some(height * 16 / 9);
    f.tbr = Some(f64::from(bandwidth_kbps));
    f.vbr = Some(f64::from(bandwidth_kbps));
    f.fragments = Some(
        segment_urls
            .into_iter()
            .map(|url| Fragment {
                url,
                byte_range: None,
                init_url: None,
                init_byte_range: None,
                duration: Some(6.0),
                filesize: None,
            })
            .collect(),
    );
    f
}

fn make_hls_audio_rendition(id: &str, abr_kbps: u32, segment_urls: Vec<String>) -> Format {
    let mut f = Format::new(
        id,
        format!("http://mock-ignored/{id}.m3u8"),
        "aac",
        DownloadProtocol::M3u8Native,
    );
    f.vcodec = Codec::Absent;
    f.acodec = Codec::from("mp4a.40.2".to_string());
    f.abr = Some(f64::from(abr_kbps));
    f.tbr = Some(f64::from(abr_kbps));
    f.fragments = Some(
        segment_urls
            .into_iter()
            .map(|url| Fragment {
                url,
                byte_range: None,
                init_url: None,
                init_byte_range: None,
                duration: Some(6.0),
                filesize: None,
            })
            .collect(),
    );
    f
}

fn info_with_formats(formats: Vec<Format>) -> InfoDict {
    let mut info = InfoDict::new(
        "hls-e2e-test",
        "HLS Separate-Audio Rendition E2E Test",
        "test",
        "http://mock-ignored/master.m3u8",
    );
    info.formats = formats;
    info
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

/// `bv*+ba` on a 3-format `InfoDict` (720p video-only, 1080p video-only,
/// audio-only rendition) must:
///
///   A. Return `DownloadPlan::Merge { video=1080p, audio=audio_en }`.
///   B. Fetch 1080p + audio segments exactly once each.
///   C. Never fetch 720p segments.
///   D. Write correct concatenated bytes to the intermediate files.
#[tokio::test]
async fn hls_bv_star_plus_ba_auto_pairs_separate_audio_rendition() {
    // -----------------------------------------------------------------------
    // 1. Spin up mockito and register segment endpoints
    // -----------------------------------------------------------------------
    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    // 720p segments — must NOT be called
    let _v720_seg0 = server
        .mock("GET", "/v720_seg0.ts")
        .with_body(b"V720S0")
        .expect(0)
        .create_async()
        .await;
    let _v720_seg1 = server
        .mock("GET", "/v720_seg1.ts")
        .with_body(b"V720S1")
        .expect(0)
        .create_async()
        .await;

    // 1080p segments — must be called exactly once each
    let _v1080_seg0 = server
        .mock("GET", "/v1080_seg0.ts")
        .with_body(b"V1080S0")
        .expect(1)
        .create_async()
        .await;
    let _v1080_seg1 = server
        .mock("GET", "/v1080_seg1.ts")
        .with_body(b"V1080S1")
        .expect(1)
        .create_async()
        .await;

    // Audio rendition segments — must be called exactly once each
    let _audio_seg0 = server
        .mock("GET", "/audio_en_seg0.aac")
        .with_body(b"AUDS0")
        .expect(1)
        .create_async()
        .await;
    let _audio_seg1 = server
        .mock("GET", "/audio_en_seg1.aac")
        .with_body(b"AUDS1")
        .expect(1)
        .create_async()
        .await;

    // -----------------------------------------------------------------------
    // 2. Build HLS Formats with pre-resolved fragment URLs pointing at
    //    the mockito server. This mirrors what expand_hls_master produces.
    // -----------------------------------------------------------------------
    let v720 = make_hls_video_variant(
        "v720",
        720,
        800, // 800 kbps — lower bandwidth, should be rejected by bv*
        vec![
            format!("{base}/v720_seg0.ts"),
            format!("{base}/v720_seg1.ts"),
        ],
    );

    let v1080 = make_hls_video_variant(
        "v1080",
        1080,
        2_500, // 2.5 Mbps — higher bandwidth, preferred by bv*
        vec![
            format!("{base}/v1080_seg0.ts"),
            format!("{base}/v1080_seg1.ts"),
        ],
    );

    let audio_en = make_hls_audio_rendition(
        "audio_en",
        128,
        vec![
            format!("{base}/audio_en_seg0.aac"),
            format!("{base}/audio_en_seg1.aac"),
        ],
    );

    let info = info_with_formats(vec![v720, v1080, audio_en]);

    // -----------------------------------------------------------------------
    // 3. Create orchestrator with explicit `bv*+ba` selector
    //    (explicit selector makes the test FFmpeg-availability-independent)
    // -----------------------------------------------------------------------
    let config = Config {
        format: Some("bv*+ba".to_string()),
        ..Default::default()
    };
    let orch = orchestrator_with_config(config);

    // -----------------------------------------------------------------------
    // 4. Assertion A: selector chooses Merge { video=1080p, audio=audio_en }
    // -----------------------------------------------------------------------
    let plan = orch
        .select_format(&info, false)
        .await
        .expect("select_format must succeed")
        .expect("select_format must return Some(plan)");

    let (video_fmt, audio_fmt) = match plan {
        DownloadPlan::Merge { video, audio } => (video, audio),
        DownloadPlan::Single(f) => {
            panic!(
                "Expected DownloadPlan::Merge but got Single({}) — \
                 check that bv*+ba returns two formats for HLS with audio rendition",
                f.format_id
            )
        }
    };

    assert_eq!(
        video_fmt.format_id, "v1080",
        "bv* must pick the higher-bandwidth 1080p variant over 720p"
    );
    assert_eq!(
        audio_fmt.format_id, "audio_en",
        "ba must pick the only audio rendition"
    );

    // -----------------------------------------------------------------------
    // 5. Assertions B–D: download_merge_pair fetches the right segments and
    //    writes the expected bytes
    // -----------------------------------------------------------------------
    let dir = TempDir::new().expect("tempdir");
    let base_output = dir.path().join("test-video.mp4");

    let outcome = orch
        .download_merge_pair(&video_fmt, &audio_fmt, &base_output)
        .await
        .expect("download_merge_pair must not error")
        .expect("download_merge_pair must not be cancelled");

    // Assertion D — video bytes (concatenated segments in order)
    let video_bytes = tokio::fs::read(&outcome.video_path)
        .await
        .expect("video intermediate must exist on disk");
    assert_eq!(
        video_bytes, b"V1080S0V1080S1",
        "video intermediate must contain seg0 + seg1 for the 1080p variant"
    );

    // Assertion D — audio bytes
    let audio_bytes = tokio::fs::read(&outcome.audio_path)
        .await
        .expect("audio intermediate must exist on disk");
    assert_eq!(
        audio_bytes, b"AUDS0AUDS1",
        "audio intermediate must contain seg0 + seg1 for the audio rendition"
    );

    // Assertions B and C are enforced by mockito's `.expect(N)` guards: the
    // mock objects drop at end of scope and panic if the call count doesn't
    // match.
}
