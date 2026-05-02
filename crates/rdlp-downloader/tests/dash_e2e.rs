//! End-to-end DASH download test (mockito): MPD fetch → parse → init+segment
//! fetch → concat to `<output>.video.m4s` + `<output>.audio.m4s`.
//!
//! Mux into a single container is Task 5; this test asserts only that the
//! intermediates are produced and that the muxed output does NOT yet exist.

use rdlp_core::Downloader;
use rdlp_downloader::DashDownloader;
use tempfile::TempDir;

#[tokio::test]
async fn downloads_segment_template_video_and_audio() {
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
    let stats = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect("download succeeds");

    let video = dir.path().join("out.video.m4s");
    let audio = dir.path().join("out.audio.m4s");
    assert!(video.exists(), "video file written");
    assert!(audio.exists(), "audio file written");
    assert_eq!(tokio::fs::read(&video).await.unwrap(), b"VINITV1V2");
    assert_eq!(tokio::fs::read(&audio).await.unwrap(), b"AINITA1A2");
    // Mux is Task 5 — out.mp4 itself does NOT exist yet.
    assert!(
        !out.exists(),
        "muxed output is Task 5; should not exist after Task 4"
    );
    assert!(stats.bytes_downloaded > 0);
}
