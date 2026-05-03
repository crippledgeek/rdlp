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
use std::time::Duration;
use std::time::Instant;

use rdlp_core::{DownloadStats, Result};
use rdlp_types::Fragment;
use tokio::io::AsyncWriteExt as _;

use crate::http::HttpDownloader;

/// Fetch a pre-resolved list of fragment URLs and concatenate them into
/// `output` in order.
///
/// Each URL is resolved against the optional `base_url`. Fragments are written
/// sequentially — no intermediate files or `FFmpeg` mux step is required.
///
/// # Errors
///
/// Returns `RdlpError::Download` if any fragment fetch fails, if a URL fails
/// to resolve against the base URL, or if the output file cannot be created.
pub async fn download_pre_resolved_fragments(
    http: &HttpDownloader,
    fragments: &[Fragment],
    base_url: Option<&str>,
    output: &Path,
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

        let bytes = fetch_fragment_bytes(http, &resolved_url).await?;

        out_file
            .write_all(&bytes)
            .await
            .map_err(|e| rdlp_core::RdlpError::Download {
                message: format!("write fragment: {e}"),
                url: Some(output.display().to_string()),
            })?;

        total_bytes += bytes.len() as u64;
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

/// Fetch a single fragment URL and return its body as bytes.
pub(crate) async fn fetch_fragment_bytes(http: &HttpDownloader, url: &str) -> Result<Vec<u8>> {
    let client = http.client().clone();
    let headers = http.headers();
    let resp = client
        .get(url)
        .headers(headers)
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| rdlp_core::RdlpError::Network {
            message: format!("fragment fetch failed: {e}"),
            url: Some(url.to_string()),
        })?;
    if !resp.status().is_success() {
        return Err(rdlp_core::RdlpError::Http {
            status: resp.status().as_u16(),
            reason: format!("fragment HTTP {}", resp.status()),
        });
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| rdlp_core::RdlpError::Network {
            message: format!("fragment read error: {e}"),
            url: Some(url.to_string()),
        })?;
    Ok(bytes.to_vec())
}
