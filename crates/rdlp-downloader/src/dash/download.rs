//! DASH download orchestration: MPD fetch + parallel segment fetch + concat.
//!
//! Mux to a single output container is performed by Task 5; this module
//! produces `<output>.video.m4s` and (optionally) `<output>.audio.m4s`.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use backon::Retryable;
use futures::StreamExt;
use log::{info, warn};
use rdlp_core::{
    DownloadStats, ProgressCallback, RdlpError, Result, RetryConfig, is_retryable_error,
};
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use url::Url;

use crate::dash::manifest;
use crate::dash::segments::SegmentPlan;
use crate::http::HttpDownloader;

/// Run a DASH download for `mpd_url` into `output_path`.
///
/// Produces sibling intermediates `<output>.video.m4s` and (when an audio
/// repr exists) `<output>.audio.m4s`. The actual `output_path` is NOT
/// written by this function — Task 5 will mux the intermediates into it.
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

    let started = Instant::now();
    let bytes_total = Arc::new(AtomicU64::new(0));

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
    let video_path = intermediate_path(output_path, "video");
    let video_bytes = download_representation(
        http_downloader,
        &retry_config,
        concurrent_segments,
        buffer_size,
        &parsed.video.id,
        video_init,
        video_seg_urls,
        &video_path,
        Arc::clone(&bytes_total),
    )
    .await?;

    // Audio (optional).
    let audio_bytes = download_audio_if_present(
        http_downloader,
        &retry_config,
        concurrent_segments,
        buffer_size,
        &parsed,
        output_path,
        mpd_url,
        Arc::clone(&bytes_total),
    )
    .await?;

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
    if let Some(cb) = progress {
        cb.on_log(&format!(
            "DASH download complete: {} bytes in {:.1}s",
            bytes,
            elapsed.as_secs_f64()
        ));
        cb.on_complete(&stats);
    }

    Ok(stats)
}

#[allow(clippy::too_many_arguments)]
async fn download_audio_if_present(
    http: &HttpDownloader,
    retry: &Arc<RetryConfig>,
    concurrent: usize,
    buffer_size: usize,
    parsed: &manifest::ParsedManifest,
    output_path: &Path,
    mpd_url: &str,
    bytes_counter: Arc<AtomicU64>,
) -> Result<u64> {
    let Some(audio_repr) = parsed.audio.as_ref() else {
        return Ok(0);
    };
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
    let audio_path = intermediate_path(output_path, "audio");
    download_representation(
        http,
        retry,
        concurrent,
        buffer_size,
        &audio_repr.id,
        audio_init,
        audio_seg_urls,
        &audio_path,
        bytes_counter,
    )
    .await
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

#[allow(clippy::too_many_arguments)]
async fn download_representation(
    http: &HttpDownloader,
    retry: &Arc<RetryConfig>,
    concurrent: usize,
    buffer_size: usize,
    repr_id: &str,
    init_url: Option<Url>,
    seg_urls: Vec<Url>,
    output: &Path,
    bytes_counter: Arc<AtomicU64>,
) -> Result<u64> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).await?;
    }

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)
        .await?;
    let mut writer = BufWriter::with_capacity(buffer_size, file);

    let mut total: u64 = 0;

    if let Some(u) = init_url {
        let bytes = download_one(http, retry, &u).await?;
        let len = bytes.len() as u64;
        writer.write_all(&bytes).await?;
        bytes_counter.fetch_add(len, Ordering::Relaxed);
        total += len;
        log::debug!("DASH repr {repr_id}: init {len} bytes");
    }

    let concurrent = concurrent.max(1);
    let mut results: BTreeMap<usize, Vec<u8>> = BTreeMap::new();

    let mut stream = futures::stream::iter(seg_urls.into_iter().enumerate().map(|(i, u)| {
        let http = http.clone();
        let retry = Arc::clone(retry);
        async move {
            let bytes = download_one(&http, &retry, &u).await?;
            Ok::<(usize, Vec<u8>), RdlpError>((i, bytes))
        }
    }))
    .buffer_unordered(concurrent);

    while let Some(item) = stream.next().await {
        let (i, bytes) = item?;
        let len = bytes.len() as u64;
        bytes_counter.fetch_add(len, Ordering::Relaxed);
        total += len;
        results.insert(i, bytes);
    }

    for (_, bytes) in results {
        writer.write_all(&bytes).await?;
    }
    writer.flush().await?;

    Ok(total)
}

async fn download_one(
    http: &HttpDownloader,
    retry: &Arc<RetryConfig>,
    url: &Url,
) -> Result<Vec<u8>> {
    let client = http.client().clone();
    let headers = http.headers();
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
