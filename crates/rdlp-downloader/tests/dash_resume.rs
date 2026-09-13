//! DASH resume test (mockito): partial first run leaves state + per-segment
//! parts on disk; second run skips already-fetched segments.

use std::time::Duration;

use mockito::Server;
use rdlp_core::{Downloader, RetryConfig};
use rdlp_downloader::DashDownloader;
use tempfile::TempDir;

/// Tight retry policy so tests don't burn 60s per 503.
fn fast_retry() -> RetryConfig {
    RetryConfig::new(2, Duration::from_millis(10), Duration::from_millis(50), 2.0)
        .with_jitter(false)
}

fn fast_dl() -> DashDownloader {
    DashDownloader::new().with_retry_config(fast_retry())
}

/// `duration_ts` is the `SegmentTemplate@duration` value (in timescale units,
/// timescale=1000) — parameterised so `resume_starts_fresh_when_segment_timeline_changed_under_same_paths`
/// can serve a variant MPD whose segment names are unchanged but whose
/// manifest fingerprint differs, without pasting a second MPD string.
fn mpd_body_with_duration(server_url: &str, duration_ts: u32) -> String {
    format!(
        r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static"
     mediaPresentationDuration="PT12S" minBufferTime="PT2S">
  <Period duration="PT12S">
    <BaseURL>{server_url}/</BaseURL>
    <AdaptationSet contentType="video">
      <Representation id="v1" bandwidth="500000" mimeType="video/mp4">
        <SegmentTemplate timescale="1000" duration="{duration_ts}" startNumber="1"
          initialization="vinit.mp4" media="vseg-$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#
    )
}

/// The default fixture's segment duration (6000 ts units at timescale 1000
/// => 6s per segment; matches `mediaPresentationDuration="PT12S"`, 2
/// segments).
const DEFAULT_DURATION_TS: u32 = 6000;

fn mpd_body(server_url: &str) -> String {
    mpd_body_with_duration(server_url, DEFAULT_DURATION_TS)
}

#[tokio::test]
async fn resume_skips_already_downloaded_segments() {
    // Phase 1: succeed init + seg 1, fail seg 2 with 503. State should
    // record seg 0 (0-based) as done.
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_first = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    // First run: must error because seg 2 fails (503 burns through retries).
    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 → fail");

    // State file should exist; init + seg-0 part on disk.
    let state_file = dir.path().join("out.mp4.dash_state.json");
    assert!(state_file.exists(), "state persisted");
    let parts = dir.path().join("out.video.parts");
    let init_part = parts.join("init.m4s");
    let part0 = parts.join("0000.m4s");
    assert!(init_part.exists(), "init part written");
    assert!(part0.exists(), "seg 0 part written");

    // State JSON should record seg 0 as done, with its byte length (v2
    // schema — #677).
    let body = tokio::fs::read_to_string(&state_file).await.unwrap();
    assert!(
        body.contains("\"0\":2"),
        "state should record seg 0 done with its length (2 bytes, \"V1\"); got: {body}"
    );

    // Phase 2: drop the failing mock, replace with success. Wire a NEW
    // vseg-1 mock with expect(0) — it MUST NOT be hit again.
    drop(v2_fail);
    drop(v1_first);

    let v1_replay = server
        .mock("GET", "/vseg-1.m4s")
        .expect(0)
        .with_body(b"V1")
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    // Second run: outcome may be Ok or Err depending on whether the
    // fake-byte mux works (it doesn't, FFmpeg rejects). We only care that
    // vseg-1 was NOT re-fetched.
    let _ = downloader.download_to_file(&url, &out, None).await;

    v1_replay.assert_async().await;
}

