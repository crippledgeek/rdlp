//! Shared download execution with CDN fallback and token validation
//!
//! Consolidates the CDN fallback retry loop and token expiry check that were
//! previously duplicated between `state.rs` and `playlist.rs`.

use super::{Orchestrator, errors::*};
use log::{info, warn};
use rdlp_core::{DownloadStats, Format, RdlpError};
use rdlp_security::validate_url_security;
use std::path::Path;

/// Result of a successful download with CDN fallback
pub(super) struct DownloadOutcome {
    /// Download statistics from the successful attempt
    #[allow(dead_code)]
    pub stats: DownloadStats,
    /// Whether HLS was detected for this format
    pub is_hls: bool,
}

impl Orchestrator {
    /// Execute a download with CDN fallback retry and token validation.
    ///
    /// This is the single implementation of the CDN fallback pattern, used by
    /// both the state machine (single videos) and playlist downloads.
    ///
    /// # Flow
    /// 1. Check CDN token expiry (if present in URL)
    /// 2. Find appropriate downloader
    /// 3. Try primary URL, then fallback URLs on failure
    ///
    /// Progress is reported via [`Event::Progress`] through the event channel.
    ///
    /// # Returns
    /// - `Ok(Some(outcome))` — download succeeded
    /// - `Ok(None)` — user cancelled
    /// - `Err(e)` — all URLs failed or token expired
    pub(super) async fn download_with_cdn_fallback(
        &self,
        format: &Format,
        output_path: &Path,
        resume_from: u64,
    ) -> Result<Option<DownloadOutcome>> {
        let is_hls = format.is_hls();

        // HLS downloads use segment-based resume (tracked in .hls_state.json), not byte offsets
        let effective_resume = if is_hls { 0 } else { resume_from };

        let estimated_size = if is_hls {
            None // HLS uses segment-based progress, not byte-based
        } else {
            format.filesize.or(format.filesize_approx)
        };

        // SSRF protection: validate primary URL before passing to downloader
        validate_url_security(&format.url).map_err(|e| {
            OrchestratorError::DownloadFailed(RdlpError::Network(format!(
                "Security validation failed for format URL: {e}"
            )))
        })?;

        // Find downloader (with extra HTTP headers if the format specifies them)
        let downloader = self
            .downloader_registry
            .find_downloader_with_headers(&format.url, format.http_headers.as_ref())
            .ok_or_else(|| OrchestratorError::NoDownloader {
                url: format.url.clone(),
            })?;

        // Validate CDN token expiry
        self.check_cdn_token_expiry(&format.url)?;

        // Build URL chain: primary + fallbacks (validate each fallback before use)
        let download_urls: Vec<&str> = std::iter::once(format.url.as_str())
            .chain(format.fallback_urls.iter().flatten().map(String::as_str))
            .collect();

        let mut stats = None;
        let mut last_err = None;
        for (i, download_url) in download_urls.iter().enumerate() {
            if i > 0 {
                // SSRF protection: validate each fallback URL before use
                if let Err(e) = validate_url_security(download_url) {
                    warn!(fallback = i; "Skipping fallback URL that failed security validation: {e}");
                    last_err = Some(OrchestratorError::DownloadFailed(RdlpError::Network(
                        format!("Security validation failed for fallback URL: {e}"),
                    )));
                    continue;
                }
                warn!(
                    fallback = i,
                    url:? = download_url;
                    "Primary CDN failed, trying fallback"
                );
            }
            match self
                .execute_download(
                    &downloader,
                    download_url,
                    output_path,
                    effective_resume,
                    estimated_size,
                )
                .await
            {
                Ok(Some(s)) => {
                    stats = Some(s);
                    last_err = None;
                    break;
                }
                Ok(None) => return Ok(None), // User cancelled
                Err(e) => {
                    if i < download_urls.len() - 1 {
                        warn!("Download failed: {e}, will try fallback CDN");
                    }
                    last_err = Some(e);
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
        let stats = stats.expect("stats is Some when no error occurred");

        info!("Downloaded successfully!");
        info!("   File: {}", output_path.display());
        info!("   Stats: {stats:?}");

        Ok(Some(DownloadOutcome { stats, is_hls }))
    }

    /// Execute a download to stdout with token validation (no CDN fallback).
    ///
    /// Unlike `download_with_cdn_fallback`, this does **not** try fallback URLs.
    /// Once bytes have been written to a pipe, retrying with a different CDN URL
    /// would corrupt the stream (the consumer would see partial data from URL 1
    /// followed by a full stream from URL 2).
    ///
    /// HLS formats are rejected (not yet supported for stdout streaming).
    /// Merge downloads are rejected at the state machine level.
    ///
    /// **Note on stdout validation split:** HLS and Merge checks happen here at
    /// runtime rather than in `Config::validate()` because they depend on the
    /// selected format, which is only known after extraction.
    ///
    /// # Returns
    /// - `Ok(Some(stats))` — download succeeded
    /// - `Ok(None)` — user cancelled
    /// - `Err(e)` — download failed, token expired, or unsupported format
    pub(super) async fn download_to_stdout(
        &self,
        format: &Format,
    ) -> Result<Option<DownloadStats>> {
        if format.is_hls() {
            return Err(OrchestratorError::Configuration(
                "HLS is not yet supported with -o - (stdout output); \
                 select an HTTP format instead"
                    .to_string(),
            ));
        }

        // Log when fallback URLs exist but can't be used for pipe output
        if let Some(ref fallbacks) = format.fallback_urls {
            if !fallbacks.is_empty() {
                info!(
                    count = fallbacks.len();
                    "Format has fallback CDN URLs but they cannot be used \
                     with stdout (partial pipe writes are irreversible)"
                );
            }
        }

        // SSRF protection: validate URL before passing to downloader
        validate_url_security(&format.url).map_err(|e| {
            OrchestratorError::DownloadFailed(RdlpError::Network(format!(
                "Security validation failed for format URL: {e}"
            )))
        })?;

        let downloader = self
            .downloader_registry
            .find_downloader_with_headers(&format.url, format.http_headers.as_ref())
            .ok_or_else(|| OrchestratorError::NoDownloader {
                url: format.url.clone(),
            })?;

        self.check_cdn_token_expiry(&format.url)?;

        let writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send> = Box::new(tokio::io::stdout());

        match self
            .execute_download_to_writer(&downloader, &format.url, writer)
            .await
        {
            Ok(Some(stats)) => {
                info!("Stdout download completed: {stats:?}");
                Ok(Some(stats))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Check if the CDN token in a URL has expired.
    ///
    /// Supports two URL token formats:
    /// - `validto=TIMESTAMP` (PornHub direct MP4)
    /// - `~exp=TIMESTAMP~` (PornHub HLS / Akamai)
    ///
    /// Returns `Ok(())` if no token or token is still valid.
    fn check_cdn_token_expiry(&self, url: &str) -> Result<()> {
        if let Some(expires) = parse_url_expiry(url) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now >= expires {
                return Err(OrchestratorError::DownloadFailed(RdlpError::Network(
                    format!(
                        "CDN token expired {}s ago — re-run the command to get a fresh URL",
                        now - expires
                    ),
                )));
            } else if expires - now < 60 {
                warn!("CDN token expires in less than 60 seconds — download may fail");
            }
        }
        Ok(())
    }
}

/// Parse CDN token expiry timestamp from a URL.
///
/// Supports two formats:
/// - `validto=TIMESTAMP` (PornHub direct MP4)
/// - `~exp=TIMESTAMP~` (PornHub HLS / Akamai)
fn parse_url_expiry(url: &str) -> Option<u64> {
    /// Extract a `u64` timestamp immediately following `prefix` in `url`.
    fn extract_ts_after(url: &str, prefix: &str) -> Option<u64> {
        let rest = &url[url.find(prefix)? + prefix.len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u64>().ok()
    }

    extract_ts_after(url, "validto=").or_else(|| extract_ts_after(url, "~exp="))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_expiry_validto() {
        let url = "https://cdn.example.com/video.mp4?validto=1700000000&token=abc";
        assert_eq!(parse_url_expiry(url), Some(1_700_000_000));
    }

    #[test]
    fn test_parse_url_expiry_akamai() {
        let url = "https://cdn.example.com/video.mp4?~exp=1700000000~acl=/video/*";
        assert_eq!(parse_url_expiry(url), Some(1_700_000_000));
    }

    #[test]
    fn test_parse_url_expiry_none() {
        let url = "https://cdn.example.com/video.mp4?token=abc";
        assert_eq!(parse_url_expiry(url), None);
    }

    /// Verify that private-network format URLs are rejected by `validate_url_security`
    /// before they can reach the downloader (SSRF protection).
    #[test]
    fn test_private_network_format_url_rejected() {
        // SSRF targets that must never reach the downloader
        let private_urls = [
            "http://169.254.169.254/latest/meta-data/", // AWS instance metadata
            "http://192.168.1.1/video.mp4",             // RFC-1918 LAN
            "http://10.0.0.1/video.mp4",                // RFC-1918 LAN
            "http://172.16.0.1/video.mp4",              // RFC-1918 LAN
            "http://localhost/video.mp4",               // loopback
            "http://127.0.0.1/video.mp4",               // loopback
        ];
        for url in &private_urls {
            assert!(
                validate_url_security(url).is_err(),
                "Expected security rejection for private URL: {url}"
            );
        }
    }

    /// Verify that legitimate public CDN URLs pass `validate_url_security`.
    #[test]
    fn test_public_format_url_accepted() {
        let public_urls = [
            "https://cdn.example.com/video.mp4",
            "https://ev-h-ph.rdtcdn.com/hls/videos/123/master.m3u8",
            "http://media.example.com/file.ts",
        ];
        for url in &public_urls {
            assert!(
                validate_url_security(url).is_ok(),
                "Expected security pass for public URL: {url}"
            );
        }
    }
}
