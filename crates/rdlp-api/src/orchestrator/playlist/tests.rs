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
// The `mockito::ServerGuard` must stay alive for the whole body — dropping
// it recycles the underlying `Server` back to mockito's server pool, and
// that recycle step calls `reset()`, clearing every mock it had registered.
// Rather than allowing clippy's early-drop suggestion, `server` is dropped
// explicitly as the LAST statement below, after every assertion — same
// convention as `orchestrator/tests/hls_e2e.rs`.
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
    drop(server);
}

// ---------------------------------------------------------------------------
// #572 review finding 1: the playlist-episode path must claim the output
// path exactly like the Single-video state-machine path does.
// ---------------------------------------------------------------------------

/// A second rdlp process (separate `TempRegistry` instance, per
/// `state::tests::part_lock_tests`'s convention) already holding the claim
/// on an episode's `.rdlp-part` path must make `download_from_info_to_dir`
/// fail with `OutputBusy` — not silently detect-resume and attempt the
/// download anyway.
///
/// Uses a mockito loopback URL (same fixture shape as
/// `foreign_part_files_survive_an_episode_download` above) so that even the
/// UNPATCHED code fails FAST: without the fix, `resolve_resume` runs
/// unguarded and the function proceeds to the real download attempt, which
/// the SSRF gate rejects synchronously (`DownloadFailed`) rather than
/// hanging on a real network call — deterministic either way.
///
/// RED against the unpatched `episode.rs` (no `PartLock::claim` at all):
/// panics with "got: Err(DownloadFailed(..))" instead of `OutputBusy`.
// The `mockito::Server` must stay alive for the whole body — same rationale
// as `foreign_part_files_survive_an_episode_download` above; `server` is
// dropped explicitly as the LAST statement below instead of allowing
// clippy's early-drop suggestion.
#[tokio::test]
async fn playlist_episode_collision_reports_output_busy() {
    use crate::events::Event;
    use crate::handle::DownloadId;
    use crate::orchestrator::Orchestrator;
    use crate::orchestrator::part_lock::PartLock;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/video.mp4")
        .with_status(200)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let title = "My Show S01E01";

    let fmt = rdlp_types::Format::new(
        "http-1",
        format!("{}/video.mp4", server.url()),
        "mp4",
        rdlp_types::DownloadProtocol::Https,
    );
    let mut info =
        rdlp_types::InfoDict::new("id-572", title, "test", format!("{}/watch", server.url()));
    info.formats = vec![fmt.clone()];

    let (tx, _rx) = mpsc::channel::<Event>(64);
    let orch = Orchestrator::new(
        Arc::new(rdlp_types::Config::default()),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );

    // Simulate another process already downloading this exact episode:
    // reconstruct the SAME `.rdlp-part` path episode.rs will compute, and
    // claim it via a SEPARATE registry instance before the real call.
    let file_ext = orch.determine_file_extension(&fmt);
    let sanitized_title = Orchestrator::sanitize_filename(title);
    let clean_path = dir.path().join(format!("{sanitized_title}.{file_ext}"));
    let part = crate::orchestrator::naming::part_path(&clean_path);
    let other_process_registry = Arc::new(rdlp_postprocess::TempRegistry::new());
    let _held_elsewhere = PartLock::claim(other_process_registry, part.clone())
        .expect("simulated other-process claim must succeed");

    let result = orch
        .download_from_info_to_dir(&info, false, dir.path(), &[], None, None)
        .await;

    assert!(
        matches!(
            result,
            Err(crate::orchestrator::errors::OrchestratorError::OutputBusy { ref path }) if *path == part
        ),
        "playlist episode download must be refused as OutputBusy when another \
         process already claims its .rdlp-part path, got: {result:?}"
    );
    drop(server);
}

// ---------------------------------------------------------------------------
// `download_episodes` / `run_retry_waves` (#768): `Option<&T>` at the
// boundary, and the archive keyed on `archive_token_for`, never on the
// display name.
// ---------------------------------------------------------------------------

