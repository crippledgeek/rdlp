//! DASH download orchestration: MPD fetch + parallel segment fetch + concat
//! + FFmpeg stream-copy mux into a single output container.
//!
//! Resume model: per-segment temp files under `<output>.<suffix>.parts/`,
//! with init markers and completed-index tracking persisted to
//! `<output>.dash_state.json`. State is path-only-URL-keyed (CDN-tolerant),
//! saved every 16 successful segment fetches, and deleted on full mux
//! success.

#![allow(clippy::doc_markdown)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use backon::Retryable;
use futures::StreamExt;
use log::{debug, info, warn};
use rdlp_core::{
    DownloadStats, ProgressCallback, RdlpError, Result, RetryConfig, is_retryable_error,
};
use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use url::Url;

use crate::adaptive::{AdaptiveConfig, AdaptiveController, ControllerMode};
use crate::dash::errors::DashError;
use crate::dash::manifest;
use crate::dash::segments::SegmentPlan;
use crate::dash::state::DashDownloadState;
use crate::http::HttpDownloader;

/// Number of successful segment fetches between state-save flushes.
const STATE_SAVE_BATCH: usize = 16;

/// Run a DASH download for `mpd_url` into `output_path`.
///
/// Internally produces `<output>.video.m4s` and (when an audio repr exists)
/// `<output>.audio.m4s`, then muxes them via FFmpeg stream-copy into
/// `output_path`. Intermediates are deleted on success and retained on
/// mux failure for diagnosis. Resume state is tracked under
/// `<output>.dash_state.json` and pruned on successful mux.
#[allow(clippy::too_many_lines, clippy::similar_names)]
pub async fn run(
    http_downloader: &HttpDownloader,
    retry_config: Arc<RetryConfig>,
    concurrent_segments: usize,
    buffer_size: usize,
    mpd_url: &str,
    output_path: &Path,
    progress: Option<Box<dyn ProgressCallback>>,
) -> Result<DownloadStats> {
    let mpd_url_parsed = Url::parse(mpd_url).map_err(|e| RdlpError::Extraction {
        message: format!("invalid MPD URL: {e}"),
        url: Some(mpd_url.to_string()),
    })?;

    // Fetch MPD body.
    let client = http_downloader.client().clone();
    let response = client
        .get(mpd_url)
        .headers(http_downloader.headers())
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| RdlpError::Network {
            message: format!("MPD fetch failed: {e}"),
            url: Some(mpd_url.to_string()),
        })?;
    if !response.status().is_success() {
        return Err(RdlpError::Http {
            status: response.status().as_u16(),
            reason: format!("MPD returned HTTP {}", response.status()),
        });
    }
    let body = response.text().await.map_err(|e| RdlpError::Network {
        message: format!("MPD read error: {e}"),
        url: Some(mpd_url.to_string()),
    })?;

    let parsed = manifest::parse_mpd(&body, &mpd_url_parsed).map_err(RdlpError::from)?;

    // Same-origin gate seed (issue #319). The MPD URL IS the format URL for
    // this legacy path, so its origin is the seed against which all segment
    // URLs are compared. Cross-origin segments (CDN-redirected media) get
    // operator-set headers stripped. Same gate pattern as PR #273 for the
    // modern per-Repr path; gate applied inside `download_one`.
    let mpd_origin = mpd_url_parsed.origin();

    let started = Instant::now();
    let bytes_total = Arc::new(AtomicU64::new(0));

    // Wrap the progress callback in Arc so it can be shared with the per-
    // representation adaptive controllers without cloning the Box.
    let progress_arc: Option<Arc<dyn ProgressCallback>> =
        progress.map(|cb| Arc::from(cb) as Arc<dyn ProgressCallback>);

    // Resolve all paths up-front.
    let video_final = intermediate_path(output_path, "video");
    let audio_final = intermediate_path(output_path, "audio");
    let video_parts = parts_dir(output_path, "video");
    let audio_parts = parts_dir(output_path, "audio");
    let state_file = state_path(output_path);

    let video_repr_id = parsed.video.id.clone();
    let audio_repr_id = parsed.audio.as_ref().map(|a| a.id.clone());

    // Load matching state, or start fresh.
    let loaded_state = DashDownloadState::load_matching(
        &state_file,
        &mpd_url_parsed,
        &video_repr_id,
        audio_repr_id.as_deref(),
    )
    .await;
    let initial_state = loaded_state.unwrap_or_else(|| {
        DashDownloadState::new(
            &mpd_url_parsed,
            video_repr_id.clone(),
            audio_repr_id.clone(),
        )
    });
    let state_arc = Arc::new(Mutex::new(initial_state));

    // Video (always present after parse).
    let video_ts = period_duration_ts(parsed.period_duration, &parsed.video.plan);
    let video_init = parsed.video.plan.init_url(&parsed.video.base_urls);
    let video_seg_urls = parsed
        .video
        .plan
        .segment_urls(&parsed.video.base_urls, video_ts);
    if video_seg_urls.is_empty() {
        return Err(RdlpError::Download {
            message: "DASH: no video segments resolved".into(),
            url: Some(mpd_url.to_string()),
        });
    }
    let video_bytes = download_representation(
        http_downloader,
        &retry_config,
        concurrent_segments,
        buffer_size,
        &video_repr_id,
        video_init,
        video_seg_urls,
        &video_final,
        &video_parts,
        Arc::clone(&state_arc),
        &state_file,
        true,
        Arc::clone(&bytes_total),
        progress_arc.clone(),
        &mpd_origin,
    )
    .await?;

    // Audio (optional).
    let has_audio = parsed.audio.is_some();
    let audio_bytes = if let Some(audio_repr) = parsed.audio.as_ref() {
        let audio_ts = period_duration_ts(parsed.period_duration, &audio_repr.plan);
        let audio_init = audio_repr.plan.init_url(&audio_repr.base_urls);
        let audio_seg_urls = audio_repr
            .plan
            .segment_urls(&audio_repr.base_urls, audio_ts);
        if audio_seg_urls.is_empty() {
            return Err(RdlpError::Download {
                message: "DASH: no audio segments resolved".into(),
                url: Some(mpd_url.to_string()),
            });
        }
        download_representation(
            http_downloader,
            &retry_config,
            concurrent_segments,
            buffer_size,
            &audio_repr.id,
            audio_init,
            audio_seg_urls,
            &audio_final,
            &audio_parts,
            Arc::clone(&state_arc),
            &state_file,
            false,
            Arc::clone(&bytes_total),
            progress_arc.clone(),
            &mpd_origin,
        )
        .await?
    } else {
        0
    };

    // ----- Mux video + audio into the single output container -----
    //
    // The trait's single-file contract (`download_to_file` writes one
    // output) is preserved by performing the mux here, before returning.
    // Intermediates are deleted on success and retained on failure for
    // diagnosis.
    mux_outputs(&video_final, has_audio.then_some(&audio_final), output_path).await?;

    // Mux succeeded → wipe resume state.
    if let Err(e) = fs::remove_file(&state_file).await {
        debug!(
            "DASH state cleanup: failed to remove {} ({e})",
            state_file.display()
        );
    }

    let elapsed = started.elapsed();
    let bytes = video_bytes + audio_bytes;
    #[allow(clippy::cast_precision_loss)]
    let avg = if elapsed.as_secs_f64() > 0.0 {
        bytes as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let stats = DownloadStats {
        bytes_downloaded: bytes,
        duration: elapsed,
        average_speed: avg,
        retries: 0,
        fragments: None,
    };

    info!(
        "DASH download complete: {} bytes in {:.1}s",
        bytes,
        elapsed.as_secs_f64()
    );
    if let Some(ref cb) = progress_arc {
        cb.on_log(&format!(
            "DASH download complete: {} bytes in {:.1}s",
            bytes,
            elapsed.as_secs_f64()
        ));
        cb.on_complete(&stats);
    }

    Ok(stats)
}

