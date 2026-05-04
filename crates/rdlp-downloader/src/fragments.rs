//! Shared fragment-list downloader for pre-resolved segment URLs.
//!
//! Used by both DASH (when the extractor expanded an MPD into per-Representation
//! segment lists via `expand_dash_representations`) and HLS (when the extractor
//! pre-resolved segment URLs into `Format.fragments`). Fragments are written
//! sequentially to the output file — no intermediate files or `FFmpeg` mux step
//! is required because the extractor already resolved each stream into a
//! separate `Format` entry.

// `Duration::from_mins` (lint's suggested replacement) needs Rust 1.95;
// workspace MSRV is 1.85.
#![allow(clippy::duration_suboptimal_units)]

use std::path::Path;
use std::time::Instant;

use rdlp_core::{DownloadProgress, DownloadStats, ProgressCallback, Result};
use rdlp_types::{Fragment, Progress};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::http::HttpDownloader;
use crate::progress::SpeedTracker;
use rdlp_security;

/// Fetch a pre-resolved list of fragment URLs and concatenate them into
/// `output` in order.
///
/// Each URL is resolved against the optional `base_url`. Fragments are written
/// sequentially — no intermediate files or `FFmpeg` mux step is required.
///
/// # Progress
///
/// When `progress` is `Some`, emits `DownloadProgress` events on a 100ms
/// throttle plus a forced emit at the final-fragment boundary. When
/// `expected_total` is `None`, emitted events carry `total_bytes = None`
/// and `progress = None`; consumers see fragment-count progress only
/// (`segments_downloaded` / `total_segments`).
///
/// # Cancellation
///
/// When `cancel` is `Some`, the helper checks `is_cancelled()` pre-loop
/// and after each fragment write, and races each fragment fetch against
/// `cancelled()`. On cancel: flushes the partial output and returns
/// `Err(RdlpError::Cancelled)`.
///
/// # Errors
///
/// Returns `RdlpError::Download` if any fragment fetch fails, if a URL fails
/// to resolve against the base URL, or if the output file cannot be created.
/// Returns `RdlpError::Cancelled` if `cancel` fires.
// 102/100 lines after the cancellation wrapper additions; splitting would
// obscure the loop's bookkeeping flow.
#[allow(clippy::too_many_lines)]
pub async fn download_pre_resolved_fragments(
    http: &HttpDownloader,
    fragments: &[Fragment],
    base_url: Option<&str>,
    expected_total: Option<u64>,
    progress: Option<&dyn ProgressCallback>,
    output: &Path,
    cancel: Option<&CancellationToken>,
) -> Result<DownloadStats> {
    let started = Instant::now();

    let mut out_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)
        .await
        .map_err(|e| rdlp_core::RdlpError::Download {
            message: format!("create output: {e}"),
            url: Some(output.display().to_string()),
        })?;

    let mut total_bytes: u64 = 0;
    let mut current_init: Option<String> = None;

    const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
    let mut last_emit = Instant::now();
    let mut last_observed_at = Instant::now();
    let mut frags_done: u64 = 0;
    let total_frags = fragments.len() as u64;
    let mut speed = SpeedTracker::new();

    // Pre-loop cancellation check.
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        out_file.flush().await.ok();
        return Err(rdlp_core::RdlpError::Cancelled);
    }

    for frag in fragments {
        let resolved_url = resolve_fragment_url(&frag.url, base_url)?;

        // NOTE: per-fragment SSRF validation is intentionally NOT performed
        // here. It mirrors the legacy MPD-URL path, which also fetches
        // segments without per-segment gating. The orchestrator validates
        // `format.url` at the format-dispatch boundary
        // (`crates/rdlp-api/src/orchestrator/download.rs`), and fragment
        // URLs SHOULD be validated at extract time inside
        // `expand_dash_representations` (TODO: track as a follow-up — the
        // hardened defence-in-depth gate belongs at extraction, where the
        // URLs are first introduced into the Format). HLS `merge.rs:184`
        // is the outlier that inlines the gate; we don't replicate that
        // pattern because it forces every test to use a public-routable
        // mock host.

        // Init transition: refetch only when the URI changes between consecutive fragments.
        if frag.init_url.as_deref() != current_init.as_deref() {
            if let Some(init_url) = &frag.init_url {
                let resolved_init = resolve_fragment_url(init_url, base_url)?;
                let init_bytes =
                    fetch_with_optional_cancel(http, &resolved_init, frag.init_byte_range, cancel)
                        .await?;
                total_bytes += init_bytes.len() as u64;
                out_file.write_all(&init_bytes).await.map_err(|e| {
                    rdlp_core::RdlpError::Download {
                        message: format!("write init fragment: {e}"),
                        url: Some(output.display().to_string()),
                    }
                })?;
                current_init = Some(init_url.clone());
            } else {
                current_init = None;
            }
        }

        let bytes =
            fetch_with_optional_cancel(http, &resolved_url, frag.byte_range, cancel).await?;

        out_file
            .write_all(&bytes)
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("write fragment: {e}"),
                url: Some(output.display().to_string()),
            })?;

        total_bytes += bytes.len() as u64;

        // Update progress accounting.
        let now = Instant::now();
        let elapsed = now.duration_since(last_observed_at);
        speed.observe(bytes.len() as u64, elapsed);
        last_observed_at = now;
        frags_done += 1;

        // Emit progress (100ms throttle OR fragment-N boundary).
        if let Some(cb) = progress
            && (now.duration_since(last_emit) >= PROGRESS_INTERVAL || frags_done == total_frags)
        {
            cb.on_progress(&DownloadProgress {
                bytes_downloaded: total_bytes,
                total_bytes: expected_total,
                progress: expected_total.map(|t| Progress::from_ratio(total_bytes, t)),
                segments_downloaded: Some(frags_done),
                total_segments: Some(total_frags),
                speed: speed.bytes_per_sec(),
                eta: speed.eta(expected_total.map(|t| t.saturating_sub(total_bytes))),
                duration_downloaded: None,
                total_duration: None,
            });
            last_emit = now;
        }

        // Cancellation check between fragments.
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            out_file.flush().await.ok();
            return Err(rdlp_core::RdlpError::Cancelled);
        }
    }

    out_file
        .flush()
        .await
        .map_err(|e| rdlp_core::RdlpError::Download {
            message: format!("flush output: {e}"),
            url: Some(output.display().to_string()),
        })?;

    let elapsed = started.elapsed();
    #[allow(clippy::cast_precision_loss)]
    let avg = if elapsed.as_secs_f64() > 0.0 {
        total_bytes as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    Ok(DownloadStats {
        bytes_downloaded: total_bytes,
        duration: elapsed,
        average_speed: avg,
        retries: 0,
        fragments: Some(fragments.len()),
    })
}

