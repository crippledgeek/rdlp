//! Tests for playlist helper functions

use super::*;

#[test]
fn test_detect_audio_types_empty() {
    let infos: Vec<rdlp_types::InfoDict> = Vec::new();
    assert!(detect_audio_types(&infos).is_empty());
}

#[test]
fn test_detect_audio_types_single() {
    let mut info = rdlp_types::InfoDict::new("id", "title", "test", "http://example.com");
    let mut fmt = rdlp_types::Format::new(
        "f1",
        "http://example.com/v.m3u8",
        "mp4",
        rdlp_types::DownloadProtocol::M3u8,
    );
    fmt.language = Some("SUB".to_string());
    info.formats = vec![fmt];
    assert_eq!(detect_audio_types(&[info]), vec!["SUB"]);
}

#[test]
fn test_detect_audio_types_multiple() {
    let mut info = rdlp_types::InfoDict::new("id", "title", "test", "http://example.com");
    let mut sub = rdlp_types::Format::new(
        "f1",
        "http://example.com/v.m3u8",
        "mp4",
        rdlp_types::DownloadProtocol::M3u8,
    );
    sub.language = Some("SUB".to_string());
    let mut dub = rdlp_types::Format::new(
        "f2",
        "http://example.com/v2.m3u8",
        "mp4",
        rdlp_types::DownloadProtocol::M3u8,
    );
    dub.language = Some("DUB".to_string());
    info.formats = vec![dub, sub];
    // Should be sorted alphabetically
    assert_eq!(detect_audio_types(&[info]), vec!["DUB", "SUB"]);
}

#[test]
fn test_detect_audio_types_no_language() {
    let mut info = rdlp_types::InfoDict::new("id", "title", "test", "http://example.com");
    let fmt = rdlp_types::Format::new(
        "f1",
        "http://example.com/v.m3u8",
        "mp4",
        rdlp_types::DownloadProtocol::M3u8,
    );
    info.formats = vec![fmt];
    assert!(detect_audio_types(&[info]).is_empty());
}

#[test]
fn test_filter_formats_by_language() {
    let mut info = rdlp_types::InfoDict::new("id", "title", "test", "http://example.com");
    let mut sub = rdlp_types::Format::new(
        "f1",
        "http://example.com/v.m3u8",
        "mp4",
        rdlp_types::DownloadProtocol::M3u8,
    );
    sub.language = Some("SUB".to_string());
    let mut dub = rdlp_types::Format::new(
        "f2",
        "http://example.com/v2.m3u8",
        "mp4",
        rdlp_types::DownloadProtocol::M3u8,
    );
    dub.language = Some("DUB".to_string());
    info.formats = vec![sub, dub];

    let mut infos = vec![info];
    filter_formats_by_language(&mut infos, "SUB");

    assert_eq!(infos[0].formats.len(), 1);
    assert_eq!(infos[0].formats[0].language.as_deref(), Some("SUB"));
}

#[test]
fn test_extract_cdn_host() {
    assert_eq!(
        extract_cdn_host("https://s2.netmagcdn.com/hls/master.m3u8"),
        Some("s2.netmagcdn.com")
    );
    assert_eq!(
        extract_cdn_host("http://cdn.example.com/video.mp4"),
        Some("cdn.example.com")
    );
    assert_eq!(extract_cdn_host("not-a-url"), None);
}

#[test]
fn test_extract_cdn_host_stickiness_sort() {
    let primary = "https://s2.netmagcdn.com/hls/ep1/master.m3u8";
    let primary_host = extract_cdn_host(primary);

    let mut urls = [
        "https://s1.netmagcdn.com/hls/ep1/alt.m3u8".to_string(),
        "https://s2.netmagcdn.com/hls/ep1/alt2.m3u8".to_string(),
        "https://s3.netmagcdn.com/hls/ep1/alt3.m3u8".to_string(),
    ];

    urls.sort_by_key(|url| u8::from(extract_cdn_host(url) != primary_host));

    // s2 (same host) should sort first
    assert!(urls[0].contains("s2.netmagcdn.com"));
}
