//! Tests for `DashDownloader`'s pre-resolved fragments branch.
//!
//! When `Format.fragments` is `Some(...)`, `download_format` must fetch the
//! listed segment URLs directly without re-fetching or re-parsing the MPD.

use rdlp_core::Downloader as _;
use rdlp_downloader::DashDownloader;
use rdlp_types::{DownloadProtocol, Format, Fragment};
use tempfile::TempDir;

#[tokio::test]
async fn pre_resolved_fragments_skip_mpd_fetch() {
    let mut server = mockito::Server::new_async().await;

    // The MPD endpoint MUST NOT be fetched.
    let mpd_mock = server
        .mock("GET", "/manifest.mpd")
        .with_body("<MPD></MPD>")
        .expect(0)
        .create_async()
        .await;

    // Init segment + 3 media segments.
    let _init_mock = server
        .mock("GET", "/init.m4s")
        .with_status(200)
        .with_body(b"INIT")
        .expect(1)
        .create_async()
        .await;
    let _seg1_mock = server
        .mock("GET", "/seg-1.m4s")
        .with_status(200)
        .with_body(b"SEG1")
        .expect(1)
        .create_async()
        .await;
    let _seg2_mock = server
        .mock("GET", "/seg-2.m4s")
        .with_status(200)
        .with_body(b"SEG2")
        .expect(1)
        .create_async()
        .await;
    let _seg3_mock = server
        .mock("GET", "/seg-3.m4s")
        .with_status(200)
        .with_body(b"SEG3")
        .expect(1)
        .create_async()
        .await;

    let base = server.url();
    let mut format = Format::new(
        "v720p",
        format!("{base}/manifest.mpd"),
        "mp4",
        DownloadProtocol::HttpDashSegments,
    );
    format.fragments = Some(vec![
        Fragment {
            url: format!("{base}/init.m4s"),
            duration: None,
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-1.m4s"),
            duration: Some(2.0),
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-2.m4s"),
            duration: Some(2.0),
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-3.m4s"),
            duration: Some(2.0),
            filesize: None,
        },
    ]);

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.m4s");

    let downloader = DashDownloader::new();
    let stats = downloader
        .download_format(&format, &output, None)
        .await
        .expect("download should succeed");

    // Output must be init + seg1 + seg2 + seg3 concatenated.
    let body = tokio::fs::read(&output).await.unwrap();
    assert_eq!(body, b"INITSEG1SEG2SEG3");

    // Stats must reflect total bytes.
    assert_eq!(stats.bytes_downloaded, 16);

    // MPD must never have been fetched.
    mpd_mock.assert_async().await;
}
