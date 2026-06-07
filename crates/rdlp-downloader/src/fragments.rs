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

use std::io::SeekFrom;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt as _;
use rdlp_core::{DownloadProgress, DownloadStats, ProgressCallback, Result};
use rdlp_types::Fragment;
use tokio::io::{AsyncSeekExt as _, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;

use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};
use crate::atomic::{SIDECAR_SAVE_FAILURE_THRESHOLD, SaveFailureTracker};
use crate::http::HttpDownloader;
use crate::progress::SpeedMeter;
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
/// `expected_total` is `None` (HLS), emitted events carry `total_bytes = None`
/// but `progress` is the **segment-based** fraction (`segments_downloaded /
/// total_segments`) so progress bars animate rather than jump 0->100.
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
/// # Resume
///
/// Fragment-level resume is self-managed via a `<output>.hls_state.json`
/// sidecar (see `HlsResumeState`). On entry the helper matches the
/// sidecar against a path-only fingerprint of `fragments` + the fragment
/// count; on a match (and when the existing partial is at least as long as the
/// last confirmed byte boundary and the download is incomplete) it truncates
/// any torn tail to that boundary, seeks, and skips the already-completed
/// fragments. Otherwise it starts fresh. The sidecar is rewritten atomically
/// after each fragment and removed on successful completion; it is left in
/// place on cancel or error so a later run can resume.
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

    // ---- Resume setup (issue #354) ----
    let state_file = state_path(output);
    let fingerprint = state::fragment_fingerprint(fragments);
    let total = fragments.len() as u64;
    let actual_len = tokio::fs::metadata(output).await.map_or(0, |m| m.len());
    let loaded = state::HlsResumeState::load_matching(&state_file, fingerprint, total).await;
    // Resume only when state matches, the partial is at least as long as the
    // last confirmed boundary, and the download is not already complete.
    let resume = loaded
        .as_ref()
        .is_some_and(|s| actual_len >= s.byte_len && s.fragments_done < total);
    let mut hls_state = loaded
        .filter(|_| resume)
        .unwrap_or_else(|| state::HlsResumeState::new(fingerprint, total));
    let skip = hls_state.fragments_done as usize;

    let mut out_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(output)
        .await
        .map_err(|e| rdlp_core::RdlpError::Download {
            message: format!("create output: {e}"),
            url: Some(output.display().to_string()),
        })?;
    if resume {
        // Drop any torn tail past the last confirmed boundary, then append.
        out_file
            .set_len(hls_state.byte_len)
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("truncate to resume boundary: {e}"),
                url: Some(output.display().to_string()),
            })?;
        out_file
            .seek(SeekFrom::Start(hls_state.byte_len))
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("seek to resume boundary: {e}"),
                url: Some(output.display().to_string()),
            })?;
    } else {
        // Fresh start: equivalent to the previous unconditional truncate(true).
        out_file
            .set_len(0)
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("truncate output: {e}"),
                url: Some(output.display().to_string()),
            })?;
    }

    let mut total_bytes: u64 = hls_state.byte_len;

    const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
    let mut last_emit = Instant::now();
    let mut frags_done: u64 = hls_state.fragments_done;
    let total_frags = fragments.len() as u64;
    let mut speed = SpeedMeter::new();
    let mut save_tracker = SaveFailureTracker::new(SIDECAR_SAVE_FAILURE_THRESHOLD);

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
    let stream = futures::stream::iter(tasks.into_iter().skip(skip).map(|(frag, needs_init)| {
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

        // Update progress accounting from the cumulative byte total. Feeding the
        // running total (not per-fragment deltas) to a windowed meter is what
        // prevents the parallel-yield speed spike (#355).
        let now = Instant::now();
        speed.update(total_bytes, now);
        frags_done += 1;

        hls_state.fragments_done = frags_done;
        hls_state.byte_len = total_bytes;
        crate::atomic::note_sidecar_save(
            hls_state.save(&state_file).await,
            &mut save_tracker,
            "HLS",
        );

        // Emit progress (100ms throttle OR fragment-N boundary).
        if let Some(cb) = progress
            && (now.duration_since(last_emit) >= PROGRESS_INTERVAL || frags_done == total_frags)
        {
            // Byte-extrapolation (yt-dlp): prefer a real Content-Length total;
            // else estimate total = avg-bytes-per-frag × total_frags (available
            // once >= 1 fragment completed). Feeding this to `new` makes BOTH the
            // progress fraction and the ETA byte-based + self-correcting.
            let est_total = expected_total.or_else(|| {
                (frags_done > 0).then(|| total_bytes.saturating_mul(total_frags) / frags_done)
            });
            let mut info =
                DownloadProgress::new(total_bytes, est_total, speed.bytes_per_sec().unwrap_or(0.0));
            info.segments_downloaded = Some(frags_done); // secondary "frag N/M" text
            info.total_segments = Some(total_frags);
            info.is_estimated = expected_total.is_none() && est_total.is_some();
            cb.on_progress(&info);
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

    // Download completed — drop the resume sidecar.
    let _ = tokio::fs::remove_file(&state_file).await;

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

/// Sidecar path for HLS resume state: `<output>.hls_state.json`.
/// Appends the suffix to the full filename so it never collides with the
/// output's own extension.
pub(crate) fn state_path(output: &Path) -> std::path::PathBuf {
    let mut s = output.as_os_str().to_os_string();
    s.push(".hls_state.json");
    std::path::PathBuf::from(s)
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

pub(crate) mod state;

#[cfg(test)]
mod tests;
