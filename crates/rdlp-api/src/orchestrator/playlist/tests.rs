//! Tests for playlist helper functions
#![allow(clippy::indexing_slicing)]

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

// ---------------------------------------------------------------------------
// #558 regression: a download must not delete files it did not create
// ---------------------------------------------------------------------------

/// `cleanup_leftover_segments` used to enumerate the output directory before a
/// download and unlink every entry matching `starts_with(title)` +
/// `contains(".part")` + digits after the last `.part`. That matched partial
/// files left by other tools (wget, aria2, a browser) and deleted them on a
/// normal, successful download.
///
/// This drives the real `download_from_info_to_dir` path against a mock server
/// rather than asserting on source text, so it stays valid however a future
/// sweep might be spelled — including the existence-probe shape that
/// `scripts/check-no-dir-sweep-delete.sh` cannot see.
// The `mockito::Server` must stay alive for the whole body — it stops serving
// when dropped — so its drop cannot be tightened. Same rationale as the
// module-level allow in `orchestrator/tests/hls_e2e.rs`.
#[allow(clippy::significant_drop_tightening)]
#[tokio::test]
async fn foreign_part_files_survive_an_episode_download() {
    use crate::events::Event;
    use crate::handle::DownloadId;
    use crate::orchestrator::Orchestrator;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    let mut server = mockito::Server::new_async().await;
    let body = b"video-bytes";
    let _mock = server
        .mock("GET", "/video.mp4")
        .with_status(200)
        .with_header("content-length", &body.len().to_string())
        .with_body(body)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let title = "My Show S01E01";

    // Files another tool could plausibly have left in the user's download
    // directory. Each matched the old sweep's pattern.
    let foreign_part = dir.path().join(format!("{title}.mp4.part1"));
    // The old digit check accepted an EMPTY suffix too: all() on an empty
    // iterator is vacuously true, so a bare `.part` matched as well.
    let foreign_bare = dir.path().join(format!("{title}.mp4.part"));
    let unrelated = dir.path().join("holiday.jpg");
    for p in [&foreign_part, &foreign_bare, &unrelated] {
        tokio::fs::write(p, b"not rdlp's").await.unwrap();
    }

    let fmt = rdlp_types::Format::new(
        "http-1",
        format!("{}/video.mp4", server.url()),
        "mp4",
        rdlp_types::DownloadProtocol::Https,
    );
    let mut info =
        rdlp_types::InfoDict::new("id-558", title, "test", format!("{}/watch", server.url()));
    info.formats = vec![fmt];

    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orch = Orchestrator::new(
        Arc::new(rdlp_types::Config::default()),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );

    let result = orch
        .download_from_info_to_dir(&info, false, dir.path(), &[], None, None)
        .await;

    // Reachability guard. The foreign-file assertions below are deliberately
    // independent of the download's outcome — the sweep ran BEFORE the
    // download, so it destroyed these even when the download later failed. But
    // that independence means all three would pass vacuously if a future change
    // made this call bail BEFORE the former sweep site, so pin how far we got.
    //
    // The download cannot actually succeed here: mockito serves on loopback and
    // `validate_url_security` rejects private hosts for format URLs. That
    // rejection is itself the proof we need — it happens at download dispatch,
    // which is downstream of where the sweep ran (`episode.rs`, right after the
    // output path was computed). An earlier bail-out would surface as a
    // different variant and fail this assertion.
    let err = result.expect_err("loopback format URL must be rejected by the SSRF gate");
    assert!(
        matches!(
            err,
            crate::orchestrator::errors::OrchestratorError::DownloadFailed(_)
        ),
        "expected to reach download dispatch (downstream of the old sweep site); \
         a different error means this test no longer exercises that path and the \
         assertions below are vacuous. Got: {err:?}"
    );

    assert!(
        foreign_part.exists(),
        "a foreign .part1 file must survive — deleting it is #558"
    );
    assert!(
        foreign_bare.exists(),
        "a foreign bare .part file must survive (empty-suffix match)"
    );
    assert!(unrelated.exists(), "an unrelated file must survive");
}
