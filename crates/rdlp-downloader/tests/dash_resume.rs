//! DASH resume test (mockito): partial first run leaves state + per-segment
//! parts on disk; second run skips already-fetched segments.

use std::time::Duration;

use mockito::Server;
use rdlp_core::{Downloader, RetryConfig};
use rdlp_downloader::DashDownloader;
use tempfile::TempDir;

/// Tight retry policy so tests don't burn 60s per 503.
fn fast_retry() -> RetryConfig {
    RetryConfig::new(2, Duration::from_millis(10), Duration::from_millis(50), 2.0)
        .with_jitter(false)
}

fn fast_dl() -> DashDownloader {
    DashDownloader::new().with_retry_config(fast_retry())
}

fn mpd_body(server_url: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT12S" minBufferTime="PT2S">
  <Period duration="PT12S">
    <BaseURL>{server_url}/</BaseURL>
    <AdaptationSet contentType="video">
      <Representation id="v1" bandwidth="500000" mimeType="video/mp4">
        <SegmentTemplate timescale="1000" duration="6000" startNumber="1"
          initialization="vinit.mp4" media="vseg-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#
    )
}

#[tokio::test]
async fn resume_skips_already_downloaded_segments() {
    // Phase 1: succeed init + seg 1, fail seg 2 with 503. State should
    // record seg 0 (0-based) as done.
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_first = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    // First run: must error because seg 2 fails (503 burns through retries).
    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 → fail");

    // State file should exist; init + seg-0 part on disk.
    let state_file = dir.path().join("out.mp4.dash_state.json");
    assert!(state_file.exists(), "state persisted");
    let parts = dir.path().join("out.video.parts");
    let init_part = parts.join("init.m4s");
    let part0 = parts.join("0000.m4s");
    assert!(init_part.exists(), "init part written");
    assert!(part0.exists(), "seg 0 part written");

    // State JSON should record seg 0 as done.
    let body = tokio::fs::read_to_string(&state_file).await.unwrap();
    assert!(
        body.contains("\"v1\":[0]") || body.contains("\"v1\": [0]"),
        "state should record seg 0 done; got: {body}"
    );

    // Phase 2: drop the failing mock, replace with success. Wire a NEW
    // vseg-1 mock with expect(0) — it MUST NOT be hit again.
    drop(v2_fail);
    drop(v1_first);

    let v1_replay = server
        .mock("GET", "/vseg-1.m4s")
        .expect(0)
        .with_body(b"V1")
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    // Second run: outcome may be Ok or Err depending on whether the
    // fake-byte mux works (it doesn't, FFmpeg rejects). We only care that
    // vseg-1 was NOT re-fetched.
    let _ = downloader.download_to_file(&url, &out, None).await;

    v1_replay.assert_async().await;
}

/// State file path is `<output>.dash_state.json` — i.e. the FULL output
/// filename plus suffix, not the stem.
#[tokio::test]
async fn state_path_uses_full_output_filename() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .create_async()
        .await;
    // Init succeeds, then segment 1 fails so we get partial state written.
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .create_async()
        .await;
    let _v1 = server
        .mock("GET", "/vseg-1.m4s")
        .with_status(503)
        .create_async()
        .await;
    let _v2 = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());

    let _err = fast_dl()
        .download_to_file(&url, &out, None)
        .await
        .expect_err("segs 503 must fail");

    // The correct path includes the full output filename.
    assert!(
        dir.path().join("out.mp4.dash_state.json").exists(),
        "state path must be <output>.dash_state.json"
    );
    // The wrong (stem-only) path must NOT exist.
    assert!(
        !dir.path().join("out.dash_state.json").exists(),
        "state path must include the .mp4 suffix, not just the stem"
    );
}
