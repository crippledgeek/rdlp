use super::*;
use mockito::Matcher;
use rdlp_types::Fragment;

#[tokio::test]
async fn byte_range_emits_http_range_header() {
    let mut server = mockito::Server::new_async().await;
    let _seg = server
        .mock("GET", "/seg.m4s")
        .match_header("Range", Matcher::Regex(r"^bytes=1024-2047$".to_string()))
        .with_status(206)
        .with_header("Content-Range", "bytes 1024-2047/8192")
        .with_body(b"X".repeat(1024))
        .expect(1)
        .create_async()
        .await;

    // Catch-all if the Range header is missing or wrong. 418 rather than a
    // 5xx because a canary's job is to fail the test *fast*: since #570 a
    // 5xx is retryable, so a regression here would walk the whole backoff
    // ladder before surfacing. `is_retryable_error` rejects 418.
    let _unmatched = server
        .mock("GET", Matcher::Any)
        .with_status(418)
        .create_async()
        .await;

    let url = format!("{}/seg.m4s", server.url());
    let frags = vec![Fragment {
        url: url.clone(),
        byte_range: Some((1024, 2048)), // (start, end_exclusive)
        init_url: None,
        init_byte_range: None,
        duration: Some(6.0),
        filesize: None,
    }];

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("download must succeed with Range header");
}

#[tokio::test]
async fn multi_init_dedups_fetches() {
    let mut server = mockito::Server::new_async().await;

    // Init A is needed for fragments 1 + 2 → 1 fetch (deduped on consecutive).
    let _init_a = server
        .mock("GET", "/init-a.mp4")
        .with_body(b"INITA")
        .expect(1)
        .create_async()
        .await;

    // Init B is needed for fragment 3 → 1 fetch.
    let _init_b = server
        .mock("GET", "/init-b.mp4")
        .with_body(b"INITB")
        .expect(1)
        .create_async()
        .await;

    // 3 data segments.
    for i in 1..=3_u32 {
        server
            .mock("GET", format!("/seg-{i}.m4s").as_str())
            .with_body(b"DATA")
            .expect(1)
            .create_async()
            .await;
    }

    let init_a = format!("{}/init-a.mp4", server.url());
    let init_b = format!("{}/init-b.mp4", server.url());

    let frags: Vec<Fragment> = (1..=3_u32)
        .map(|i| Fragment {
            url: format!("{}/seg-{i}.m4s", server.url()),
            byte_range: None,
            init_url: Some(if i < 3 {
                init_a.clone()
            } else {
                init_b.clone()
            }),
            init_byte_range: None,
            duration: Some(6.0),
            filesize: None,
        })
        .collect();

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("multi-init download must succeed");

    let written = tokio::fs::read(tmp.path()).await.unwrap();
    // Expected: INITA + DATA + DATA + INITB + DATA = 5 + 4 + 4 + 5 + 4 = 22 bytes.
    assert_eq!(written.len(), 22);
    assert_eq!(&written[0..5], b"INITA");
    assert_eq!(&written[5..9], b"DATA");
    assert_eq!(&written[9..13], b"DATA");
    assert_eq!(&written[13..18], b"INITB");
    assert_eq!(&written[18..22], b"DATA");
}

// ---- Progress capture mock + tests (issue #272) ----

use std::sync::Mutex;

struct CaptureCallback {
    events: Mutex<Vec<DownloadProgress>>,
}

impl CaptureCallback {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn events(&self) -> Vec<DownloadProgress> {
        self.events.lock().expect("lock").clone()
    }
}

impl ProgressCallback for CaptureCallback {
    fn on_progress(&self, p: &DownloadProgress) {
        self.events.lock().expect("lock").push(p.clone());
    }
    fn on_complete(&self, _stats: &DownloadStats) {}
    fn on_error(&self, _msg: &str) {}
}

fn frag(url: String) -> Fragment {
    Fragment {
        url,
        byte_range: None,
        init_url: None,
        init_byte_range: None,
        duration: None,
        filesize: None,
    }
}

#[tokio::test]
async fn fragment_progress_final_boundary_emit_covers_all_frags() {
    // N=2 fragments. The 100ms throttle suppresses intermediate emits when
    // fragments arrive quickly, but the final-fragment boundary rule
    // (`frags_done == total_frags`) guarantees at least one event with
    // segments_downloaded == total_segments and bytes_downloaded == total.
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let _f2 = server
        .mock("GET", "/f2")
        .with_body(vec![0u8; 200])
        .create_async()
        .await;
    let frags = vec![
        frag(format!("{}/f1", server.url())),
        frag(format!("{}/f2", server.url())),
    ];
    let cb = CaptureCallback::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _stats = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        Some(300),
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    let evs = cb.events();
    assert!(!evs.is_empty(), "boundary emit must fire at least once");
    let last = evs.last().expect("at least one");
    assert_eq!(last.segments_downloaded, Some(2));
    assert_eq!(last.total_segments, Some(2));
    assert_eq!(last.bytes_downloaded, 300);
    assert!(
        !last.is_estimated,
        "explicit Content-Length is authoritative, not estimated"
    );
}

#[tokio::test]
async fn fragment_progress_none_callback_runs_to_completion() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let frags = vec![frag(format!("{}/f1", server.url()))];
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("ok");
    assert_eq!(stats.bytes_downloaded, 100);
}

#[tokio::test]
async fn fragment_no_byte_total_uses_byte_extrapolation() {
    // HLS with no Content-Length: progress + total must now be byte-extrapolated
    // (total_bytes Some, is_estimated true).
    let mut server = mockito::Server::new_async().await;
    for i in 0..4_u32 {
        server
            .mock("GET", format!("/f{i}").as_str())
            .with_body(vec![0u8; 100])
            .create_async()
            .await;
    }
    let frags: Vec<Fragment> = (0..4)
        .map(|i| frag(format!("{}/f{i}", server.url())))
        .collect();
    let cb = CaptureCallback::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _ = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None, /* expected_total */
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    let evs = cb.events();
    let last = evs.last().expect("at least one event");
    // Byte-extrapolation: a total is now known (estimated) — was None pre-change.
    assert!(
        last.total_bytes.is_some(),
        "estimated total must be reported"
    );
    assert!(last.is_estimated, "no-Content-Length total is an estimate");
    // NOTE: eta is intentionally NOT asserted here — SpeedMeter's 50ms sample
    // gate yields speed 0 for instant mock downloads, so new()'s `speed > 1.0`
    // eta gate produces None in-test. Byte-extrapolation eta is unit-covered by
    // downloader.rs's `new()` eta test (real speed → Some). The byte-extrapolation
    // *behavior* is guarded below by total_bytes.is_some() + is_estimated.
    // Final fragment: frags_done == total_frags → est_total == total_bytes → 100%.
    let frac = last.progress.expect("progress some").fraction();
    assert!(
        (frac - 1.0).abs() < 1e-6,
        "final progress is 1/1; got {frac}"
    );
    // Segment counts survive as secondary info.
    assert_eq!(last.segments_downloaded, Some(4));
    assert_eq!(last.total_segments, Some(4));
}

