//! Smoke test: `AdaptiveController` integration for DASH segment fetching.
//!
//! Verifies that `DashDownloader::download_to_file` routes segment fetching
//! through the AIMD adaptive controller rather than a fixed-concurrency
//! `buffer_unordered`.  The signal used is the controller's startup log
//! message ("Adaptive controller:") forwarded via `ProgressCallback::on_log`
//! — if the controller is not wired, no such message is produced.

#![allow(clippy::disallowed_methods, clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use rdlp_core::Downloader;
use rdlp_core::{DownloadProgress, DownloadStats, ProgressCallback};
use rdlp_downloader::DashDownloader;
use tempfile::TempDir;

// ─── Capturing progress callback ─────────────────────────────────────────────

/// `ProgressCallback` that captures every `on_log` message for inspection.
struct LogCapture {
    messages: Arc<Mutex<Vec<String>>>,
}

impl LogCapture {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let cap = Self {
            messages: Arc::clone(&messages),
        };
        (cap, messages)
    }
}

impl ProgressCallback for LogCapture {
    fn on_progress(&self, _info: &DownloadProgress) {}
    fn on_complete(&self, _stats: &DownloadStats) {}
    fn on_error(&self, _error: &str) {}
    fn on_log(&self, message: &str) {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message.to_string());
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Build a minimal `SegmentTemplate` MPD with `count` segments per
/// representation, served from `base_url`.
fn make_mpd(base_url: &str, seg_count: u32) -> String {
    // Each segment covers 2 seconds → total duration = seg_count × 2 s.
    let duration_secs = seg_count * 2;
    let period_dur = format!("PT{duration_secs}S");
    let seg_dur_ts: u64 = 2000; // timescale = 1000 ms, duration = 2000 ts

    format!(
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="{period_dur}" minBufferTime="PT4S">
  <Period duration="{period_dur}">
    <BaseURL>{base_url}/</BaseURL>
    <AdaptationSet contentType="video">
      <Representation id="v1" bandwidth="800000" mimeType="video/mp4">
        <SegmentTemplate timescale="1000" duration="{seg_dur_ts}" startNumber="1"
          initialization="vinit.mp4" media="vseg-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
    <AdaptationSet contentType="audio" lang="en">
      <Representation id="a1" bandwidth="64000" mimeType="audio/mp4">
        <SegmentTemplate timescale="1000" duration="{seg_dur_ts}" startNumber="1"
          initialization="ainit.mp4" media="aseg-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#
    )
}

// ─── Smoke test ───────────────────────────────────────────────────────────────

/// Smoke test: adaptive controller governs a DASH download of 24 video + 24
/// audio segments.
///
/// The test asserts:
///   1. `download_to_file` returns an error **only** on the FFmpeg mux step
///      (placeholder bytes are not valid fMP4 — identical to the `dash_e2e`
///      test contract).  All segment fetches succeed.
///   2. All segment endpoints are called exactly once (the controller did not
///      skip or double-fetch any segment).
///   3. At least one "Adaptive controller:" log message was forwarded via
///      `ProgressCallback::on_log`, proving the `AdaptiveController` was
///      constructed and its log path was exercised.
///
/// Assertion 3 fails before the adaptive wiring is in place (the controller
/// is never instantiated, so its startup message is never produced).
#[tokio::test]
async fn smoke_adaptive_controller_governs_dash_segments() {
    const SEG_COUNT: u32 = 24;

    let mut server = mockito::Server::new_async().await;
    let base = server.url();

    let mpd_body = make_mpd(&base, SEG_COUNT);

    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(&mpd_body)
        .create_async()
        .await;

    // Init segments.
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .create_async()
        .await;
    let _ai = server
        .mock("GET", "/ainit.mp4")
        .with_body(b"AINIT")
        .create_async()
        .await;

    // Register all 24 video segments.
    let mut video_mocks = Vec::new();
    for n in 1..=SEG_COUNT {
        let path = format!("/vseg-{n}.m4s");
        let body = format!("V{n}");
        let m = server
            .mock("GET", path.as_str())
            .with_body(body.as_bytes())
            .expect(1)
            .create_async()
            .await;
        video_mocks.push(m);
    }

    // Register all 24 audio segments.
    let mut audio_mocks = Vec::new();
    for n in 1..=SEG_COUNT {
        let path = format!("/aseg-{n}.m4s");
        let body = format!("A{n}");
        let m = server
            .mock("GET", path.as_str())
            .with_body(body.as_bytes())
            .expect(1)
            .create_async()
            .await;
        audio_mocks.push(m);
    }

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{base}/manifest.mpd");

    let (cb, log_messages) = LogCapture::new();

    let downloader = DashDownloader::new();
    let result = downloader
        .download_to_file(&url, &out, Some(Box::new(cb)))
        .await;

    // Assertion 1 — download should fail only at the mux step (placeholder
    // bytes are not valid fMP4).
    let err = result.expect_err("mux must fail on non-fMP4 placeholder bytes");
    let err_str = format!("{err}").to_lowercase();
    assert!(
        err_str.contains("mux") || err_str.contains("ffmpeg") || err_str.contains("invalid"),
        "expected mux/ffmpeg failure, got: {err_str}"
    );

    // Assertion 2 — every segment mock must have been called exactly once.
    for m in video_mocks {
        m.assert_async().await;
    }
    for m in audio_mocks {
        m.assert_async().await;
    }

    // Assertion 3 — adaptive controller log forwarded via on_log.
    let msgs = log_messages
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let adaptive_msg_count = msgs
        .iter()
        .filter(|m| m.to_lowercase().contains("adaptive"))
        .count();
    assert!(
        adaptive_msg_count > 0,
        "expected at least one 'Adaptive controller:' log message forwarded via \
         on_log (got 0); this signals AdaptiveController was not wired into DASH \
         segment fetching.\n\nAll captured log messages:\n{}",
        msgs.join("\n")
    );
}
