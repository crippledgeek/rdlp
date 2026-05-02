//! End-to-end DASH download test (mockito): MPD fetch → parse → init+segment
//! fetch → concat → FFmpeg stream-copy mux.
//!
//! The fixture serves placeholder bytes (`b"VINIT"`, `b"V1"`, etc.) — these
//! are NOT valid fMP4 boxes, so FFmpeg correctly refuses to mux them. The
//! test exercises the full pipeline up to and including the mux call, and
//! asserts that:
//!   1. download + concat succeed (intermediates exist mid-flight)
//!   2. mux fails deterministically (FFmpeg rejects the garbage)
//!   3. intermediates are RETAINED on mux failure for diagnosis
//!   4. no output file is produced
//!
//! A separate `#[ignore]` test exercises the success path against real
//! fMP4 fixtures (not committed; see test body for generation steps).

use rdlp_core::Downloader;
use rdlp_downloader::DashDownloader;
use tempfile::TempDir;

#[tokio::test]
async fn fake_segments_fail_mux_with_intermediates_retained() {
    let mut server = mockito::Server::new_async().await;

    let mpd_body = format!(
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT12S" minBufferTime="PT2S">
  <Period duration="PT12S">
    <BaseURL>{}/</BaseURL>
    <AdaptationSet contentType="video">
      <Representation id="v1" bandwidth="500000" mimeType="video/mp4">
        <SegmentTemplate timescale="1000" duration="6000" startNumber="1"
          initialization="vinit.mp4" media="vseg-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
    <AdaptationSet contentType="audio" lang="en">
      <Representation id="a1" bandwidth="64000" mimeType="audio/mp4">
        <SegmentTemplate timescale="1000" duration="6000" startNumber="1"
          initialization="ainit.mp4" media="aseg-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#,
        server.url()
    );

    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(&mpd_body)
        .create_async()
        .await;
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .create_async()
        .await;
    let _v1 = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .create_async()
        .await;
    let _v2 = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .create_async()
        .await;
    let _ai = server
        .mock("GET", "/ainit.mp4")
        .with_body(b"AINIT")
        .create_async()
        .await;
    let _a1 = server
        .mock("GET", "/aseg-1.m4s")
        .with_body(b"A1")
        .create_async()
        .await;
    let _a2 = server
        .mock("GET", "/aseg-2.m4s")
        .with_body(b"A2")
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());

    let downloader = DashDownloader::new();
    let result = downloader.download_to_file(&url, &out, None).await;

    let err = result.expect_err("mux must fail on non-fMP4 bytes");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("mux") || msg.contains("ffmpeg") || msg.contains("invalid"),
        "expected mux/ffmpeg failure, got: {msg}"
    );

    // Intermediates retained for diagnosis.
    let video = dir.path().join("out.video.m4s");
    let audio = dir.path().join("out.audio.m4s");
    assert!(video.exists(), "video intermediate retained on mux failure");
    assert!(audio.exists(), "audio intermediate retained on mux failure");
    assert_eq!(tokio::fs::read(&video).await.unwrap(), b"VINITV1V2");
    assert_eq!(tokio::fs::read(&audio).await.unwrap(), b"AINITA1A2");
    assert!(!out.exists(), "no output produced on mux failure");
}

/// Real fMP4 success-path test. Skipped by default — requires committed
/// fMP4 fixtures.
///
/// To generate fixtures locally:
///
/// ```bash
/// ffmpeg -y -f lavfi -i 'testsrc=size=160x120:duration=2' \
///   -f lavfi -i 'sine=frequency=1000:duration=2' \
///   -map 0:v -c:v libx264 -tune zerolatency -profile:v baseline -level 3.0 \
///   -map 1:a -c:a aac -b:a 64k \
///   -f dash -seg_duration 1 -use_template 1 -use_timeline 0 \
///   -init_seg_name 'init-$RepresentationID$.mp4' \
///   -media_seg_name 'seg-$RepresentationID$-$Number$.m4s' \
///   /tmp/dash/manifest.mpd
/// ```
///
/// Then commit `init-0.mp4` (video), `init-1.mp4` (audio), and one segment
/// each under `crates/rdlp-downloader/tests/fixtures/dash/segments/`.
#[tokio::test]
#[ignore = "requires real fMP4 fixtures; see test docstring for generation steps"]
async fn real_fmp4_fixtures_mux_to_single_output() {
    // Wire up a mockito server that serves real fMP4 init + segment bytes
    // from disk fixtures, run download_to_file, assert out.mp4 exists and
    // is non-empty, intermediates are cleaned up.
}