#[tokio::test]
async fn fragment_progress_cdn_overrun_saturates_at_one() {
    // expected_total smaller than actual bytes — Progress::from_ratio saturates.
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 200])
        .create_async()
        .await;
    let frags = vec![frag(format!("{}/f1", server.url()))];
    let cb = CaptureCallback::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _ = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        Some(100),
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    let final_p = cb
        .events()
        .last()
        .expect("at least one")
        .progress
        .expect("set");
    assert!((final_p.fraction() - 1.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn fragment_progress_expected_total_larger_completes_under_one() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let frags = vec![frag(format!("{}/f1", server.url()))];
    let cb = CaptureCallback::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _ = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        Some(1_000),
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    let final_p = cb
        .events()
        .last()
        .expect("at least one")
        .progress
        .expect("set");
    // Pin the exact ratio: 100 / 1000 = 0.1. A regression that drops the
    // numerator (e.g. emits with `bytes_downloaded = 0`) would still pass
    // a bare `< 1.0` assertion — this catches it.
    assert!(
        (final_p.fraction() - 0.1).abs() < 0.01,
        "expected ~0.1, got {}",
        final_p.fraction()
    );
}

#[tokio::test]
async fn fragment_progress_zero_fragments_no_emit() {
    let frags: Vec<Fragment> = vec![];
    let cb = CaptureCallback::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let stats = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None,
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    assert_eq!(stats.bytes_downloaded, 0);
    assert_eq!(cb.events().len(), 0);
    assert!(tmp.path().exists());
}

#[tokio::test]
async fn fragment_progress_n_one_emits_exactly_once() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let frags = vec![frag(format!("{}/f1", server.url()))];
    let cb = CaptureCallback::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _ = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        Some(100),
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    assert_eq!(cb.events().len(), 1);
}

#[tokio::test]
async fn fragment_progress_mid_stream_failure_returns_error_partial_file() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let _f2 = server
        .mock("GET", "/f2")
        .with_status(500)
        .create_async()
        .await;
    let frags = vec![
        frag(format!("{}/f1", server.url())),
        frag(format!("{}/f2", server.url())),
    ];
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    // Retries off: the 500 is retryable since #570, and the default backoff
    // would spend minutes here proving something this test is not about.
    let http = no_retry_http();
    let res =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await;
    // Pin the typed error: a regression that reshapes the failure into
    // Cancelled / Network / Other would silently pass a bare is_err().
    let err = res.expect_err("must error on f2 500");
    assert!(
        matches!(
            err,
            rdlp_core::RdlpError::Download { .. } | rdlp_core::RdlpError::Http { .. }
        ),
        "unexpected err shape: {err:?}"
    );
    let written = tokio::fs::metadata(tmp.path()).await.expect("exists").len();
    // First fragment was written + flushed before f2 attempted; f2 errors
    // before any of its bytes reach the file.
    assert_eq!(written, 100, "exact first-fragment bytes; got {written}");
}

#[tokio::test]
async fn single_init_fetched_once() {
    let mut server = mockito::Server::new_async().await;

    // Init must be fetched EXACTLY ONCE for 10 fragments.
    let _init = server
        .mock("GET", "/init.mp4")
        .with_body(b"INIT")
        .expect(1)
        .create_async()
        .await;

    for i in 1..=10_u32 {
        server
            .mock("GET", format!("/seg-{i}.m4s").as_str())
            .with_body(b"D")
            .expect(1)
            .create_async()
            .await;
    }

    let init_url = format!("{}/init.mp4", server.url());
    let frags: Vec<Fragment> = (1..=10_u32)
        .map(|i| Fragment {
            url: format!("{}/seg-{i}.m4s", server.url()),
            byte_range: None,
            init_url: Some(init_url.clone()),
            init_byte_range: None,
            duration: Some(6.0),
            filesize: None,
        })
        .collect();

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("single-init download must succeed");

    let written = tokio::fs::read(tmp.path()).await.unwrap();
    // 4 (INIT) + 10 × 1 (D) = 14 bytes.
    assert_eq!(written.len(), 14);
}

// ---- Cancellation regression tests (issue #272) ----

/// Black-hole TCP listener: accepts connections, then sleeps without
/// responding. Forces the client to hang until cancellation fires.
///
/// The spawned task is intentionally not joined; it lives for the rest
/// of the test process. Each invocation leaks one tokio task plus one
/// open TCP listener socket. Acceptable for a handful of tests.
///
/// Mirror of `crates/rdlp-extractor/src/base/common/mod.rs:563`'s helper,
/// which is module-private to that crate.
async fn spawn_blackhole() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                drop(stream);
            }
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fragment_cancel_before_first_returns_immediately() {
    let port = spawn_blackhole().await;
    let frags = vec![frag(format!("http://127.0.0.1:{port}/f1"))];
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel(); // pre-cancel
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let started = std::time::Instant::now();
    let res = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None,
        None,
        tmp.path(),
        None,
        Some(&token),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(matches!(res, Err(rdlp_core::RdlpError::Cancelled)));
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "should return fast; got {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fragment_cancel_mid_stream_returns_cancelled_partial_file() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let port = spawn_blackhole().await;
    let frags = vec![
        frag(format!("{}/f1", server.url())),
        frag(format!("http://127.0.0.1:{port}/f2")),
    ];
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();
    tokio::spawn(async move {
        // 500ms gives the helper time to fully fetch + write + flush f1
        // (mockito serves instantly; the budget covers tokio scheduling
        // jitter on loaded CI runners). Lower budgets risk firing cancel
        // before f1 lands on disk, which would break the partial-file
        // assertion below.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        token_clone.cancel();
    });
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            Some(&token),
        ),
    )
    .await
    .expect("test timeout");
    assert!(matches!(res, Err(rdlp_core::RdlpError::Cancelled)));
    let written = tokio::fs::metadata(tmp.path()).await.expect("exists").len();
    // First fragment fully written + flushed; second hung in the black-hole
    // until token.cancel() fired between fragments. Output has exactly the
    // first fragment's bytes.
    assert_eq!(
        written, 100,
        "first fragment should be flushed; got {written}"
    );
}

#[tokio::test]
async fn fragment_cancel_none_runs_to_completion() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let frags = vec![frag(format!("{}/f1", server.url()))];
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("ok");
    assert_eq!(stats.bytes_downloaded, 100);
}

#[tokio::test]
async fn fragment_cancel_never_fired_runs_to_completion() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let frags = vec![frag(format!("{}/f1", server.url()))];
    let token = tokio_util::sync::CancellationToken::new();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let stats = download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None,
        None,
        tmp.path(),
        None,
        Some(&token),
    )
    .await
    .expect("ok");
    assert_eq!(stats.bytes_downloaded, 100);
}

// ---- F1 Task 3: AIMD wiring test ----

