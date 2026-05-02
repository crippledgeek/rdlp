//! End-to-end test: per-Repr DASH Formats → `FormatSelector` picks `bv*+ba` →
//! orchestrator dispatches two `download_format` calls via `download_merge_pair`
//! → both stream intermediates written to disk with the correct bytes.
//!
//! # Scope reduction
//!
//! This test does NOT exercise the `MergeStage` / `FFmpeg` mux. The segment
//! bodies are placeholder bytes (not valid `fMP4`), so the pipeline would fail
//! deterministically. What this test uniquely covers:
//!
//!   - `select_format` returns `DownloadPlan::Merge` with the **1080p** video
//!     Rep (higher bandwidth wins) and the single audio Rep when given the
//!     explicit selector `bv*+ba`.
//!   - `download_merge_pair` fetches each Rep's pre-resolved fragments and
//!     writes the concatenated segment bytes to the expected intermediate files.
//!   - The 720p Rep's mock endpoints are never called (wrong Rep filtered out
//!     by selector).
//!
//! `FFmpeg` mux correctness is separately covered by
//! `crates/rdlp-downloader/tests/dash_e2e.rs`.

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

fn make_video_repr(
    id: &str,
    height: u32,
    bandwidth_kbps: u32,
    fragment_urls: Vec<String>,
) -> Format {
    let mut f = Format::new(
        id,
        format!("http://mock-ignored/{id}.mpd"),
        "mp4",
        DownloadProtocol::HttpDashSegments,
    );
    f.vcodec = Codec::from("avc1".to_string());
    f.acodec = Codec::Absent;
    f.height = Some(height);
    f.width = Some(height * 16 / 9);
    f.tbr = Some(f64::from(bandwidth_kbps));
    f.vbr = Some(f64::from(bandwidth_kbps));
    f.fragments = Some(
        fragment_urls
            .into_iter()
            .map(|url| Fragment {
                url,
                duration: Some(4.0),
                filesize: None,
            })
            .collect(),
    );
    f
}

fn make_audio_repr(id: &str, abr_kbps: u32, fragment_urls: Vec<String>) -> Format {
    let mut f = Format::new(
        id,
        format!("http://mock-ignored/{id}.mpd"),
        "m4a",
        DownloadProtocol::HttpDashSegments,
    );
    f.vcodec = Codec::Absent;
    f.acodec = Codec::from("mp4a".to_string());
    f.abr = Some(f64::from(abr_kbps));
    f.tbr = Some(f64::from(abr_kbps));
    f.fragments = Some(
        fragment_urls
            .into_iter()
            .map(|url| Fragment {
                url,
                duration: Some(4.0),
                filesize: None,
            })
            .collect(),
    );
    f
}

fn info_with_formats(formats: Vec<Format>) -> InfoDict {
    let mut info = InfoDict::new(
        "dash-e2e-test",
        "DASH Per-Repr E2E Test",
        "test",
        "http://mock-ignored/manifest.mpd",
    );
    info.formats = formats;
    info
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

/// `bv*+ba` on a 3-format `InfoDict` (720p video-only, 1080p video-only, audio-only)
/// must:
///
///   A. Return `DownloadPlan::Merge { video=1080p, audio=a1 }`.
///   B. Fetch 1080p + audio segments exactly once each.
///   C. Never fetch 720p segments.
///   D. Write correct concatenated bytes to the intermediate files.
#[tokio::test]
async fn test_bv_star_plus_ba_selects_1080p_and_downloads_both_streams() {
    // -----------------------------------------------------------------------
    // 1. Spin up mockito and register segment endpoints
    // -----------------------------------------------------------------------
    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    // 720p segments — must NOT be called
    let _v720_init = server
        .mock("GET", "/v720/init.mp4")
        .with_body(b"V720INIT")
        .expect(0)
        .create_async()
        .await;
    let _v720_seg = server
        .mock("GET", "/v720/seg1.m4s")
        .with_body(b"V720S1")
        .expect(0)
        .create_async()
        .await;

    // 1080p segments — must be called exactly once each
    let _v1080_init = server
        .mock("GET", "/v1080/init.mp4")
        .with_body(b"V1080INIT")
        .expect(1)
        .create_async()
        .await;
    let _v1080_seg = server
        .mock("GET", "/v1080/seg1.m4s")
        .with_body(b"V1080S1")
        .expect(1)
        .create_async()
        .await;

    // Audio segments — must be called exactly once each
    let _a_init = server
        .mock("GET", "/a1/init.mp4")
        .with_body(b"A1INIT")
        .expect(1)
        .create_async()
        .await;
    let _a_seg = server
        .mock("GET", "/a1/seg1.m4s")
        .with_body(b"A1S1")
        .expect(1)
        .create_async()
        .await;

    // -----------------------------------------------------------------------
    // 2. Build per-Repr Formats with pre-resolved fragment URLs pointing at
    //    the mockito server
    // -----------------------------------------------------------------------
    let v720 = make_video_repr(
        "dash_v_720p",
        720,
        2_000, // 2 Mbps
        vec![
            format!("{base}/v720/init.mp4"),
            format!("{base}/v720/seg1.m4s"),
        ],
    );

    let v1080 = make_video_repr(
        "dash_v_1080p",
        1080,
        5_000, // 5 Mbps — higher bandwidth, preferred by bv*
        vec![
            format!("{base}/v1080/init.mp4"),
            format!("{base}/v1080/seg1.m4s"),
        ],
    );

    let a1 = make_audio_repr(
        "dash_a_1",
        128,
        vec![format!("{base}/a1/init.mp4"), format!("{base}/a1/seg1.m4s")],
    );

    let info = info_with_formats(vec![v720, v1080, a1]);

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
    // 4. Assertion A: selector chooses Merge { video=1080p, audio=a1 }
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
                 check that bv*+ba returns two formats",
                f.format_id
            )
        }
    };

    assert_eq!(
        video_fmt.format_id, "dash_v_1080p",
        "bv* must pick the higher-bandwidth 1080p Rep over 720p"
    );
    assert_eq!(
        audio_fmt.format_id, "dash_a_1",
        "ba must pick the only audio Rep"
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

    // Assertion D — video bytes
    let video_bytes = tokio::fs::read(&outcome.video_path)
        .await
        .expect("video intermediate must exist on disk");
    assert_eq!(
        video_bytes, b"V1080INITV1080S1",
        "video intermediate must contain init + seg1 for the 1080p Rep"
    );

    // Assertion D — audio bytes
    let audio_bytes = tokio::fs::read(&outcome.audio_path)
        .await
        .expect("audio intermediate must exist on disk");
    assert_eq!(
        audio_bytes, b"A1INITA1S1",
        "audio intermediate must contain init + seg1 for the audio Rep"
    );

    // Assertions B and C are enforced by mockito's `.expect(N)` guards: the
    // mock objects drop at end of scope and panic if the call count doesn't
    // match.
}
