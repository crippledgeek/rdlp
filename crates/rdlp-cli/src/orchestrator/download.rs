//! Shared download execution with CDN fallback and token validation
//!
//! Consolidates the CDN fallback retry loop and token expiry check that were
//! previously duplicated between `state.rs` and `playlist.rs`.

use super::{Orchestrator, errors::*};
use log::{info, warn};
use rdlp_core::{DownloadStats, Format};
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
    /// 2. Create progress bar (byte-based for HTTP, segment-based for HLS)
    /// 3. Find appropriate downloader
    /// 4. Try primary URL, then fallback URLs on failure
    ///
    /// # Returns
    /// - `Ok(Some(outcome))` — download succeeded
    /// - `Ok(None)` — user cancelled (Ctrl+C)
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

        // Create appropriate progress bar
        let progress_bar = if is_hls {
            self.create_hls_progress_bar()?
        } else {
            self.create_progress_bar(estimated_size, effective_resume)?
        };

        // Find downloader (with extra HTTP headers if the format specifies them)
        let downloader = self
            .downloader_registry
            .find_downloader_with_headers(&format.url, format.http_headers.as_ref())
            .ok_or_else(|| OrchestratorError::NoDownloader {
                url: format.url.clone(),
            })?;

        // Validate CDN token expiry
        self.check_cdn_token_expiry(&format.url)?;

        // Build URL chain: primary + fallbacks
        let download_urls: Vec<&str> = std::iter::once(format.url.as_str())
            .chain(format.fallback_urls.iter().flatten().map(String::as_str))
            .collect();

        let mut stats = None;
        let mut last_err = None;
        for (i, download_url) in download_urls.iter().enumerate() {
            if i > 0 {
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
                    progress_bar.as_ref(),
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
        let stats = stats.unwrap();

        info!("Downloaded successfully!");
        info!("   File: {}", output_path.display());
        info!("   Stats: {stats:?}");

        Ok(Some(DownloadOutcome { stats, is_hls }))
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
                return Err(OrchestratorError::DownloadFailed(
                    rdlp_core::RdlpError::Network(format!(
                        "CDN token expired {}s ago — re-run the command to get a fresh URL",
                        now - expires
                    )),
                ));
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
}