/// An episode as a plugin produces it: the manifest `display_name` in
/// `extractor`, the manifest `name` in `extractor_key`, one format on a
/// loopback URL the SSRF gate rejects at dispatch — so an episode that is
/// NOT skipped fails deterministically without any network, and one that
/// is skipped never reaches dispatch at all.
fn plugin_episode() -> rdlp_types::InfoDict {
    let mut info = rdlp_types::InfoDict::new(
        "ep-1",
        "Episode One",
        "XHamster Display",
        "http://127.0.0.1:1/watch/ep-1",
    );
    info.extractor_key = Some("xhamster".to_string());
    info.formats = vec![rdlp_types::Format::new(
        "http-1",
        "http://127.0.0.1:1/ep-1.mp4",
        "mp4",
        rdlp_types::DownloadProtocol::Https,
    )];
    info
}

fn bare_orchestrator() -> crate::orchestrator::Orchestrator {
    use crate::handle::DownloadId;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    let (tx, _rx) = mpsc::channel::<crate::events::Event>(64);
    crate::orchestrator::Orchestrator::new(
        Arc::new(rdlp_types::Config::default()),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    )
}

/// The archive is consulted through `archive_token_for`: a line under the
/// plugin's `name` skips the episode; a line under its `display_name`
/// (what `extractor` carries) does not, and neither does no archive.
#[tokio::test(start_paused = true)]
async fn download_episodes_skips_only_an_episode_recorded_under_its_archive_token() {
    use tempfile::TempDir;

    let orch = bare_orchestrator();
    let dir = TempDir::new().unwrap();
    let infos = vec![plugin_episode()];
    let existing = HashMap::new();

    let under_key: HashSet<String> = [archive::archive_key("xhamster", "ep-1")].into();
    let under_display: HashSet<String> = [archive::archive_key("XHamster Display", "ep-1")].into();
    let mut outcomes = Vec::new();
    for archive in [Some(&under_key), Some(&under_display), None] {
        let (downloaded, failed, interrupted) = orch
            .download_episodes(
                &infos,
                &existing,
                archive,
                dir.path(),
                &[],
                None,
                Instant::now(),
                infos.len(),
            )
            .await;
        assert!(downloaded.is_empty() && !interrupted);
        outcomes.push(failed);
    }
    let [skipped, under_display_name, no_archive] = outcomes.as_slice() else {
        panic!("three runs");
    };
    assert!(skipped.is_empty(), "recorded under its token: skipped");
    assert_eq!(
        under_display_name.len(),
        1,
        "a line under the display name is not this episode's token"
    );
    assert_eq!(no_archive.len(), 1, "no archive: the episode is attempted");
    assert_eq!(no_archive.first().map(|f| f.0), Some(1));
}

/// Every wave retries each still-failed episode under the audio filter it
/// is handed; an episode that keeps failing stays in `failed` and counts
/// nothing recovered. Paused time makes the three wave delays free.
#[tokio::test(start_paused = true)]
async fn run_retry_waves_retries_each_failed_episode_and_keeps_the_survivors() {
    use tempfile::TempDir;

    let orch = bare_orchestrator();
    let dir = TempDir::new().unwrap();
    let infos = vec![plugin_episode()];
    let mut failed = vec![(1, "Episode One".to_string(), "first pass".to_string())];
    let mut downloaded = Vec::new();
    let mut interrupted = false;

    let recovered = orch
        .run_retry_waves(
            &mut failed,
            &mut downloaded,
            &mut interrupted,
            &infos,
            dir.path(),
            &[],
            Some("dub"),
            infos.len(),
        )
        .await;

    assert_eq!(recovered, 0);
    assert!(downloaded.is_empty());
    assert!(!interrupted);
    assert_eq!(failed.len(), 1, "still failed after every wave");
    assert_eq!(failed.first().map(|f| f.0), Some(1));
    assert_ne!(
        failed.first().map(|f| f.2.as_str()),
        Some("first pass"),
        "the recorded error is the last wave's, not the first pass's"
    );
}