fn period_duration_ts(period_duration: Duration, plan: &SegmentPlan) -> u64 {
    let timescale = match plan {
        SegmentPlan::Template(t) => t.timescale,
        SegmentPlan::Timeline(t) => t.timescale,
        SegmentPlan::List(_) => 1,
    };
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let ts = (period_duration.as_secs_f64() * timescale as f64).round() as u64;
    ts
}

fn intermediate_path(output: &Path, suffix: &str) -> PathBuf {
    let stem = output
        .file_stem()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let parent = output
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut name = stem.into_string().unwrap_or_default();
    name.push('.');
    name.push_str(suffix);
    name.push_str(".m4s");
    parent.join(name)
}

fn parts_dir(output: &Path, suffix: &str) -> PathBuf {
    let stem = output
        .file_stem()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let parent = output
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut name = stem.into_string().unwrap_or_default();
    name.push('.');
    name.push_str(suffix);
    name.push_str(".parts");
    parent.join(name)
}

fn state_path(output: &Path) -> PathBuf {
    let mut p = output.to_path_buf().into_os_string();
    p.push(".dash_state.json");
    PathBuf::from(p)
}

fn segment_filename(idx: usize) -> String {
    format!("{idx:04}.m4s")
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn download_representation(
    http: &HttpDownloader,
    retry: &Arc<RetryConfig>,
    concurrent: usize,
    buffer_size: usize,
    repr_id: &str,
    init_url: Option<Url>,
    seg_urls: Vec<Url>,
    final_path: &Path,
    parts_dir: &Path,
    state_arc: Arc<Mutex<DashDownloadState>>,
    state_path: &Path,
    is_video: bool,
    bytes_counter: Arc<AtomicU64>,
    log_callback: Option<Arc<dyn ProgressCallback>>,
    mpd_origin: &url::Origin,
) -> Result<u64> {
    if let Some(parent) = final_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).await?;
    }
    fs::create_dir_all(parts_dir).await?;

    let mut total: u64 = 0;

    // ----- Init segment -----
    let init_part_path = parts_dir.join("init.m4s");
    if let Some(u) = init_url {
        let init_done = {
            let s = state_arc.lock().await;
            if is_video {
                s.init_video_done
            } else {
                s.init_audio_done
            }
        };
        let on_disk = fs::metadata(&init_part_path)
            .await
            .is_ok_and(|m| m.len() > 0);
        if init_done && on_disk {
            // Resume — count existing bytes toward total.
            let len = fs::metadata(&init_part_path).await?.len();
            bytes_counter.fetch_add(len, Ordering::Relaxed);
            total += len;
            debug!("DASH repr {repr_id}: init resumed ({len} bytes)");
        } else {
            let bytes = download_one(http, retry, &u, mpd_origin).await?;
            let len = bytes.len() as u64;
            fs::write(&init_part_path, &bytes).await?;
            bytes_counter.fetch_add(len, Ordering::Relaxed);
            total += len;
            {
                let mut s = state_arc.lock().await;
                if is_video {
                    s.init_video_done = true;
                } else {
                    s.init_audio_done = true;
                }
                if let Err(e) = s.save(state_path).await {
                    warn!("DASH state save failed (init): {e}");
                }
            }
            debug!("DASH repr {repr_id}: init {len} bytes");
        }
    }

    // ----- Segments -----
    //
    // Partition into "already-done on disk" (skip + count bytes) vs
    // "needs fetch" (enqueue).
    let total_segments = seg_urls.len();
    let mut to_fetch: Vec<(usize, Url)> = Vec::new();
    {
        let mut s = state_arc.lock().await;
        for (i, u) in seg_urls.into_iter().enumerate() {
            let part_path = parts_dir.join(segment_filename(i));
            let recorded_done = s.is_segment_done(repr_id, i as u64);
            let on_disk_len = fs::metadata(&part_path).await.map_or(0, |m| m.len());
            if recorded_done && on_disk_len > 0 {
                bytes_counter.fetch_add(on_disk_len, Ordering::Relaxed);
                total += on_disk_len;
            } else {
                // If state said done but file is missing or zero-length, drop
                // the bookkeeping so we re-fetch.
                if recorded_done && let Some(v) = s.completed_segments.get_mut(repr_id) {
                    v.retain(|x| *x != i as u64);
                }
                to_fetch.push((i, u));
            }
        }
    }

    let concurrent = concurrent.max(1);

    // Build an AIMD adaptive controller for this representation.
    // `HlsSegments` mode skips chunk-level adjustments (segment sizes are
    // server-determined), matching the HLS downloader's pattern exactly.
    let controller = Arc::new(AdaptiveController::new(
        0, // total_size not meaningful for segment-based downloads
        AdaptiveConfig {
            max_connections: concurrent,
            initial_connections: concurrent.min(2),
            ..AdaptiveConfig::default()
        },
        ControllerMode::HlsSegments,
        log_callback,
    ));
    let sem = controller.semaphore().clone();

    let mpd_origin_owned = mpd_origin.clone();
    let mut stream = futures::stream::iter(to_fetch.into_iter().map(|(i, u)| {
        let http = http.clone();
        let retry = Arc::clone(retry);
        let parts_dir = parts_dir.to_path_buf();
        let sem = sem.clone();
        let controller = Arc::clone(&controller);
        let mpd_origin = mpd_origin_owned.clone();
        async move {
            // Acquire a semaphore permit so the adaptive controller governs
            // actual concurrency (mirrors the HLS pattern in merge.rs).
            let _permit = sem.acquire_owned().await.map_err(|_| RdlpError::Download {
                message: "DASH adaptive semaphore closed".to_string(),
                url: None,
            })?;

            let fetch_start = Instant::now();
            let bytes = download_one(&http, &retry, &u, &mpd_origin).await?;
            let len = bytes.len() as u64;
            let elapsed = fetch_start.elapsed();

            let part_path = parts_dir.join(segment_filename(i));
            fs::write(&part_path, &bytes).await.map_err(RdlpError::Io)?;

            // Report to the adaptive controller so AIMD can tune connection
            // count based on observed throughput.  Permit is still held here;
            // it drops at the end of this async block.
            controller.report_segment_complete(len, elapsed, None);

            Ok::<(usize, u64), RdlpError>((i, len))
        }
    }))
    .buffer_unordered(concurrent * 2);

    let mut completed_since_save: usize = 0;
    let mut stream_err: Option<RdlpError> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok((i, len)) => {
                bytes_counter.fetch_add(len, Ordering::Relaxed);
                total += len;
                let mut s = state_arc.lock().await;
                s.record_segment(repr_id, i as u64);
                completed_since_save += 1;
                if completed_since_save >= STATE_SAVE_BATCH {
                    if let Err(e) = s.save(state_path).await {
                        warn!("DASH state save failed (batch): {e}");
                    }
                    completed_since_save = 0;
                }
            }
            Err(e) => {
                // Don't return immediately — drain remaining in-flight
                // results so their bookkeeping is recorded and persisted
                // before we propagate the error.
                if stream_err.is_none() {
                    stream_err = Some(e);
                }
            }
        }
    }

    // Always flush state once the stream has drained — covers both the
    // success path and the error path (so the next run can resume from
    // whatever did complete before the failure).
    {
        let mut s = state_arc.lock().await;
        if let Err(e) = s.save(state_path).await {
            warn!("DASH state save failed (final): {e}");
        }
    }
    if let Some(e) = stream_err {
        return Err(e);
    }
    let _ = completed_since_save;

    // ----- Concat init + sorted segments → final_path -----
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(final_path)
        .await?;
    let mut writer = BufWriter::with_capacity(buffer_size, file);

    if init_url_was_present(&init_part_path).await {
        let init_bytes = fs::read(&init_part_path).await?;
        writer.write_all(&init_bytes).await?;
    }
    for i in 0..total_segments {
        let part_path = parts_dir.join(segment_filename(i));
        let seg_bytes = fs::read(&part_path).await?;
        writer.write_all(&seg_bytes).await?;
    }
    writer.flush().await?;

    // Best-effort cleanup of parts dir — successful concat means we don't
    // need them anymore. Keep them on error so the next run can resume.
    if let Err(e) = fs::remove_dir_all(parts_dir).await {
        debug!(
            "DASH parts cleanup: failed to remove {} ({e})",
            parts_dir.display()
        );
    }

    Ok(total)
}