/// Verifies the AIMD controller is wired with `ControllerMode::HlsSegments`
/// and that `report_segment_complete` is called (observable via AIMD-adjusted
/// concurrency still completing all fragments correctly).
///
/// The failing-first contract: Task 2 uses a fixed `Arc<Semaphore>` with no
/// `AdaptiveController`. Task 3 replaces it with a controller. We assert two
/// things that are only true after Task 3 lands:
///
/// 1. `AdaptiveController::mode()` returns `HlsSegments` — verified directly
///    on a controller constructed with the same parameters as the production
///    path (additive unit test for the new `mode()` accessor).
/// 2. The download completes correctly with the fixed concurrency
///    (`max_connections=2`) — a regression guard that AIMD wiring does
///    not break source-order writes or omit bytes.
///
/// The test FAILS against Task 2's code because the `mode()` accessor did not
/// exist on `AdaptiveController` before Task 3 added it (it is a NEW public
/// method added in this task). Once the accessor is added and the controller
/// is wired, both assertions pass.
#[tokio::test]
async fn aimd_controller_hls_segments_mode_wired_and_download_correct() {
    use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};

    // Part 1: unit test for the new mode() accessor.
    let controller = AdaptiveController::new(
        0,
        AdaptiveConfig {
            max_connections: 2,
            ..AdaptiveConfig::default()
        },
        ControllerMode::HlsSegments,
        None,
    );
    assert_eq!(
        controller.mode(),
        ControllerMode::HlsSegments,
        "AdaptiveController::mode() must return the ControllerMode it was constructed with"
    );

    // Part 2: behavioral regression guard — download completes correctly
    // when AIMD controller replaces the fixed semaphore.
    let mut server = mockito::Server::new_async().await;
    let (frags, expected) = build_ordered_frags(&mut server, 6).await;
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    // N=2 concurrency — AIMD starts at min(2,2)=2 connections.
    let http = HttpDownloader::with_client(wreq::Client::new()).with_concurrent_fragments(2);
    let stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("AIMD-wired download must complete");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(
        written, expected,
        "AIMD wiring must not break source-order writes"
    );
    assert_eq!(
        stats.bytes_downloaded, 6,
        "all 6 bytes must be accounted for"
    );
}

// ---- F1 parallel fragment-fetch tests ----

/// Build N fragments pointing at sequentially-numbered mockito mocks each
/// returning a unique 1-byte body so concatenation order is observable.
/// Returns (frags, expected concatenated bytes).
async fn build_ordered_frags(server: &mut mockito::Server, n: usize) -> (Vec<Fragment>, Vec<u8>) {
    let mut expected = Vec::with_capacity(n);
    let mut frags = Vec::with_capacity(n);
    for i in 0..n {
        // 1-byte body per fragment; byte value = index (mod 256) so the
        // concatenated output reads as 0,1,2,3,... in source order.
        let body = vec![(i % 256) as u8];
        expected.push(body[0]);
        server
            .mock("GET", format!("/seg-{i}").as_str())
            .with_body(body.clone())
            .expect_at_least(1)
            .create_async()
            .await;
        frags.push(frag(format!("{}/seg-{i}", server.url())));
    }
    (frags, expected)
}

/// Structural proof that the stream combinator polls N items concurrently.
///
/// Uses `tokio::sync::Barrier` to force all N futures to be simultaneously
/// in-flight before any can proceed. Sequential polling deadlocks at the
/// barrier (caught by `tokio::time::timeout`); concurrent polling releases
/// it. Independent of wall-clock and mock-server thread pools — passes
/// deterministically on Linux / macOS / Windows.
///
/// Architectural note: this test exercises `futures::stream::buffered(N)`'s
/// concurrency semantic in isolation, NOT `download_pre_resolved_fragments`
/// directly. The production function's use of `buffered(N)` (vs. a `for`
/// loop or `buffer_unordered`) is regression-guarded by
/// `parallel_fetch_writes_in_source_order_not_arrival_order` below — that
/// test would fail under either a sequential `for` loop (semantic match by
/// accident) or `buffer_unordered` (arrival-order output) when run against
/// 12 mockito-served fragments with default concurrency=8. The two tests
/// together cover both the library combinator and the production wiring.
///
/// `Barrier::wait` is documented as "not cancel safe" — this is harmless
/// here. The barrier is created fresh per test run and is never reused
/// after the `tokio::time::timeout` drops it; an inconsistent rendezvous
/// state on cancel never leaks to a subsequent test or run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_stream_polls_n_items_concurrently() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Barrier;

    const N: usize = 4;
    let max_observed = Arc::new(AtomicUsize::new(0));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(N));

    let futures: Vec<_> = (0..N)
        .map(|_| {
            let max_observed = Arc::clone(&max_observed);
            let in_flight = Arc::clone(&in_flight);
            let barrier = Arc::clone(&barrier);
            async move {
                let cur = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                let mut prev = max_observed.load(Ordering::SeqCst);
                while cur > prev {
                    match max_observed.compare_exchange(
                        prev,
                        cur,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(actual) => prev = actual,
                    }
                }
                barrier.wait().await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
            }
        })
        .collect();

    use futures::stream::{self, StreamExt as _};
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream::iter(futures).buffered(N).collect::<Vec<_>>(),
    )
    .await;

    assert!(
        result.is_ok(),
        "stream deadlocked at the barrier — futures were polled sequentially"
    );

    let peak = max_observed.load(Ordering::SeqCst);
    assert_eq!(
        peak, N,
        "expected all {N} futures in-flight simultaneously; observed peak={peak}"
    );
}

#[tokio::test]
async fn parallel_fetch_writes_in_source_order_not_arrival_order() {
    // 12 fragments, default N=8 in the new path. Each mock body = its index.
    // If the implementation used buffer_unordered (arrival order) instead of
    // buffered (source order), output bytes would be a permutation of 0..12,
    // not the literal sequence 0,1,2,...,11.
    let mut server = mockito::Server::new_async().await;
    let (frags, expected) = build_ordered_frags(&mut server, 12).await;
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("ok");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(
        written, expected,
        "output must preserve fragment source order; arrival-order is a regression"
    );
}

#[tokio::test]
async fn parallel_fetch_byte_identical_to_sequential_for_six_frags() {
    // 6 fragments, each with a distinct 1024-byte body. Output must be the
    // exact source-order concatenation regardless of internal concurrency.
    let mut server = mockito::Server::new_async().await;
    let mut bodies: Vec<Vec<u8>> = Vec::new();
    let mut frags: Vec<Fragment> = Vec::new();
    for i in 0..6_u8 {
        let body = vec![i; 1024];
        bodies.push(body.clone());
        server
            .mock("GET", format!("/seg-{i}").as_str())
            .with_body(body)
            .expect_at_least(1)
            .create_async()
            .await;
        frags.push(frag(format!("{}/seg-{i}", server.url())));
    }
    let expected: Vec<u8> = bodies.into_iter().flatten().collect();
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("ok");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(written.len(), 6 * 1024);
    assert_eq!(written, expected);
}

#[tokio::test]
async fn parallel_fetch_n_one_degenerates_to_sequential() {
    // Construct a Config with concurrent_fragments=1 and verify byte-for-byte
    // identical output to the multi-fragment case. This is the regression
    // guard for the N=1 boundary — buffered(1) MUST still execute correctly.
    let mut server = mockito::Server::new_async().await;
    let (frags, expected) = build_ordered_frags(&mut server, 4).await;
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new()).with_concurrent_fragments(1);
    let _stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("ok");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(
        written, expected,
        "N=1 must degenerate cleanly to sequential"
    );
}

#[tokio::test]
async fn parallel_fetch_count_greater_than_concurrency_completes_all() {
    // 12 fragments, N=4 — every fragment must download exactly once and
    // appear in source order in the output.
    let mut server = mockito::Server::new_async().await;
    let (frags, expected) = build_ordered_frags(&mut server, 12).await;
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new()).with_concurrent_fragments(4);
    let _stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("ok");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(written.len(), 12);
    assert_eq!(written, expected);
}

