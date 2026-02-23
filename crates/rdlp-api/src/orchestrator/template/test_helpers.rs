//! Shared test fixtures for template tests

use super::*;

pub(super) fn test_ctx() -> RenderContext {
    RenderContext {
        epoch: 1700000000,
        autonumber: 1,
    }
}

pub(super) fn test_info() -> rdlp_core::InfoDict {
    let mut info = rdlp_core::InfoDict::new(
        "abc123",
        "My Video Title",
        "TestExtractor",
        "https://example.com/video",
    );
    info.uploader = Some("TestUploader".to_string());
    info.playlist_index = Some(5);
    info.view_count = Some(12345);
    info.upload_date = Some("20231114".to_string());
    info.duration = Some(3661.5);
    info.tags = Some(vec![
        "tag1".to_string(),
        "tag2".to_string(),
        "tag3".to_string(),
    ]);
    info
}

pub(super) fn test_format() -> rdlp_core::Format {
    let mut f = rdlp_core::Format::new(
        "720p",
        "https://example.com/video.mp4",
        "mp4",
        rdlp_core::DownloadProtocol::Https,
    );
    f.height = Some(720);
    f.width = Some(1280);
    f.filesize = Some(590_000_000);
    f
}