async fn init_url_was_present(init_part_path: &Path) -> bool {
    fs::metadata(init_part_path).await.is_ok()
}

/// Mux video (+ optional audio) intermediates into `output_path` via
/// FFmpeg stream-copy. On success, intermediates are deleted (best-effort).
/// On failure they are retained for diagnosis and the error is propagated.
///
/// When `audio_path` is `None`, the video intermediate is moved (renamed,
/// or copy+remove on cross-mount failure) into the final output path.
async fn mux_outputs(
    video_path: &Path,
    audio_path: Option<&Path>,
    output_path: &Path,
) -> Result<()> {
    if let Some(audio_path) = audio_path {
        let runner = FFmpegRunner::new().map_err(|e| {
            RdlpError::from(DashError::Mux(format!("FFmpegRunner init failed: {e}")))
        })?;
        let opts = RemuxOptions {
            encoding_tool_override: Some(rdlp_ffmpeg::encoding_tool_tag("dash")),
            ..Default::default()
        };
        match runner
            .merge(video_path, audio_path, output_path, &opts, None)
            .await
        {
            Ok(()) => {
                if let Err(e) = fs::remove_file(video_path).await {
                    debug!(
                        "DASH cleanup: failed to remove {} ({e})",
                        video_path.display()
                    );
                }
                if let Err(e) = fs::remove_file(audio_path).await {
                    debug!(
                        "DASH cleanup: failed to remove {} ({e})",
                        audio_path.display()
                    );
                }
                Ok(())
            }
            Err(e) => {
                warn!(
                    "DASH mux failed; intermediates retained: {} {}",
                    video_path.display(),
                    audio_path.display()
                );
                Err(RdlpError::from(DashError::Mux(format!(
                    "FFmpeg merge failed: {e}"
                ))))
            }
        }
    } else {
        // Video-only: move intermediate into place. Try rename first; fall
        // back to copy+remove on cross-mount failures.
        if let Err(rename_err) = fs::rename(video_path, output_path).await {
            debug!("DASH video-only rename failed ({rename_err}); falling back to copy+remove");
            fs::copy(video_path, output_path)
                .await
                .map_err(RdlpError::Io)?;
            if let Err(e) = fs::remove_file(video_path).await {
                debug!(
                    "DASH cleanup: failed to remove {} ({e})",
                    video_path.display()
                );
            }
        }
        Ok(())
    }
}

