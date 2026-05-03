//! Tests for HlsDownloader::download_format pre-resolved + legacy paths.

use rdlp_core::Downloader as _;
use rdlp_downloader::HlsDownloader;
use rdlp_types::{DownloadProtocol, Format};
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
