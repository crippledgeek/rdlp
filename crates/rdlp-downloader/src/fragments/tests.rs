use super::*;
use mockito::Matcher;
use rdlp_types::Fragment;

#[tokio::test]
async fn byte_range_emits_http_range_header() {
    let mut server = mockito::Server::new_async().await;
    let _seg = server
        .mock("GET", "/seg.m4s")
        .match_header("Range", Matcher::Regex(r"^bytes=1024-2047$".to_string()))
        .with_body(b"X".repeat(1024))
        .expect(1)
        .create_async()
        .await;

    // Catch-all 501 if Range header is missing or wrong.
    let _unmatched = server
        .mock("GET", Matcher::Any)
        .with_status(501)
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
async fn fragment_progress_expected_total_none_uses_segment_fraction() {
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
        None,
        Some(&cb),
        tmp.path(),
        None,
        None,
    )
    .await
    .expect("ok");
    let evs = cb.events();
    assert!(!evs.is_empty());
    // No byte total is known (HLS), so `total_bytes` stays None — but `progress`
    // is now the SEGMENT fraction so progress bars animate instead of jumping
    // 0->100 at completion.
    assert!(
        evs.iter().all(|e| e.total_bytes.is_none()),
        "byte total must remain unknown when expected_total is None"
    );
    assert!(
        evs.iter().all(|e| e.progress.is_some()),
        "progress must be the segment-based fraction, not None"
    );
    let last = evs.last().expect("at least one event");
    assert_eq!(last.segments_downloaded, Some(1));
    assert_eq!(last.total_segments, Some(1));
    let frac = last.progress.expect("final progress some").fraction();
    assert!((frac - 1.0).abs() < 1e-6, "final event = 1/1 segments");
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
    let http = HttpDownloader::with_client(wreq::Client::new());
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
/// Referer. The catch-all 501 on `init_server` fires if the header leaks.
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

    // Catch-all on init_server: 501 if Referer arrived unexpectedly.
    let _init_catchall = init_server
        .mock("GET", Matcher::Any)
        .with_status(501)
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
/// Both must receive the Referer. The catch-all 501 fires if headers were
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

    // Catch-all: 501 if Referer was NOT present (headers were stripped).
    let _catchall = format_server
        .mock("GET", Matcher::Any)
        .with_status(501)
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
/// 501 on `cross_server` fires if the Referer was forwarded.
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

    // Catch-all on cross_server: 501 if Referer arrived.
    let _catchall = cross_server
        .mock("GET", Matcher::Any)
        .with_status(501)
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

// ── Progress fraction: byte-based when total known, segment-based otherwise ──
//
// HLS pre-resolved-fragment downloads pass `expected_total = None` (segment-
// based progress), so the byte fraction is unavailable. Without a segment
// fallback the emitted `progress` is `None` and UIs that read it (the desktop
// bar, `events.rs`) sit at 0 then jump to 100. These guard the fallback.

#[test]
fn fragment_progress_fraction_prefers_byte_total() {
    // Byte total known → byte-based fraction (progressive / sized HLS).
    let p = fragment_progress_fraction(Some(1000), 500, 1, 4).expect("some");
    assert!((p.fraction() - 0.5).abs() < 1e-6, "byte fraction 500/1000");
}

#[test]
fn fragment_progress_fraction_falls_back_to_segments() {
    // Byte total unknown (HLS) → segment-based fraction (the 0->100 jump fix).
    let mid = fragment_progress_fraction(None, 0, 1, 4).expect("some");
    assert!((mid.fraction() - 0.25).abs() < 1e-6, "segment fraction 1/4");
    let done = fragment_progress_fraction(None, 12_345, 4, 4).expect("some");
    assert!((done.fraction() - 1.0).abs() < 1e-6, "segment fraction 4/4");
}

#[test]
fn fragment_progress_fraction_none_when_nothing_known() {
    // No byte total and zero segments → no fraction to report.
    assert!(fragment_progress_fraction(None, 0, 0, 0).is_none());
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
    // but the already-done fragments are 501 — if resume re-fetches them the
    // download errors, proving they were skipped. Remaining fragments serve
    // their real bodies.
    let mut server = mockito::Server::new_async().await;
    for i in 0..done {
        server
            .mock("GET", format!("/seg-{i}.ts").as_str())
            .with_status(501)
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
        .expect("resume must succeed without hitting the 501 done-fragments");

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
            .with_status(501)
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