async fn download_one(
    http: &HttpDownloader,
    retry: &Arc<RetryConfig>,
    url: &Url,
    mpd_origin: &url::Origin,
) -> Result<Vec<u8>> {
    let client = http.client().clone();
    // Same-origin header gate (issue #319). When the segment URL's origin
    // differs from the MPD URL's origin (CDN-redirected media), strip the
    // operator-set headers. `Origin::Opaque` compares not-equal — fails
    // closed. Mirrors PR #273 for the modern per-Repr path.
    let headers = if url.origin() == *mpd_origin {
        http.headers()
    } else {
        wreq::header::HeaderMap::new()
    };
    let backoff = retry.to_backoff();
    let url_str = url.to_string();
    (|| {
        let client = client.clone();
        let headers = headers.clone();
        let url_str = url_str.clone();
        async move {
            let resp = client
                .get(&url_str)
                .headers(headers)
                .timeout(Duration::from_secs(60))
                .send()
                .await
                .map_err(|e| RdlpError::Network {
                    message: format!("DASH segment fetch failed: {e}"),
                    url: Some(url_str.clone()),
                })?;
            if !resp.status().is_success() {
                return Err(RdlpError::Http {
                    status: resp.status().as_u16(),
                    reason: format!("DASH segment HTTP {}", resp.status()),
                });
            }
            let bytes = resp.bytes().await.map_err(|e| RdlpError::Network {
                message: format!("DASH segment read error: {e}"),
                url: Some(url_str.clone()),
            })?;
            Ok(bytes.to_vec())
        }
    })
    .retry(backoff)
    .when(is_retryable_error)
    .notify(|err, dur| {
        warn!("DASH segment fetch retry after {dur:?}: {err}");
    })
    .await
}

