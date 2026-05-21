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
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt as _;
use rdlp_core::{DownloadProgress, DownloadStats, ProgressCallback, Result};
use rdlp_types::{Fragment, Progress};
use tokio::io::AsyncWriteExt as _;
use tokio_util::sync::CancellationToken;

use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};
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
/// Under parallel mode (concurrency > 1), at most `concurrent_fragments - 1`
/// extra fragments may have completed in-flight before the post-write cancel
/// check fires; their bytes will appear in the partial output. This is the
/// price of parallelism — sequential mode (`concurrent_fragments = 1`)
/// preserves the strict "at most 1 extra fragment after cancel" semantic.
///
/// # Same-origin header gate
///
/// `Format.http_headers` (Referer, Cookie, Authorization, Origin) are forwarded
/// only when the target URL's origin (scheme + host + port, per RFC 6454) matches
/// `format_url`'s origin. Cross-origin fetches receive no operator headers,
/// preventing header exfiltration via a compromised or malicious manifest whose
/// init / fragment URIs point to an attacker-controlled CDN.
///
/// When `format_url` is `None` or unparseable, ALL fetches are treated as
/// cross-origin (fail-closed).
///
/// # Errors
///
/// Returns `RdlpError::Download` if any fragment fetch fails, if a URL fails
/// to resolve against the base URL, or if the output file cannot be created.
/// Returns `RdlpError::Cancelled` if `cancel` fires.
// 102/100 lines after the cancellation wrapper additions; splitting would
// obscure the loop's bookkeeping flow.
#[allow(clippy::too_many_lines)]
// 8 args: the extra `format_url` param for the same-origin header gate (#273)
// pushes us one over clippy's default limit of 7.
#[allow(clippy::too_many_arguments)]
pub async fn download_pre_resolved_fragments(
    http: &HttpDownloader,
    fragments: &[Fragment],
    base_url: Option<&str>,
    expected_total: Option<u64>,
    progress: Option<&dyn ProgressCallback>,
    output: &Path,
    format_url: Option<&str>,
    cancel: Option<&CancellationToken>,
) -> Result<DownloadStats> {
    let started = Instant::now();

    // Parse the format origin once. Used by each fragment task to decide
    // whether to forward `http.headers()` to that fetch. Fails closed:
    // `None` (no format_url) or `Some(Opaque(...))` (non-tuple-origin scheme)
    // both compare not-equal to any target origin, so headers are dropped.
    let format_origin: Option<url::Origin> = format_url
        .and_then(|u| url::Url::parse(u).ok())
        .map(|u| u.origin());

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

    // Build the AIMD adaptive controller in HlsSegments mode.
    // `HlsSegments` skips chunk-level adjustments (segment sizes are
    // server-determined) and tunes only connection count via the semaphore.
    // Mirrors dash/download.rs:389-399 line-for-line.
    //
    // `log_callback` is `None` here (same as dash/download.rs:393) because
    // `progress` is `Option<&dyn ProgressCallback>` (borrowed, non-`'static`)
    // while `AdaptiveController::log_callback` requires `Arc<dyn ProgressCallback + 'static>`.
    // AIMD log messages are still emitted via the `log` crate's `info!()` macro
    // inside `AdaptiveController::new` regardless of `log_callback`.
    let concurrency = http.concurrent_fragments().max(1);

    // total_size = 0: not meaningful for segment-based downloads (mirrors
    // dash/download.rs:390). HlsSegments mode skips chunk-level adjustments
    // anyway, so the value is unused; passing 0 keeps parity with DASH.
    let controller = Arc::new(AdaptiveController::new(
        0,
        AdaptiveConfig {
            max_connections: concurrency,
            initial_connections: concurrency.min(2),
            ..AdaptiveConfig::default()
        },
        ControllerMode::HlsSegments,
        None,
    ));
    let sem = controller.semaphore().clone();

    // Determine per-fragment init-fetch needs in a single linear pass over the
    // source list (cheap clone of Option<String>). Only the first fragment of
    // each new init-group fetches the init segment; consecutive fragments under
    // the same init URL skip the fetch. This is the dedup logic that previously
    // lived in the sequential loop's `current_init` variable — we must compute
    // it eagerly because each parallel task cannot share mutable state.
    //
    // SSRF validation runs at the producing layer:
    // - DASH: `crates/rdlp-extractor/src/base/common/dash/expand.rs::validate_resolved_url`
    //   validates `<BaseURL>` chain + each emitted fragment URL during
    //   `expand_dash_representations` (closes #290).
    // - HLS:  `crates/rdlp-extractor/src/hls/expand.rs::validate_resolved_url`
    //   validates each variant + segment URI during master/media playlist
    //   expansion.
    // Per-fragment validation here would be redundant for built-in extractor
    // output; plugin-emitted fragments that bypass both expanders are tracked
    // as a separate sandbox-trust concern in the plugin runtime.
    let mut last_init: Option<String> = None;
    let tasks: Vec<(Fragment, bool)> = fragments
        .iter()
        .map(|frag| {
            let needs_init =
                frag.init_url.is_some() && frag.init_url.as_deref() != last_init.as_deref();
            if needs_init {
                last_init.clone_from(&frag.init_url);
            } else if frag.init_url.is_none() {
                last_init = None;
            }
            (frag.clone(), needs_init)
        })
        .collect();

    // Build a stream of per-fragment fetch futures. `buffered(concurrency)`
    // polls up to N concurrently AND yields results in source order, which is
    // the load-bearing property: writes happen in the same order as `fragments`
    // even though fetches overlap. Each task emits init-bytes (when needed)
    // immediately preceding fragment-bytes in a single Vec<u8>, so RFC 8216
    // §4.3.2.5 decode-ordering is preserved without a group-by-init pre-pass.
    let stream = futures::stream::iter(tasks.into_iter().map(|(frag, needs_init)| {
        let sem = Arc::clone(&sem);
        let base = base_url.map(str::to_string);
        let cancel = cancel.cloned();
        // Clone the parsed origin so each parallel task owns a copy.
        // `url::Origin` is Clone (it is an enum of copyable fields).
        let task_origin = format_origin.clone();
        async move {
            let _permit =
                sem.acquire_owned()
                    .await
                    .map_err(|_| rdlp_core::RdlpError::Download {
                        message: "fragment semaphore closed".to_string(),
                        url: None,
                    })?;

            let fetch_start = Instant::now();
            let mut out: Vec<u8> = Vec::new();

            // Init bytes (if this fragment introduces a new init group).
            if needs_init && let Some(init_url) = &frag.init_url {
                let resolved_init = resolve_fragment_url(init_url, base.as_deref())?;
                let init_bytes = fetch_with_optional_cancel(
                    http,
                    &resolved_init,
                    frag.init_byte_range,
                    task_origin.as_ref(),
                    cancel.as_ref(),
                )
                .await?;
                out.extend_from_slice(&init_bytes);
            }

            // Fragment bytes.
            let resolved_url = resolve_fragment_url(&frag.url, base.as_deref())?;
            let bytes = fetch_with_optional_cancel(
                http,
                &resolved_url,
                frag.byte_range,
                task_origin.as_ref(),
                cancel.as_ref(),
            )
            .await?;
            out.extend_from_slice(&bytes);

            let fetch_elapsed = fetch_start.elapsed();
            Ok::<(Vec<u8>, std::time::Duration, Option<f64>), rdlp_core::RdlpError>((
                out,
                fetch_elapsed,
                frag.duration,
            ))
        }
    }))
    .buffered(concurrency);

    tokio::pin!(stream);
    while let Some(item) = stream.next().await {
        // Flush before propagating any per-fragment Err. `tokio::fs::File`'s
        // `write_all` schedules the actual write on a blocking thread and
        // returns before the bytes reach the kernel; without an explicit
        // flush, returning Err drops `out_file` synchronously and abandons
        // the in-flight spawn_blocking handle — so previously-written
        // fragment bytes can disappear from the partial output. The success
        // path already flushes after the loop (see end of function); the
        // cancellation path already flushes before its own return.
        // See tokio::fs module docs ("calls to write will return before the
        // write has finished; flush will wait for the write to finish").
        let (bytes, fetch_elapsed, seg_dur) = match item {
            Ok(v) => v,
            Err(e) => {
                out_file.flush().await.ok();
                return Err(e);
            }
        };

        if let Err(e) = out_file.write_all(&bytes).await {
            out_file.flush().await.ok();
            return Err(rdlp_core::RdlpError::Download {
                message: format!("write fragment: {e}"),
                url: Some(output.display().to_string()),
            });
        }

        total_bytes += bytes.len() as u64;

        // Inform the AIMD controller of segment completion so it can tune
        // the connection count. Mirrors dash/download.rs:426.
        controller.report_segment_complete(bytes.len() as u64, fetch_elapsed, seg_dur);

        // Update progress accounting.
        let now = Instant::now();
        let elapsed_observe = now.duration_since(last_observed_at);
        speed.observe(bytes.len() as u64, elapsed_observe);
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

        // Cancellation check between fragment writes. Per-fetch cancel is
        // already wired inside fetch_with_optional_cancel; this catches
        // cancels that fire after a fetch completes but before the next poll.
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
///
/// `format_origin` is forwarded to `fetch_with_optional_range` for the
/// same-origin header gate.
async fn fetch_with_optional_cancel(
    http: &HttpDownloader,
    url: &str,
    byte_range: Option<(u64, u64)>,
    format_origin: Option<&url::Origin>,
    cancel: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    match cancel {
        Some(token) => tokio::select! {
            biased;
            () = token.cancelled() => Err(rdlp_core::RdlpError::Cancelled),
            res = fetch_with_optional_range(http, url, byte_range, format_origin) => res,
        },
        None => fetch_with_optional_range(http, url, byte_range, format_origin).await,
    }
}

/// Fetch `url`, optionally as an HTTP byte range.
///
/// The `byte_range` tuple is `(start, end_exclusive)` and is converted to RFC 7233
/// `Range: bytes=start-end_inclusive` (subtract 1 for HTTP's inclusive end).
///
/// `format_origin` enforces the same-origin header gate: `http.headers()` are
/// forwarded only when `url`'s origin (scheme + host + port) matches `format_origin`.
/// Fails closed — `None` `format_origin`, opaque origin, or parse failure all
/// result in no operator headers being forwarded.
async fn fetch_with_optional_range(
    http: &HttpDownloader,
    url: &str,
    byte_range: Option<(u64, u64)>,
    format_origin: Option<&url::Origin>,
) -> Result<Vec<u8>> {
    use wreq::header::HeaderValue;

    let safe_url = rdlp_security::sanitize_for_logging(url);

    // Same-origin gate: forward operator headers only when the target origin
    // matches the format origin. Fails closed on any parse failure or opaque origin.
    let same_origin = match (format_origin, url::Url::parse(url).ok()) {
        (Some(seed), Some(target)) => *seed == target.origin(),
        _ => false,
    };
    let mut req = http.client().get(url);
    if same_origin {
        req = req.headers(http.headers());
    }
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
        let stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
        let res = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
        let stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
    /// 2. The download completes correctly with AIMD-constrained concurrency
    ///    (`initial_connections=1`) — a regression guard that AIMD wiring does
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
                initial_connections: 1,
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
        let stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
    async fn build_ordered_frags(
        server: &mut mockito::Server,
        n: usize,
    ) -> (Vec<Fragment>, Vec<u8>) {
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
        let _stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
        let _stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
        let _stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
        let _stats = download_pre_resolved_fragments(
            &http,
            &frags,
            None,
            None,
            None,
            tmp.path(),
            None,
            None,
        )
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
}