#[tokio::test]
async fn parallel_fetch_init_transition_preserves_decode_order() {
    // 4 fragments under init A, then 4 under init B. With buffered(N>1),
    // fragments may complete out of arrival order; the source-order write
    // discipline must still place INITA before its 4 frags and INITB before
    // its 4. Output: INITA + 4×A + INITB + 4×B = 18 bytes.
    let mut server = mockito::Server::new_async().await;
    let _init_a = server
        .mock("GET", "/init-a.mp4")
        .with_body(b"INITA")
        .expect_at_least(1)
        .create_async()
        .await;
    let _init_b = server
        .mock("GET", "/init-b.mp4")
        .with_body(b"INITB")
        .expect_at_least(1)
        .create_async()
        .await;
    for i in 1..=4_u32 {
        server
            .mock("GET", format!("/a-{i}.m4s").as_str())
            .with_body(b"A")
            .expect(1)
            .create_async()
            .await;
        server
            .mock("GET", format!("/b-{i}.m4s").as_str())
            .with_body(b"B")
            .expect(1)
            .create_async()
            .await;
    }
    let init_a = format!("{}/init-a.mp4", server.url());
    let init_b = format!("{}/init-b.mp4", server.url());
    let mut frags: Vec<Fragment> = Vec::new();
    for i in 1..=4_u32 {
        frags.push(Fragment {
            url: format!("{}/a-{i}.m4s", server.url()),
            byte_range: None,
            init_url: Some(init_a.clone()),
            init_byte_range: None,
            duration: Some(6.0),
            filesize: None,
        });
    }
    for i in 1..=4_u32 {
        frags.push(Fragment {
            url: format!("{}/b-{i}.m4s", server.url()),
            byte_range: None,
            init_url: Some(init_b.clone()),
            init_byte_range: None,
            duration: Some(6.0),
            filesize: None,
        });
    }
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new()).with_concurrent_fragments(4);
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("init-transition under parallel must succeed");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    // INITA (5) + AAAA (4) + INITB (5) + BBBB (4) = 18 bytes.
    assert_eq!(written.len(), 18, "got {written:?}");
    assert_eq!(&written[0..5], b"INITA");
    assert_eq!(&written[5..9], b"AAAA");
    assert_eq!(&written[9..14], b"INITB");
    assert_eq!(&written[14..18], b"BBBB");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fragment_cancel_does_not_delete_partial_file() {
    let mut server = mockito::Server::new_async().await;
    let _f1 = server
        .mock("GET", "/f1")
        .with_body(vec![0u8; 100])
        .create_async()
        .await;
    let port = spawn_blackhole().await;
    let frags = vec![
        frag(format!("{}/f1", server.url())),
        frag(format!("http://127.0.0.1:{port}/f2")),
    ];
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();
    tokio::spawn(async move {
        // 500ms gives the helper time to fully fetch + write + flush f1
        // (mockito serves instantly; the budget covers tokio scheduling
        // jitter on loaded CI runners). Lower budgets risk firing cancel
        // before f1 lands on disk, which would break the partial-file
        // assertion below.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        token_clone.cancel();
    });
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let http = HttpDownloader::with_client(wreq::Client::new());
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            Some(&token),
        ),
    )
    .await;
    assert!(
        tmp.path().exists(),
        "partial file should be preserved for resume"
    );
    let written = tokio::fs::metadata(tmp.path()).await.expect("exists").len();
    assert!(written > 0);
}

// ---- Same-origin header gate tests (issue #273) ----

/// Build an `HttpDownloader` with a single extra header baked in.
///
/// Mirrors the pattern used in the HLS/DASH production paths where
/// `Format.http_headers` are loaded via `with_extra_headers`.
fn make_downloader_with_header(name: &str, value: &str) -> HttpDownloader {
    let mut headers = std::collections::HashMap::new();
    headers.insert(name.to_string(), value.to_string());
    HttpDownloader::with_client(wreq::Client::new()).with_extra_headers(Some(&headers))
}

/// Negative test: cross-origin init URL must NOT receive `Format.http_headers`.
///
/// Two mockito servers = two origins (same loopback address, different ports).
/// The fragment is served from `format_server` (same-origin) → must see Referer.
/// The init segment is served from `init_server` (cross-origin) → must NOT see
/// Referer. The catch-all 418 on `init_server` fires if the header leaks.
#[tokio::test]
async fn cross_origin_init_url_does_not_forward_seed_headers() {
    let mut format_server = mockito::Server::new_async().await;
    let mut init_server = mockito::Server::new_async().await;

    // Fragment on format_server: must see Referer (same-origin).
    let _frag_mock = format_server
        .mock("GET", "/frag0.m4s")
        .match_header("Referer", "https://operator.example.com/page")
        .with_body(&[0u8; 8][..])
        .expect(1)
        .create_async()
        .await;

    // Init on init_server: must NOT see Referer (cross-origin).
    let _init_mock = init_server
        .mock("GET", "/init.m4s")
        .match_header("Referer", Matcher::Missing)
        .with_body(&[0u8; 4][..])
        .expect(1)
        .create_async()
        .await;

    // Catch-all on init_server: 418 if Referer arrived unexpectedly.
    let _init_catchall = init_server
        .mock("GET", Matcher::Any)
        .with_status(418)
        .create_async()
        .await;

    let init_abs_url = format!("{}/init.m4s", init_server.url());
    let frag_abs_url = format!("{}/frag0.m4s", format_server.url());

    let frags = vec![Fragment {
        url: frag_abs_url.clone(),
        duration: Some(2.0),
        byte_range: None,
        init_url: Some(init_abs_url),
        init_byte_range: None,
        filesize: None,
    }];

    let http = make_downloader_with_header("Referer", "https://operator.example.com/page");
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let format_url = format!("{}/page", format_server.url());

    download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None,
        None,
        tmp.path(),
        Some(&format_url),
        None,
    )
    .await
    .expect("same-origin fragment sees Referer, cross-origin init does not");
}

/// Positive companion: same-origin init URL DOES forward `Format.http_headers`.
///
/// Both fragment and init are on `format_server` (same-origin as `format_url`).
/// Both must receive the Referer. The catch-all 418 fires if headers were
/// stripped due to an over-aggressive same-origin check (always-strip regression).
#[tokio::test]
async fn same_origin_init_url_forwards_seed_headers() {
    let mut format_server = mockito::Server::new_async().await;

    let _init_mock = format_server
        .mock("GET", "/init.m4s")
        .match_header("Referer", "https://operator.example.com/page")
        .with_body(&[0u8; 4][..])
        .expect(1)
        .create_async()
        .await;

    let _frag_mock = format_server
        .mock("GET", "/frag0.m4s")
        .match_header("Referer", "https://operator.example.com/page")
        .with_body(&[0u8; 8][..])
        .expect(1)
        .create_async()
        .await;

    // Catch-all: 418 if Referer was NOT present (headers were stripped).
    let _catchall = format_server
        .mock("GET", Matcher::Any)
        .with_status(418)
        .create_async()
        .await;

    let init_abs_url = format!("{}/init.m4s", format_server.url());
    let frag_abs_url = format!("{}/frag0.m4s", format_server.url());
    let format_url = format!("{}/page", format_server.url());

    let frags = vec![Fragment {
        url: frag_abs_url,
        duration: Some(2.0),
        byte_range: None,
        init_url: Some(init_abs_url),
        init_byte_range: None,
        filesize: None,
    }];

    let http = make_downloader_with_header("Referer", "https://operator.example.com/page");
    let tmp = tempfile::NamedTempFile::new().unwrap();

    download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None,
        None,
        tmp.path(),
        Some(&format_url),
        None,
    )
    .await
    .expect("same-origin init + fragment must both receive Referer");
}