#[cfg(test)]
mod same_origin_gate_tests {
    //! Same-origin header gate (issue #319). Mirrors PR #273's gate on the
    //! modern per-Repr DASH path. When the segment URL's origin differs
    //! from the MPD URL's origin, operator-set `Format.http_headers` are
    //! stripped to prevent header exfiltration to a redirected CDN.
    use super::*;
    use rdlp_core::RetryConfig;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_downloader_with_header(name: &str, value: &str) -> HttpDownloader {
        let mut headers = HashMap::new();
        headers.insert(name.to_string(), value.to_string());
        HttpDownloader::with_client(wreq::Client::new()).with_extra_headers(Some(&headers))
    }

    fn fast_retry() -> Arc<RetryConfig> {
        // 0-retry config so failing tests fail immediately rather than
        // exercising backoff loops.
        Arc::new(RetryConfig {
            max_retries: 0,
            ..RetryConfig::default()
        })
    }

    /// Positive: same-origin segment URL DOES receive `Format.http_headers`.
    /// Companion to the negative tests below; prevents a defensive
    /// over-correction (e.g. always-strip) from going unnoticed.
    #[tokio::test]
    async fn download_one_same_origin_forwards_headers() {
        let mut server = mockito::Server::new_async().await;
        let referer = "https://operator.example.com/page";

        let _mock = server
            .mock("GET", "/seg.m4s")
            .match_header("Referer", referer)
            .with_body(&b"abcd"[..])
            .expect(1)
            .create_async()
            .await;

        let http = make_downloader_with_header("Referer", referer);
        let retry = fast_retry();
        let mpd_url = format!("{}/manifest.mpd", server.url());
        let mpd_origin = url::Url::parse(&mpd_url).unwrap().origin();
        let seg_url = url::Url::parse(&format!("{}/seg.m4s", server.url())).unwrap();

        let bytes = download_one(&http, &retry, &seg_url, &mpd_origin)
            .await
            .expect("same-origin segment fetch must succeed");
        assert_eq!(&bytes[..], b"abcd");
    }