/// Regression guard for #672: `DownloadStats.retries` on the DASH legacy
/// MPD-URL path was hardcoded to `0` even when a segment fetch retried. One
/// 503-then-200 on the single video segment must surface as `retries == 1`.
#[tokio::test]
async fn retry_count_is_reported_in_stats() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    // mockito matches mocks in CREATION order: the 503 is tried first, then
    // falls through to the 200 once exhausted (mockito re-checks in order
    // each request, so an `expect(1)` 503 is only matched on the FIRST call).
    let v1_fail = server
        .mock("GET", "/vseg-1.m4s")
        .with_status(503)
        .expect(1)
        .create_async()
        .await;
    let v1_ok = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect(1)
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());

    let stats = fast_dl()
        .download_to_file(&url, &out, None)
        .await
        .expect("503-then-200 must succeed via retry (video-only, no mux needed)");

    assert_eq!(
        stats.retries, 1,
        "one retried segment fetch must be reflected in DownloadStats.retries"
    );
    v1_fail.assert_async().await;
    v1_ok.assert_async().await;
}

/// #677: a segment part that is durably recorded as done but is short on
/// disk (unfsynced data lost after the sidecar had recorded it, or any other
/// out-of-band truncation) must be re-fetched, not trusted on non-emptiness.
#[tokio::test]
async fn truncated_segment_part_is_refetched_on_resume() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_first = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 → fail");

    let parts = dir.path().join("out.video.parts");
    let part0 = parts.join("0000.m4s");
    assert!(part0.exists(), "seg 0 part written");

    // Simulate lost unfsynced data: the sidecar durably recorded 2 bytes,
    // but the part file on disk is a 1-byte remnant. Non-zero, so the old
    // "on_disk_len > 0" check would have wrongly accepted it.
    tokio::fs::write(&part0, b"V").await.unwrap();

    drop(v2_fail);
    drop(v1_first);

    let v1_replay = server
        .mock("GET", "/vseg-1.m4s")
        .expect(1)
        .with_body(b"V1")
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    // Video-only representation: the download completes by renaming the
    // concatenated intermediate straight to `out`, no FFmpeg mux involved,
    // so a successful run's output is directly inspectable.
    downloader
        .download_to_file(&url, &out, None)
        .await
        .expect("second run must succeed once all segments are fetchable");

    v1_replay.assert_async().await;
    let output = tokio::fs::read(&out).await.unwrap();
    assert_eq!(
        output, b"VINITV1V2",
        "truncated segment must be replaced by a full re-fetch, not left corrupt in the output"
    );
}

/// #677 sibling: the same truncation defect on the init segment.
#[tokio::test]
async fn truncated_init_part_is_refetched_on_resume() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let vi_first = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let _v1_ok1 = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 → fail");

    let parts = dir.path().join("out.video.parts");
    let init_part = parts.join("init.m4s");
    assert!(init_part.exists(), "init part written");

    // "VINIT" is 5 bytes; truncate to a non-empty 1-byte remnant.
    tokio::fs::write(&init_part, b"V").await.unwrap();

    drop(v2_fail);
    drop(vi_first);

    let vi_replay = server
        .mock("GET", "/vinit.mp4")
        .expect(1)
        .with_body(b"VINIT")
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    downloader
        .download_to_file(&url, &out, None)
        .await
        .expect("second run must succeed once all segments are fetchable");

    vi_replay.assert_async().await;
    let output = tokio::fs::read(&out).await.unwrap();
    assert_eq!(
        output, b"VINITV1V2",
        "truncated init segment must be replaced by a full re-fetch, not left corrupt in the output"
    );
}

/// State file path is `<output>.dash_state.json` — i.e. the FULL output
/// filename plus suffix, not the stem.
#[tokio::test]
async fn state_path_uses_full_output_filename() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .create_async()
        .await;
    // Init succeeds, then segment 1 fails so we get partial state written.
    let _vi = server
        .mock("GET", "/vinit.mp4")
        .with_body(b"VINIT")
        .create_async()
        .await;
    let _v1 = server
        .mock("GET", "/vseg-1.m4s")
        .with_status(503)
        .create_async()
        .await;
    let _v2 = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());

    let _err = fast_dl()
        .download_to_file(&url, &out, None)
        .await
        .expect_err("segs 503 must fail");

    // The correct path includes the full output filename.
    assert!(
        dir.path().join("out.mp4.dash_state.json").exists(),
        "state path must be <output>.dash_state.json"
    );
    // The wrong (stem-only) path must NOT exist.
    assert!(
        !dir.path().join("out.dash_state.json").exists(),
        "state path must include the .mp4 suffix, not just the stem"
    );
}