/// Negative test: fragment URL on a different origin from `format_url` drops headers.
///
/// `init_url` is `None`; only the fragment itself is cross-origin. The catch-all
/// 418 on `cross_server` fires if the Referer was forwarded.
#[tokio::test]
async fn cross_origin_fragment_url_does_not_forward_seed_headers() {
    let format_server = mockito::Server::new_async().await;
    let mut cross_server = mockito::Server::new_async().await;

    // Cross-origin fragment: must NOT see Referer.
    let _frag_mock = cross_server
        .mock("GET", "/frag0.m4s")
        .match_header("Referer", Matcher::Missing)
        .with_body(&[0u8; 8][..])
        .expect(1)
        .create_async()
        .await;

    // Catch-all on cross_server: 418 if Referer arrived.
    let _catchall = cross_server
        .mock("GET", Matcher::Any)
        .with_status(418)
        .create_async()
        .await;

    // format_server provides a mock master url to establish format_url's origin,
    // but we only need it for the URL — no actual fetch against it.
    let format_url = format!("{}/master.m3u8", format_server.url());
    let frag_abs_url = format!("{}/frag0.m4s", cross_server.url());

    let frags = vec![Fragment {
        url: frag_abs_url,
        duration: Some(2.0),
        byte_range: None,
        init_url: None,
        init_byte_range: None,
        filesize: None,
    }];

    let http = make_downloader_with_header("Referer", "https://operator.example.com/page");
    let tmp = tempfile::NamedTempFile::new().unwrap();

    download_pre_resolved_fragments(
        &http,
        &frags,
        None,
        None,
        None,
        tmp.path(),
        Some(&format_url),
        None,
    )
    .await
    .expect("cross-origin fragment must not receive Referer");
}

// ---- HLS resume tests (issue #354) ----

use super::state::{HlsResumeState, fragment_fingerprint};

/// Deterministic N fragments where body[i] = i (mod 256), each served by a
/// fresh mock on `server`. Returns (frags, expected full concatenation).
/// Distinct from `build_ordered_frags` only in that the caller controls the
/// path stem so two servers can serve byte-identical bodies for resume tests.
async fn seeded_frags(server: &mut mockito::Server, n: usize) -> (Vec<Fragment>, Vec<u8>) {
    let mut expected = Vec::with_capacity(n);
    let mut frags = Vec::with_capacity(n);
    for i in 0..n {
        let body = vec![(i % 256) as u8];
        expected.push(body[0]);
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_body(body)
            .expect_at_least(1)
            .create_async()
            .await;
        frags.push(frag(format!("{}/seg-{i}.ts", server.url())));
    }
    (frags, expected)
}

#[tokio::test]
async fn full_download_removes_sidecar() {
    let mut server = mockito::Server::new_async().await;
    let (frags, expected) = seeded_frags(&mut server, 5).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("video.ts");
    let http = HttpDownloader::with_client(wreq::Client::new());
    download_pre_resolved_fragments(&http, &frags, None, None, None, &output, None, None)
        .await
        .expect("full download ok");
    let written = tokio::fs::read(&output).await.unwrap();
    assert_eq!(written, expected);
    let sidecar = output.with_extension("ts.hls_state.json");
    assert!(
        !sidecar.exists(),
        "sidecar must be removed on successful completion"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_writes_sidecar_with_progress() {
    // Sequential (concurrency=1) so the cancel lands deterministically between
    // fragments. A blackhole at index 2 hangs; cancel fires after f0+f1 land.
    let mut server = mockito::Server::new_async().await;
    for i in 0..2_u32 {
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_body(vec![i as u8; 10])
            .create_async()
            .await;
    }
    let port = spawn_blackhole().await;
    let mut frags: Vec<Fragment> = (0..2)
        .map(|i| frag(format!("{}/seg-{i}.ts", server.url())))
        .collect();
    frags.push(frag(format!("http://127.0.0.1:{port}/seg-2.ts")));

    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("video.ts");
    let token = tokio_util::sync::CancellationToken::new();
    let token_clone = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        token_clone.cancel();
    });
    let http = HttpDownloader::with_client(wreq::Client::new()).with_concurrent_fragments(1);
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            &output,
            None,
            Some(&token),
        ),
    )
    .await
    .expect("test timeout");
    assert!(matches!(res, Err(rdlp_core::RdlpError::Cancelled)));

    let partial_len = tokio::fs::metadata(&output)
        .await
        .expect("partial exists")
        .len();
    assert!(partial_len > 0, "partial must be present");
    let sidecar = output.with_extension("ts.hls_state.json");
    assert!(
        sidecar.exists(),
        "sidecar must persist on cancel for resume"
    );

    let fp = fragment_fingerprint(&frags);
    let total = frags.len() as u64;
    let st = HlsResumeState::load_matching(&sidecar, fp, total)
        .await
        .expect("sidecar must match the same fragment list");
    assert!(st.fragments_done >= 1, "at least one fragment recorded");
    assert_eq!(
        st.byte_len, partial_len,
        "byte_len tracks the flushed partial"
    );
}

#[tokio::test]
async fn resume_is_byte_identical_and_skips_done_fragments() {
    // Reference: uninterrupted full download.
    let mut ref_server = mockito::Server::new_async().await;
    let (ref_frags, expected) = seeded_frags(&mut ref_server, 6).await;
    let refdir = tempfile::tempdir().expect("tempdir");
    let refout = refdir.path().join("ref.ts");
    let http = HttpDownloader::with_client(wreq::Client::new());
    download_pre_resolved_fragments(&http, &ref_frags, None, None, None, &refout, None, None)
        .await
        .expect("reference ok");
    let reference = tokio::fs::read(&refout).await.unwrap();
    assert_eq!(reference, expected);

    // Craft a partial of the first 3 fragments + a matching sidecar.
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("video.ts");
    let done = 3usize;
    tokio::fs::write(&output, &reference[..done]).await.unwrap();

    // Resume server: paths identical (so the path-only fingerprint matches),
    // but the already-done fragments are 418 — if resume re-fetches them the
    // download errors, proving they were skipped. Remaining fragments serve
    // their real bodies.
    let mut server = mockito::Server::new_async().await;
    for i in 0..done {
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_status(418)
            .create_async()
            .await;
    }
    for i in done..6 {
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_body(vec![(i % 256) as u8])
            .expect(1)
            .create_async()
            .await;
    }
    let frags: Vec<Fragment> = (0..6)
        .map(|i| frag(format!("{}/seg-{i}.ts", server.url())))
        .collect();

    let sidecar = output.with_extension("ts.hls_state.json");
    let mut st = HlsResumeState::new(fragment_fingerprint(&frags), 6);
    st.fragments_done = done as u64;
    st.byte_len = done as u64; // 1 byte per fragment
    st.save(&sidecar).await.expect("seed sidecar");

    download_pre_resolved_fragments(&http, &frags, None, None, None, &output, None, None)
        .await
        .expect("resume must succeed without hitting the 418 done-fragments");

    let written = tokio::fs::read(&output).await.unwrap();
    assert_eq!(written, reference, "resumed output must be byte-identical");
    assert!(!sidecar.exists(), "sidecar removed after completion");
}

