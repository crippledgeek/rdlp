//! Tests for HlsDownloader::download_format pre-resolved + legacy paths.

use rdlp_core::Downloader as _;
use rdlp_downloader::HlsDownloader;
use rdlp_types::{DownloadProtocol, Format, Fragment};
use tempfile::TempDir;

#[tokio::test]
async fn fragments_none_falls_through_to_legacy_download_to_file() {
    let mut server = mockito::Server::new_async().await;

    // Legacy path: media playlist IS fetched at download time.
    let _media = server
        .mock("GET", "/v.m3u8")
        .with_body(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
             #EXTINF:6.0,\nseg-1.ts\n#EXT-X-ENDLIST\n",
        )
        .expect_at_least(1)
        .create_async()
        .await;

    let _seg1 = server
        .mock("GET", "/seg-1.ts")
        .with_status(200)
        .with_body(b"SEG1")
        .expect_at_least(1)
        .create_async()
        .await;

    let url = format!("{}/v.m3u8", server.url());
    let format = Format::new("hls", &url, "m3u8", DownloadProtocol::M3u8);
    // fragments stays None.

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.ts");

    let dl = HlsDownloader::new();
    let _ = dl
        .download_format(&format, &output, None)
        .await
        .expect("legacy path must work");

    assert!(output.exists());
}

#[tokio::test]
async fn pre_resolved_fragments_skip_playlist_fetch() {
    let mut server = mockito::Server::new_async().await;

    // Master + variant playlists MUST NOT be fetched at download time.
    let master_mock = server
        .mock("GET", "/master.m3u8")
        .with_body("ignored")
        .expect(0)
        .create_async()
        .await;
    let variant_mock = server
        .mock("GET", "/v.m3u8")
        .with_body("ignored")
        .expect(0)
        .create_async()
        .await;

    let _init = server
        .mock("GET", "/init.m4s")
        .with_status(200)
        .with_body(b"INIT")
        .expect(1)
        .create_async()
        .await;
    let _s1 = server
        .mock("GET", "/seg-1.m4s")
        .with_status(200)
        .with_body(b"SEG1")
        .expect(1)
        .create_async()
        .await;
    let _s2 = server
        .mock("GET", "/seg-2.m4s")
        .with_status(200)
        .with_body(b"SEG2")
        .expect(1)
        .create_async()
        .await;
    let _s3 = server
        .mock("GET", "/seg-3.m4s")
        .with_status(200)
        .with_body(b"SEG3")
        .expect(1)
        .create_async()
        .await;

    let base = server.url();
    let mut format = Format::new(
        "hls",
        format!("{base}/master.m3u8"),
        "m3u8",
        DownloadProtocol::M3u8,
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

    let stats = HlsDownloader::new()
        .download_format(&format, &output, None)
        .await
        .expect("pre-resolved download must succeed");

    let body = tokio::fs::read(&output).await.unwrap();
    assert_eq!(body, b"INITSEG1SEG2SEG3");
    assert_eq!(stats.bytes_downloaded, 16);

    master_mock.assert_async().await;
    variant_mock.assert_async().await;
}

#[tokio::test]
async fn fragment_404_propagates_as_error() {
    let mut server = mockito::Server::new_async().await;

    let _s1 = server
        .mock("GET", "/seg-1.ts")
        .with_status(200)
        .with_body(b"OK")
        .create_async()
        .await;
    let _s2 = server
        .mock("GET", "/seg-2.ts")
        .with_status(404)
        .with_body(b"not found")
        .create_async()
        .await;

    let base = server.url();
    let mut format = Format::new(
        "hls",
        format!("{base}/v.m3u8"),
        "m3u8",
        DownloadProtocol::M3u8,
    );
    format.fragments = Some(vec![
        Fragment {
            url: format!("{base}/seg-1.ts"),
            duration: Some(2.0),
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-2.ts"),
            duration: Some(2.0),
            filesize: None,
        },
    ]);

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.ts");

    let result = HlsDownloader::new()
        .download_format(&format, &output, None)
        .await;
    assert!(result.is_err(), "404 must propagate");
}

#[tokio::test]
async fn empty_fragments_vec_writes_empty_file() {
    let mut format = Format::new(
        "hls",
        "https://h.com/v.m3u8",
        "m3u8",
        DownloadProtocol::M3u8,
    );
    format.fragments = Some(vec![]);

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.ts");

    let stats = HlsDownloader::new()
        .download_format(&format, &output, None)
        .await
        .expect("empty vec writes empty file");
    // Documented no-op: the shared helper writes zero bytes when fragments
    // is empty. The expand-side guard (HlsExpandError::NoSegments) prevents
    // this case from reaching the downloader in practice.
    assert_eq!(stats.bytes_downloaded, 0);
    assert!(output.exists());
    assert_eq!(tokio::fs::read(&output).await.unwrap().len(), 0);
}
