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

use rdlp_core::{DownloadStats, Result};
use rdlp_types::Fragment;
use tokio::io::AsyncWriteExt as _;

use crate::http::HttpDownloader;
use rdlp_security;

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
    let mut current_init: Option<String> = None;

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
                    fetch_with_optional_range(http, &resolved_init, frag.init_byte_range).await?;
                total_bytes += init_bytes.len() as u64;
                out_file
                    .write_all(&init_bytes)
                    .await
                    .map_err(|e| rdlp_core::RdlpError::Download {
                        message: format!("write init fragment: {e}"),
                        url: Some(output.display().to_string()),
                    })?;
                current_init = Some(init_url.clone());
            } else {
                current_init = None;
            }
        }

        let bytes = fetch_with_optional_range(http, &resolved_url, frag.byte_range).await?;

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
            .match_header(
                "Range",
                Matcher::Regex(r"^bytes=1024-2047$".to_string()),
            )
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
        download_pre_resolved_fragments(&http, &frags, None, tmp.path())
            .await
            .expect("download must succeed with Range header");
    }

    #[tokio::test]
    async fn multi_init_dedups_fetches() {
        let mut server = mockito::Server::new_async().await;

        // Init A is needed for fragments 1 + 2 → 1 fetch (deduped on consecutive).
        let _init_a = server
            .mock("GET", "/init-a.mp4")
            .with_body(b"INITA".to_vec())
            .expect(1)
            .create_async()
            .await;

        // Init B is needed for fragment 3 → 1 fetch.
        let _init_b = server
            .mock("GET", "/init-b.mp4")
            .with_body(b"INITB".to_vec())
            .expect(1)
            .create_async()
            .await;

        // 3 data segments.
        for i in 1..=3_u32 {
            server
                .mock("GET", format!("/seg-{i}.m4s").as_str())
                .with_body(b"DATA".to_vec())
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
                init_url: Some(if i < 3 { init_a.clone() } else { init_b.clone() }),
                init_byte_range: None,
                duration: Some(6.0),
                filesize: None,
            })
            .collect();

        let http = HttpDownloader::with_client(wreq::Client::new());
        let tmp = tempfile::NamedTempFile::new().unwrap();
        download_pre_resolved_fragments(&http, &frags, None, tmp.path())
            .await
            .expect("multi-init download must succeed");

        let written = std::fs::read(tmp.path()).unwrap();
        // Expected: INITA + DATA + DATA + INITB + DATA = 5 + 4 + 4 + 5 + 4 = 22 bytes.
        assert_eq!(written.len(), 22);
        assert_eq!(&written[0..5], b"INITA");
        assert_eq!(&written[5..9], b"DATA");
        assert_eq!(&written[9..13], b"DATA");
        assert_eq!(&written[13..18], b"INITB");
        assert_eq!(&written[18..22], b"DATA");
    }

    #[tokio::test]
    async fn single_init_fetched_once() {
        let mut server = mockito::Server::new_async().await;

        // Init must be fetched EXACTLY ONCE for 10 fragments.
        let _init = server
            .mock("GET", "/init.mp4")
            .with_body(b"INIT".to_vec())
            .expect(1)
            .create_async()
            .await;

        for i in 1..=10_u32 {
            server
                .mock("GET", format!("/seg-{i}.m4s").as_str())
                .with_body(b"D".to_vec())
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
        download_pre_resolved_fragments(&http, &frags, None, tmp.path())
            .await
            .expect("single-init download must succeed");

        let written = std::fs::read(tmp.path()).unwrap();
        // 4 (INIT) + 10 × 1 (D) = 14 bytes.
        assert_eq!(written.len(), 14);
    }
}