#[tokio::test]
async fn fingerprint_mismatch_restarts_from_zero() {
    let mut server = mockito::Server::new_async().await;
    let (frags, expected) = seeded_frags(&mut server, 4).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("video.ts");

    // Seed a stale partial + sidecar whose fingerprint does NOT match `frags`.
    tokio::fs::write(&output, b"STALEDATA").await.unwrap();
    let sidecar = output.with_extension("ts.hls_state.json");
    let mut st = HlsResumeState::new(0x1234_5678, 4); // wrong fingerprint
    st.fragments_done = 2;
    st.byte_len = 9; // matches "STALEDATA"
    st.save(&sidecar).await.expect("seed sidecar");

    let http = HttpDownloader::with_client(wreq::Client::new());
    download_pre_resolved_fragments(&http, &frags, None, None, None, &output, None, None)
        .await
        .expect("mismatch restarts cleanly");
    let written = tokio::fs::read(&output).await.unwrap();
    assert_eq!(
        written, expected,
        "stale partial discarded; fresh full download"
    );
    assert!(!sidecar.exists());
}

#[tokio::test]
async fn extra_tail_is_truncated_to_byte_len_on_resume() {
    // Reference full download.
    let mut ref_server = mockito::Server::new_async().await;
    let (ref_frags, _expected) = seeded_frags(&mut ref_server, 5).await;
    let refdir = tempfile::tempdir().expect("tempdir");
    let refout = refdir.path().join("ref.ts");
    let http = HttpDownloader::with_client(wreq::Client::new());
    download_pre_resolved_fragments(&http, &ref_frags, None, None, None, &refout, None, None)
        .await
        .expect("reference ok");
    let reference = tokio::fs::read(&refout).await.unwrap();

    // Partial = first 2 fragments + a torn extra byte; sidecar says byte_len=2.
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("video.ts");
    let mut partial = reference[..2].to_vec();
    partial.push(0xFF); // torn tail beyond the confirmed boundary
    tokio::fs::write(&output, &partial).await.unwrap();

    let mut server = mockito::Server::new_async().await;
    for i in 0..2 {
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_status(418)
            .create_async()
            .await;
    }
    for i in 2..5 {
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_body(vec![(i % 256) as u8])
            .expect(1)
            .create_async()
            .await;
    }
    let frags: Vec<Fragment> = (0..5)
        .map(|i| frag(format!("{}/seg-{i}.ts", server.url())))
        .collect();
    let sidecar = output.with_extension("ts.hls_state.json");
    let mut st = HlsResumeState::new(fragment_fingerprint(&frags), 5);
    st.fragments_done = 2;
    st.byte_len = 2; // confirmed boundary BEFORE the torn 0xFF byte
    st.save(&sidecar).await.expect("seed sidecar");

    download_pre_resolved_fragments(&http, &frags, None, None, None, &output, None, None)
        .await
        .expect("resume truncates torn tail then completes");
    let written = tokio::fs::read(&output).await.unwrap();
    assert_eq!(
        written, reference,
        "torn tail dropped; final byte-identical"
    );
}

#[tokio::test]
async fn hls_completes_when_sidecar_save_always_fails() {
    // Force EVERY sidecar save to fail by pre-creating the sidecar path as a
    // directory: atomic_write_json's rename-onto-a-directory fails each time,
    // while the separate output file writes normally. The download must still
    // complete correctly (resume is best-effort; a save failure is non-fatal).
    let mut server = mockito::Server::new_async().await;
    let mut expected = Vec::new();
    let mut frags = Vec::new();
    for i in 0..4_u32 {
        let body = vec![i as u8; 8];
        expected.extend_from_slice(&body);
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_body(body)
            .create_async()
            .await;
        frags.push(frag(format!("{}/seg-{i}.ts", server.url())));
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("video.ts");
    // Use the production sidecar-path helper so this test can't silently pass
    // for the wrong reason if the suffix scheme changes.
    let sidecar = super::state_path(&output);
    tokio::fs::create_dir(&sidecar)
        .await
        .expect("mkdir sidecar-as-dir");

    let http = HttpDownloader::with_client(wreq::Client::new()).with_concurrent_fragments(1);
    let stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, &output, None, None)
            .await
            .expect("download must complete despite every sidecar save failing");

    assert_eq!(stats.bytes_downloaded, 32, "all 4×8 bytes downloaded");
    let written = tokio::fs::read(&output).await.unwrap();
    assert_eq!(written, expected, "output bytes correct under save failure");
    // The sidecar path is still the directory we created — never overwritten,
    // and remove_file on a directory is a no-op error swallowed by `let _`.
    assert!(
        tokio::fs::metadata(&sidecar)
            .await
            .expect("still exists")
            .is_dir(),
        "sidecar path remained a directory; no partial/torn sidecar written"
    );
}

// --- extrapolate_total: byte-extrapolated total estimate (issue #382) ---

#[test]
fn extrapolate_total_returns_expected_when_content_length_known() {
    // A real Content-Length total bypasses the extrapolation entirely.
    assert_eq!(
        extrapolate_total(Some(1000), 250, 10, 1),
        Some(1000),
        "expected_total present: return it verbatim, ignore the estimate"
    );
}

#[test]
fn extrapolate_total_estimates_from_average_fragment_size() {
    // 200 bytes over 2 of 8 fragments => avg 100/frag => 100 * 8 = 800.
    assert_eq!(
        extrapolate_total(None, 200, 8, 2),
        Some(800),
        "no expected_total: estimate = total_bytes * total_frags / frags_done"
    );
}

#[test]
fn extrapolate_total_none_when_no_fragments_done() {
    // frags_done == 0: nothing to extrapolate from; honest indeterminate.
    assert_eq!(
        extrapolate_total(None, 0, 8, 0),
        None,
        "frags_done == 0: estimate is unknown, must be None (not a div-by-zero)"
    );
}

#[test]
fn extrapolate_total_none_on_multiplication_overflow() {
    // total_bytes * total_frags overflows u64 => None (indeterminate),
    // NOT a saturated u64::MAX/frags_done nonsensical near-0% estimate.
    let est = extrapolate_total(None, u64::MAX / 2 + 1, 2, 1);
    assert_eq!(
        est, None,
        "overflowing extrapolation must yield None, per the \"None when unknown\" principle"
    );
}