/// Resolve a fragment URL against an optional base URL.
///
/// When `base_url` is `Some`, the fragment URL is joined against it (handles
/// relative paths). When `base_url` is `None`, the fragment URL is used as-is
/// (it must be absolute).
pub(crate) fn resolve_fragment_url(fragment_url: &str, base_url: Option<&str>) -> Result<String> {
    match base_url {
        Some(base) => {
            let base_parsed =
                url::Url::parse(base).map_err(|e| rdlp_core::RdlpError::Download {
                    message: format!("invalid fragment_base_url: {e}"),
                    url: Some(base.to_string()),
                })?;
            let resolved =
                base_parsed
                    .join(fragment_url)
                    .map_err(|e| rdlp_core::RdlpError::Download {
                        message: format!("resolve fragment url: {e}"),
                        url: Some(fragment_url.to_string()),
                    })?;
            Ok(resolved.to_string())
        }
        None => Ok(fragment_url.to_string()),
    }
}

/// Fetch a fragment with cooperative cancellation.
///
/// Wraps `fetch_with_optional_range` in `tokio::select!` against the optional
/// cancellation token. Without this, a hung connection (TCP accepted but body
/// never arrives) would block indefinitely between cooperative cancel points
/// — `fetch_with_optional_range` itself has no per-read timeout, and the
/// helper's only `is_cancelled()` checks are between fragments.
async fn fetch_with_optional_cancel(
    http: &HttpDownloader,
    url: &str,
    byte_range: Option<(u64, u64)>,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    match cancel {
        Some(token) => tokio::select! {
            biased;
            () = token.cancelled() => Err(rdlp_core::RdlpError::Cancelled),
            res = fetch_with_optional_range(http, url, byte_range) => res,
        },
        None => fetch_with_optional_range(http, url, byte_range).await,
    }
}

/// Fetch `url`, optionally as an HTTP byte range.
///
/// The `byte_range` tuple is `(start, end_exclusive)` and is converted to RFC 7233
/// `Range: bytes=start-end_inclusive` (subtract 1 for HTTP's inclusive end).
async fn fetch_with_optional_range(
    http: &HttpDownloader,
    url: &str,
    byte_range: Option<(u64, u64)>,
) -> Result<Vec<u8>> {
    use wreq::header::HeaderValue;

    let safe_url = rdlp_security::sanitize_for_logging(url);
    let mut req = http.client().get(url).headers(http.headers());
    if let Some((start, end_exclusive)) = byte_range {
        // Saturating_sub guards against any future caller passing end_exclusive == 0.
        // (Caller responsibility: end_exclusive > start, validated at expand time.)
        let end_inclusive = end_exclusive.saturating_sub(1);
        let value = format!("bytes={start}-{end_inclusive}");
        req = req.header(
            "Range",
            HeaderValue::from_str(&value).map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("fetch {safe_url}: {e}"),
                url: Some(url.to_string()),
            })?,
        );
    }

    let resp = req
        .send()
        .await
        .map_err(|e| rdlp_core::RdlpError::Network {
            message: format!("fetch {safe_url}: {e}"),
            url: Some(url.to_string()),
        })?;

    if !resp.status().is_success() {
        return Err(rdlp_core::RdlpError::Http {
            status: resp.status().as_u16(),
            reason: format!("fragment HTTP {}", resp.status()),
        });
    }

    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| rdlp_core::RdlpError::Network {
            message: format!("read {safe_url}: {e}"),
            url: Some(url.to_string()),
        })
}

#[cfg(test)]
mod tests {
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
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None)
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
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None)
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
            download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None)
                .await
                .expect("ok");
        assert_eq!(stats.bytes_downloaded, 100);
    }

    #[tokio::test]
    async fn fragment_progress_expected_total_none_emits_none_total() {
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
        let _ =
            download_pre_resolved_fragments(&http, &frags, None, None, Some(&cb), tmp.path(), None)
                .await
                .expect("ok");
        let evs = cb.events();
        assert!(!evs.is_empty());
        assert!(
            evs.iter()
                .all(|e| e.total_bytes.is_none() && e.progress.is_none())
        );
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
        let stats =
            download_pre_resolved_fragments(&http, &frags, None, None, Some(&cb), tmp.path(), None)
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
            download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None)
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
        download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None)
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
            download_pre_resolved_fragments(&http, &frags, None, None, None, tmp.path(), None)
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
            Some(&token),
        )
        .await
        .expect("ok");
        assert_eq!(stats.bytes_downloaded, 100);
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
}