    /// Negative: cross-origin segment URL does NOT receive `Format.http_headers`.
    /// Two mockito servers = different ports = different origins per RFC 6454.
    /// Catch-all 501 on the cross-origin server fires if Referer leaks.
    #[tokio::test]
    async fn download_one_cross_origin_strips_headers() {
        use mockito::Matcher;

        let mpd_server = mockito::Server::new_async().await;
        let mut cdn_server = mockito::Server::new_async().await;
        let referer = "https://operator.example.com/page";

        // Cross-origin segment: must NOT see Referer.
        let _seg = cdn_server
            .mock("GET", "/seg.m4s")
            .match_header("Referer", Matcher::Missing)
            .with_body(&b"abcd"[..])
            .expect(1)
            .create_async()
            .await;

        // Catch-all on the cross-origin server: 501 if Referer DID arrive.
        let _catchall = cdn_server
            .mock("GET", Matcher::Any)
            .with_status(501)
            .create_async()
            .await;

        let http = make_downloader_with_header("Referer", referer);
        let retry = fast_retry();
        let mpd_url = format!("{}/manifest.mpd", mpd_server.url());
        let mpd_origin = url::Url::parse(&mpd_url).unwrap().origin();
        let seg_url = url::Url::parse(&format!("{}/seg.m4s", cdn_server.url())).unwrap();

        let bytes = download_one(&http, &retry, &seg_url, &mpd_origin)
            .await
            .expect("cross-origin segment fetch must succeed without leaking headers");
        assert_eq!(&bytes[..], b"abcd");
    }

    /// Negative: `Origin::Opaque` mpd_origin (constructed via parse-of
    /// opaque-scheme URL) compares not-equal to every Tuple origin → fails
    /// closed (headers stripped). Verifies the type-level guarantee.
    #[tokio::test]
    async fn download_one_opaque_mpd_origin_strips_headers() {
        use mockito::Matcher;

        let mut server = mockito::Server::new_async().await;
        let referer = "https://operator.example.com/page";

        let _seg = server
            .mock("GET", "/seg.m4s")
            .match_header("Referer", Matcher::Missing)
            .with_body(&b"abcd"[..])
            .expect(1)
            .create_async()
            .await;

        let _catchall = server
            .mock("GET", Matcher::Any)
            .with_status(501)
            .create_async()
            .await;

        let http = make_downloader_with_header("Referer", referer);
        let retry = fast_retry();
        // `data:` is an opaque-origin scheme per RFC 6454; any `Origin::Opaque`
        // compares not-equal to every Tuple origin including itself.
        let opaque_origin = url::Url::parse("data:text/plain,seed").unwrap().origin();
        assert!(
            !opaque_origin.is_tuple(),
            "data: scheme must produce Origin::Opaque"
        );
        let seg_url = url::Url::parse(&format!("{}/seg.m4s", server.url())).unwrap();

        let bytes = download_one(&http, &retry, &seg_url, &opaque_origin)
            .await
            .expect("opaque mpd_origin must fail closed: headers stripped, fetch still succeeds");
        assert_eq!(&bytes[..], b"abcd");
    }
}