// --- fetch_with_optional_range: ranged-fragment response validation (issue #564) ---
//
// #526 fixed the parallel-chunk path (crates/rdlp-downloader/src/http/mod.rs) by
// validating a ranged response's status/Content-Range/length BEFORE any bytes
// land in the merged output's chunk slot. `fetch_with_optional_range` sends the
// same `Range` header for HLS BYTERANGE / DASH mediaRange fragments but only
// gated on `resp.status().is_success()` — a server that ignores Range (legal
// per RFC 9110 §14.2) replies 200 with the WHOLE resource, and the
// unvalidated body is written into a slot sized for one fragment.
//
// These tests drive the fix through the fragment-list entry point
// (`download_pre_resolved_fragments`) rather than calling the private
// `fetch_with_optional_range` directly, so they exercise the exact code path
// #564 describes end to end.

fn ranged_frag(url: String, start: u64, end_exclusive: u64) -> Fragment {
    Fragment {
        url,
        byte_range: Some((start, end_exclusive)),
        init_url: None,
        init_byte_range: None,
        duration: Some(6.0),
        filesize: None,
    }
}

#[tokio::test]
async fn ranged_fragment_200_full_body_is_rejected() {
    // Server ignores Range (RFC 9110 §14.2) and answers 200 with the WHOLE
    // resource — far larger than the requested 1024-byte span. Accepting
    // this silently writes the whole file into the fragment's slot.
    let mut server = mockito::Server::new_async().await;
    let _seg = server
        .mock("GET", "/seg.m4s")
        .with_status(200)
        .with_body(vec![0xAA; 8192])
        .create_async()
        .await;

    let url = format!("{}/seg.m4s", server.url());
    let frags = vec![ranged_frag(url, 1024, 2048)];

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let res =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await;
    let err = res.expect_err(
        "a plain 200 answering a ranged request must be rejected, not accepted as the fragment body",
    );
    // Discriminate: without this the test would also pass if the download
    // failed for an unrelated reason, making it a much weaker guard.
    let msg = err.to_string();
    assert!(
        msg.contains("got HTTP 200") && msg.contains("expected 206"),
        "must fail on the status check specifically, got: {msg}"
    );
}

#[tokio::test]
async fn ranged_fragment_content_range_wrong_span_is_rejected() {
    // 206 + Content-Range present, but it encloses a DIFFERENT span than the
    // one requested (bytes=1024-2047 requested; server claims bytes=0-1023).
    let mut server = mockito::Server::new_async().await;
    let _seg = server
        .mock("GET", "/seg.m4s")
        .with_status(206)
        .with_header("Content-Range", "bytes 0-1023/8192")
        .with_body(vec![0xBB; 1024])
        .create_async()
        .await;

    let url = format!("{}/seg.m4s", server.url());
    let frags = vec![ranged_frag(url, 1024, 2048)];

    // Retries off: a wrong span is retryable (#570), and this test is about
    // the rejection itself, not the retry loop.
    let http = no_retry_http();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let res =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await;
    let err = res.expect_err(
        "a 206 whose Content-Range encloses a different span than requested must be rejected",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("Content-Range bytes 0-1023") && msg.contains("different span"),
        "must fail on the span-equality check specifically, got: {msg}"
    );
}

#[tokio::test]
async fn ranged_fragment_short_body_is_rejected() {
    // 206 + correct Content-Range (1024-2047, 1024 bytes) but the body is
    // shorter than promised.
    let mut server = mockito::Server::new_async().await;
    let _seg = server
        .mock("GET", "/seg.m4s")
        .with_status(206)
        .with_header("Content-Range", "bytes 1024-2047/8192")
        .with_body(vec![0xCC; 512])
        .create_async()
        .await;

    let url = format!("{}/seg.m4s", server.url());
    let frags = vec![ranged_frag(url, 1024, 2048)];

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let res =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await;
    let err = res.expect_err(
        "a 206 whose body is shorter than its own Content-Range promise must be rejected",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("delivered 512 bytes") && msg.contains("expected exactly 1024"),
        "must fail on the body-length check specifically, and name both counts, got: {msg}"
    );
}

#[tokio::test]
async fn ranged_fragment_over_long_body_is_rejected() {
    // 206 + correct Content-Range (1024-2047, 1024 bytes) but the body is
    // longer than promised.
    let mut server = mockito::Server::new_async().await;
    let _seg = server
        .mock("GET", "/seg.m4s")
        .with_status(206)
        .with_header("Content-Range", "bytes 1024-2047/8192")
        .with_body(vec![0xDD; 2048])
        .create_async()
        .await;

    let url = format!("{}/seg.m4s", server.url());
    let frags = vec![ranged_frag(url, 1024, 2048)];

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let res =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await;
    let err = res.expect_err(
        "a 206 whose body is longer than its own Content-Range promise must be rejected",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("delivered 2048 bytes") && msg.contains("expected exactly 1024"),
        "must fail on the body-length check specifically, and name both counts, got: {msg}"
    );
}

#[tokio::test]
async fn ranged_fragment_correct_206_is_accepted() {
    // Positive: 206 + matching Content-Range + exact-length body succeeds and
    // the exact bytes land in the output.
    let mut server = mockito::Server::new_async().await;
    let body = vec![0xEE; 1024];
    let _seg = server
        .mock("GET", "/seg.m4s")
        .with_status(206)
        .with_header("Content-Range", "bytes 1024-2047/8192")
        .with_body(body.clone())
        .create_async()
        .await;

    let url = format!("{}/seg.m4s", server.url());
    let frags = vec![ranged_frag(url, 1024, 2048)];

    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("a correct 206 + matching Content-Range + exact body must succeed");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(written, body, "output must be exactly the validated span");
}

#[tokio::test]
async fn unranged_fragment_plain_200_still_succeeds() {
    // Regression guard: a fragment fetch with byte_range: None must keep
    // today's behavior — a plain 200 is correct there and must NOT be
    // required to be 206. This pins that the #564 fix does not require
    // Partial Content on the no-Range path.
    let mut server = mockito::Server::new_async().await;
    let body = vec![0x11; 100];
    let _f1 = server
        .mock("GET", "/f1")
        .with_status(200)
        .with_body(body.clone())
        .create_async()
        .await;

    let frags = vec![frag(format!("{}/f1", server.url()))];
    let http = HttpDownloader::with_client(wreq::Client::new());
    let tmp = tempfile::NamedTempFile::new().unwrap();
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("unranged fragment fetch must still succeed on a plain 200");
    let written = tokio::fs::read(tmp.path()).await.unwrap();
    assert_eq!(written, body);
}

// --- fragment retry (issue #570) ---
//
// The parallel-chunk path (`download_chunk_with_retry`) and the DASH segment
// path (`dash/download.rs::download_one`) both retry transient failures; the
// fragment path did not, so one bad edge node killed a whole multi-hundred-
// fragment HLS download. These tests pin the retry semantics: retryable
// failures are re-fetched, non-retryable ones fail on the first response.

/// `HttpDownloader` with a millisecond retry backoff, so retry-path tests
/// exercise the real loop without sleeping: `RetryConfig::default_config`
/// starts at 1s and doubles to 60s, which would turn a millisecond test into
/// a five-minute one.
fn http_with_retries(max_retries: usize) -> HttpDownloader {
    let config = crate::retry::test_retry_config(max_retries);
    // Both policies: the fragment path reads `fragment_retry_config`, and the
    // range request underneath it reads `retry_config`. Setting only one
    // leaves the other at the 10-attempt / 60s-ceiling default, which is
    // minutes per failing test.
    HttpDownloader::with_client(wreq::Client::new())
        .with_retry_config(config.clone())
        .with_fragment_retry_config(config)
}