/// #746: a resumed sidecar's anchor validator (the video init segment's
/// strong validator, RFC 9110 §8.8) is revalidated with `If-Range` against
/// the CURRENT init URL before the sidecar is trusted. A different `ETag`
/// means the origin now serves different content under the same segment
/// names — the download must start fresh, even though every part from the
/// first run is still on disk.
#[tokio::test]
async fn resume_starts_fresh_when_init_segment_etag_changed() {
    // Phase 1: init served with etag "i1", seg 1 ok, seg 2 503 => error;
    // state + parts on disk, with the init's etag recorded as the anchor.
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let vi_first = server
        .mock("GET", "/vinit.mp4")
        .with_header("etag", "\"i1\"")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_first = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 => fail");

    let parts = dir.path().join("out.video.parts");
    let init_part = parts.join("init.m4s");
    assert!(init_part.exists(), "init part written in phase 1");

    // Phase 2: same MPD, but the origin now answers with a DIFFERENT etag —
    // the representation changed under the same segment names. vseg-1 must
    // be re-fetched even though it was already recorded + on disk, and the
    // init part on disk (still present — nothing deletes it) is re-fetched
    // too, not trusted.
    drop(v2_fail);
    drop(v1_first);
    drop(vi_first);

    let vi_replay = server
        .mock("GET", "/vinit.mp4")
        .with_header("etag", "\"i2\"")
        .with_body(b"VINIT2")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_replay = server
        .mock("GET", "/vseg-1.m4s")
        .expect_at_least(1)
        .with_body(b"V1")
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    // Phase-2 outcome may be `Ok` or `Err` depending on unrelated fixture
    // details; only the mock hit counts below are asserted. (A successful
    // run cleans up `parts_dir` entirely — that cleanup is unrelated to the
    // mismatch and is not what this test pins.)
    let _ = downloader.download_to_file(&url, &out, None).await;

    vi_replay.assert_async().await;
    v1_replay.assert_async().await;
}

/// #746 counterpart: when the init segment's `If-Range` probe repeats the
/// SAME strong validator, the sidecar is trusted and resume continues —
/// vseg-1 (already recorded and intact on disk) must NOT be re-fetched.
#[tokio::test]
async fn resume_continues_when_init_etag_unchanged() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    let vi_first = server
        .mock("GET", "/vinit.mp4")
        .with_header("etag", "\"i1\"")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_first = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 => fail");

    drop(v2_fail);
    drop(v1_first);
    drop(vi_first);

    // Phase 2: the probe (one-byte ranged GET with If-Range) repeats etag
    // "i1" via a 206 — Confirmed. Init itself is never re-fetched (recorded
    // + intact on disk), so this probe mock is the ONLY hit /vinit.mp4 sees.
    let vi_probe = server
        .mock("GET", "/vinit.mp4")
        .match_header("if-range", "\"i1\"")
        .match_header("range", "bytes=0-0")
        .with_status(206)
        .with_header("content-range", "bytes 0-0/5")
        .with_header("etag", "\"i1\"")
        .with_body(b"V")
        .expect(1)
        .create_async()
        .await;
    let v1_replay = server
        .mock("GET", "/vseg-1.m4s")
        .expect(0)
        .with_body(b"V1")
        .create_async()
        .await;
    let _v2_ok = server
        .mock("GET", "/vseg-2.m4s")
        .with_body(b"V2")
        .expect_at_least(1)
        .create_async()
        .await;

    let result = downloader.download_to_file(&url, &out, None).await;
    assert!(
        result.is_ok(),
        "video-only resume must complete once the failing segment succeeds: {result:?}"
    );
    let output = tokio::fs::read(&out).await.unwrap();
    assert_eq!(
        output, b"VINITV1V2",
        "the resumed (not re-fetched) init+seg0 bytes must appear unchanged in the output"
    );

    vi_probe.assert_async().await;
    v1_replay.assert_async().await;
}

