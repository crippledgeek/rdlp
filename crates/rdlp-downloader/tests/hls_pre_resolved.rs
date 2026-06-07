//! Tests for HlsDownloader::download_format pre-resolved path + programmer-error guard.

use rdlp_core::{Downloader as _, RdlpError};
use rdlp_downloader::HlsDownloader;
use rdlp_types::{DownloadProtocol, Format, Fragment};
use tempfile::TempDir;

/// After #267, every HLS Format reaching HlsDownloader MUST carry pre-resolved
/// fragments. A Format with fragments=None is a programmer error (extractor did
/// not call expand_hls_in_place) and MUST return a typed RdlpError::Download —
/// it MUST NOT silently fetch the playlist or succeed.
#[tokio::test]
async fn fragments_none_returns_typed_download_error() {
    let format = Format::new(
        "hls",
        "https://example.com/v.m3u8",
        "m3u8",
        DownloadProtocol::M3u8,
    );
    // fragments stays None — this is the programmer-error case.

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.ts");

    let result = HlsDownloader::new()
        .download_format(&format, &output, None, None)
        .await;

    assert!(
        result.is_err(),
        "fragments=None must return an error, not succeed"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, RdlpError::Download { .. }),
        "error must be RdlpError::Download, got: {err:?}"
    );
    let RdlpError::Download { message, url } = err else {
        unreachable!()
    };
    assert!(
        message.contains("expand_hls_in_place"),
        "error message must mention expand_hls_in_place; got: {message}"
    );
    assert_eq!(
        url.as_ref().map(rdlp_redact::RedactedUrlBuf::expose),
        Some("https://example.com/v.m3u8")
    );
    // Output file must NOT have been created — no playlist was fetched.
    assert!(!output.exists());
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
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: None,
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-1.m4s"),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(2.0),
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-2.m4s"),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(2.0),
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-3.m4s"),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(2.0),
            filesize: None,
        },
    ]);

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.m4s");

    let stats = HlsDownloader::new()
        .download_format(&format, &output, None, None)
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
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(2.0),
            filesize: None,
        },
        Fragment {
            url: format!("{base}/seg-2.ts"),
            byte_range: None,
            init_url: None,
            init_byte_range: None,
            duration: Some(2.0),
            filesize: None,
        },
    ]);

    let tmp = TempDir::new().unwrap();
    let output = tmp.path().join("video.ts");

    let result = HlsDownloader::new()
        .download_format(&format, &output, None, None)
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
        .download_format(&format, &output, None, None)
        .await
        .expect("empty vec writes empty file");
    // Documented no-op: the shared helper writes zero bytes when fragments
    // is empty. The expand-side guard (HlsExpandError::NoSegments) prevents
    // this case from reaching the downloader in practice.
    assert_eq!(stats.bytes_downloaded, 0);
    assert!(output.exists());
    assert_eq!(tokio::fs::read(&output).await.unwrap().len(), 0);
}