/// Retries enabled, concurrency pinned to 1 so retry accounting is
/// deterministic across fragments.
fn retrying_http(max_retries: usize) -> HttpDownloader {
    http_with_retries(max_retries).with_concurrent_fragments(1)
}

/// Retries disabled, for the tests that assert a *failure* on a retryable
/// error and are not themselves about the retry loop.
fn no_retry_http() -> HttpDownloader {
    http_with_retries(0)
}

#[tokio::test]
async fn retryable_fragment_failure_is_retried_then_succeeds() {
    let mut server = mockito::Server::new_async().await;
    // Measured against mockito 1.7.2: mocks are matched in CREATION order and
    // a mock is retired once its `expect(n)` count is met. So creating the 500
    // first and the success second makes the response sequence exactly
    // 500, then 200. (`assert_async` on both below proves each was consumed —
    // without it a single-fetch success would pass this test vacuously.)
    let body = vec![0x5A; 128];
    let fail = server
        .mock("GET", "/f1")
        .with_status(500)
        .expect(1)
        .create_async()
        .await;
    let ok = server
        .mock("GET", "/f1")
        .with_body(body.clone())
        .expect(1)
        .create_async()
        .await;

    let frags = vec![frag(format!("{}/f1", server.url()))];
    let http = retrying_http(3);
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let stats =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect("a transient 500 on one fragment must be retried, not fail the download");
    assert_eq!(
        stats.retries, 1,
        "the reported retry count is what the operator sees; one 500 is one retry"
    );

    let written = tokio::fs::read(tmp.path()).await.expect("read output");
    assert_eq!(
        written, body,
        "the retried fetch's bytes must be what lands in the output"
    );
    fail.assert_async().await;
    ok.assert_async().await;
}

#[tokio::test]
async fn non_retryable_fragment_status_fails_without_retrying() {
    let mut server = mockito::Server::new_async().await;
    // 404 is not retryable. `expect(1)` fails the assertion below if a second
    // request arrives, which is the guard against a retry storm on a dead URL.
    let mock = server
        .mock("GET", "/f1")
        .with_status(404)
        .expect(1)
        .create_async()
        .await;

    let frags = vec![frag(format!("{}/f1", server.url()))];
    let http = retrying_http(3);
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let err =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect_err("a 404 fragment must fail the download");
    assert!(
        matches!(err, rdlp_core::RdlpError::Http { status: 404, .. }),
        "unexpected err shape: {err:?}"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn ranged_fragment_wrong_span_is_retried_then_succeeds() {
    // #564 deliberately reports a wrong Content-Range span as a *retryable*
    // `Network` error, reasoning that a retry against another CDN node
    // plausibly gets the right bytes. That distinction was inert on this path
    // until #570 — this test is what makes it load-bearing.
    let mut server = mockito::Server::new_async().await;
    let body = vec![0xEE; 1024];
    // Creation order is match order (see the note in
    // `retryable_fragment_failure_is_retried_then_succeeds`): wrong span first,
    // correct span second.
    let wrong_span = server
        .mock("GET", "/seg.m4s")
        .with_status(206)
        .with_header("Content-Range", "bytes 0-1023/8192")
        .with_body(vec![0xBB; 1024])
        .expect(1)
        .create_async()
        .await;
    let ok = server
        .mock("GET", "/seg.m4s")
        .with_status(206)
        .with_header("Content-Range", "bytes 1024-2047/8192")
        .with_body(body.clone())
        .expect(1)
        .create_async()
        .await;

    let frags = vec![ranged_frag(format!("{}/seg.m4s", server.url()), 1024, 2048)];
    let http = retrying_http(3);
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
        .await
        .expect("a wrong-span 206 is retryable and the retry delivers the right span");

    let written = tokio::fs::read(tmp.path()).await.expect("read output");
    assert_eq!(
        written, body,
        "only the correctly-spanned body may land in the output"
    );
    wrong_span.assert_async().await;
    ok.assert_async().await;
}

#[tokio::test]
async fn retry_budget_bounds_cumulative_retries_across_the_fragment_list() {
    // Per-fragment retries do not bound a *flaky* list: a fragment that fails
    // and then succeeds never propagates an error, so nothing stops a long
    // list from spending a backoff ladder on every one of its fragments. (A
    // wholly broken origin is already bounded — the download returns on the
    // first fragment to exhaust its allowance.) The list-wide budget caps the
    // cumulative total.
    //
    // 20 fragments with `max_retries = 1` sizes the budget at
    // 1 x (20 / RETRY_BUDGET_FRAGMENT_SHARE) = 2 retries for the whole list.
    // Each fragment answers 500 once, then its real body. So fragments 1 and 2
    // spend the budget and complete; fragment 3's failure finds it empty, is
    // not retried, and fails the download.
    const TOTAL: usize = 20;
    let mut server = mockito::Server::new_async().await;
    let body = vec![0x77; 32];
    for i in 1..=TOTAL {
        // Creation order is match order: the 500 first, the body second.
        server
            .mock("GET", format!("/f{i}").as_str())
            .with_status(500)
            .expect(1)
            .create_async()
            .await;
        server
            .mock("GET", format!("/f{i}").as_str())
            .with_body(body.clone())
            .create_async()
            .await;
    }

    let frags: Vec<Fragment> = (1..=TOTAL)
        .map(|i| frag(format!("{}/f{i}", server.url())))
        .collect();
    let http = retrying_http(1);
    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    let err =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect_err("the third fragment must find the retry budget spent");
    assert!(
        matches!(err, rdlp_core::RdlpError::Http { status: 500, .. }),
        "unexpected err shape: {err:?}"
    );

    let written = tokio::fs::read(tmp.path()).await.expect("read output");
    assert_eq!(
        written.len(),
        body.len() * 2,
        "exactly two fragments may be rescued by a budget of two retries"
    );
}

#[tokio::test]
async fn a_stalled_fragment_fetch_times_out_rather_than_hanging() {
    // The failure mode the fragment path could not see before it carried a
    // request timeout: a CDN that accepts the connection and then sends
    // nothing. No error is ever produced, so #570's retry cannot help — and
    // without a cancellation token there is nothing else to end the wait.
    // mockito cannot express this (it has no delay API), so this is a raw
    // listener that accepts and holds.
    // Bound but never accepted: the kernel completes the handshake into the
    // backlog, so the client connects successfully and then waits forever for
    // a response that no one will write. `listener` must stay in scope for the
    // whole test — dropping it closes the port, and the connect would then
    // fail fast instead of stalling, which is not what this test is about.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let http = http_with_retries(0).with_read_timeout(std::time::Duration::from_millis(100));
    let frags = vec![frag(format!("http://{addr}/stalled.ts"))];
    let tmp = tempfile::NamedTempFile::new().expect("tmp");

    let started = Instant::now();
    let err =
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None, None)
            .await
            .expect_err("a stalled fragment must fail, not hang");
    let elapsed = started.elapsed();

    assert!(
        matches!(err, rdlp_core::RdlpError::Network { .. }),
        "a timeout is a transient network failure, so it stays retryable: {err:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "returned only after {elapsed:?}; the fetch was not bounded by read_timeout"
    );
}