/// #746 sibling: a regenerated MPD that keeps the same segment NAMES but
/// changes `SegmentTemplate@duration` changes the manifest fingerprint, so
/// `load_matching` rejects the sidecar outright — before `run` ever reads
/// `anchor_validator` or sends a probe.
#[tokio::test]
async fn resume_starts_fresh_when_segment_timeline_changed_under_same_paths() {
    let mut server = Server::new_async().await;
    let _mpd = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body(&server.url()))
        .expect_at_least(1)
        .create_async()
        .await;
    // An etag on the phase-1 init response gives this sidecar an
    // `anchor_validator` too — the fingerprint mismatch below must reject it
    // BEFORE that validator is ever read, not merely because there happens
    // to be nothing to revalidate.
    let vi_first = server
        .mock("GET", "/vinit.mp4")
        .with_header("etag", "\"i1\"")
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_first = server
        .mock("GET", "/vseg-1.m4s")
        .with_body(b"V1")
        .expect_at_least(1)
        .create_async()
        .await;
    let v2_fail = server
        .mock("GET", "/vseg-2.m4s")
        .with_status(503)
        .expect_at_least(1)
        .create_async()
        .await;

    let dir = TempDir::new().unwrap();
    let out = dir.path().join("out.mp4");
    let url = format!("{}/manifest.mpd", server.url());
    let downloader = fast_dl();

    let _err = downloader
        .download_to_file(&url, &out, None)
        .await
        .expect_err("seg 2 503 => fail");

    // Phase 2: same segment NAMES (`vseg-1`/`vseg-2` under startNumber=1)
    // but a different `SegmentTemplate@duration` (4000 instead of 6000).
    // `_mpd`'s `expect_at_least(1)` is already satisfied by phase 1, so this
    // newer mock on the same method+path is served from here on.
    drop(v2_fail);
    drop(v1_first);
    drop(vi_first);

    let _mpd2 = server
        .mock("GET", "/manifest.mpd")
        .with_body(mpd_body_with_duration(&server.url(), 4000))
        .expect_at_least(1)
        .create_async()
        .await;
    // A fingerprint mismatch must reject the sidecar BEFORE any anchor
    // probe is attempted — this ranged/If-Range-shaped request must never
    // be sent. `vi_replay` below is scoped to `Range: Missing` so it cannot
    // silently absorb a probe request that DOES carry the header: mockito
    // serves whichever mock still has unmet hits, else the LAST matching
    // one, so an unscoped `vi_replay` would satisfy both the plain re-fetch
    // AND a stray probe, leaving `vi_no_probe.expect(0)` unable to ever fire
    // (round-1 review finding 2).
    let vi_no_probe = server
        .mock("GET", "/vinit.mp4")
        .match_header("range", "bytes=0-0")
        .expect(0)
        .create_async()
        .await;
    let vi_replay = server
        .mock("GET", "/vinit.mp4")
        .match_header("range", mockito::Matcher::Missing)
        .with_body(b"VINIT")
        .expect_at_least(1)
        .create_async()
        .await;
    let v1_replay = server
        .mock("GET", "/vseg-1.m4s")
        .expect_at_least(1)
        .with_body(b"V1")
        .create_async()
        .await;

    // Phase-2 outcome may be `Err` (the widened plan now has a third
    // segment with no mock) — only the mock hit counts below are asserted.
    let _ = downloader.download_to_file(&url, &out, None).await;

    v1_replay.assert_async().await;
    vi_no_probe.assert_async().await;
    let _ = vi_replay;
}
