//! Shared download execution with CDN fallback and token validation
//!
//! Consolidates the CDN fallback retry loop and token expiry check that were
//! previously duplicated between `state.rs` and `playlist.rs`.

use super::{
    Orchestrator,
    errors::{OrchestratorError, Result},
};
use log::{debug, info, warn};
use rdlp_core::{DownloadStats, RdlpError};
use rdlp_redact::{RedactedUrl, RedactedUrlBuf};
use rdlp_security::validate_url_security;
use rdlp_types::{DownloadProtocol, Format, Fragment};
use std::path::Path;
use std::sync::Arc;

/// Validate every URL embedded in a fragment list against the SSRF allow-list.
///
/// Plugin-supplied formats can carry `Fragment.url` and `Fragment.init_url`
/// pointing at arbitrary hosts. Without this gate, a malicious or compromised
/// plugin could direct the downloader at private-network / link-local / loopback
/// / cloud-metadata addresses. Mirrors the HLS variant-URI validation added in
/// PR #257, extended to cover DASH per-fragment + per-fragment init segments
/// introduced by the WIT v0.4.0 byte-range contract.
///
/// Returns `Ok(())` if every fragment passes; otherwise returns the first
/// rejection wrapped in `OrchestratorError::DownloadFailed`, carrying the
/// offending URL for diagnostics.
fn validate_fragment_urls(fragments: &[Fragment]) -> Result<()> {
    for frag in fragments {
        validate_fragment_url_one(&frag.url, "fragment URL")?;
        if let Some(init_url) = &frag.init_url {
            validate_fragment_url_one(init_url, "fragment init_url")?;
        }
    }
    Ok(())
}

/// Single-URL gate used by `validate_fragment_urls`. Wraps
/// `rdlp_security::validate_url_security` with a `cfg(test)`-gated loopback
/// allowance so mockito-driven orchestrator integration tests (which bind to
/// `127.0.0.1`) can drive the fragment dispatch path. Mirrors the
/// `validate_resolved_url` pattern in `rdlp-extractor/src/hls/expand.rs`.
/// Production builds compile without any loopback exemption.
fn validate_fragment_url_one(url: &str, kind: &str) -> Result<()> {
    #[cfg(test)]
    {
        if let Ok(parsed) = url::Url::parse(url) {
            let scheme_ok = matches!(parsed.scheme(), "http" | "https");
            let host_loopback = parsed.host_str().is_some_and(|h| {
                h == "127.0.0.1" || h == "localhost" || h == "[::1]" || h == "::1"
            });
            if scheme_ok && host_loopback {
                return Ok(());
            }
        }
    }
    validate_url_security(url).map_err(|e| {
        OrchestratorError::DownloadFailed(RdlpError::Network {
            message: format!("Security validation failed for {kind}: {e}"),
            url: Some(RedactedUrlBuf::from(url)),
        })
    })
}

/// Whether a format's CDN-fallback URLs require HLS re-expansion rather than a
/// minimal URL-swap rebuild.
///
/// A fragment-based HLS row's `fragments` are absolute segment URLs bound to
/// the *primary* CDN host; a fallback playlist on a different CDN cannot reuse
/// them. Swapping just the playlist URL (via `Format::new`) yields a row with
/// `protocol = M3u8Native` but `fragments = None`, which trips
/// `HlsDownloader`'s "pre-resolved fragments required" guard. Such fallbacks
/// must instead be re-expanded against the fallback URL.
///
/// Progressive HTTP fallbacks are plain URL redirects (no fragments) and DASH
/// never carries `fallback_urls`, so both keep the minimal-rebuild path.
const fn fallback_needs_hls_reexpansion(format: &Format) -> bool {
    format.fragments.is_some()
        && matches!(
            format.protocol,
            DownloadProtocol::M3u8 | DownloadProtocol::M3u8Native
        )
}

/// Result of a successful download with CDN fallback.
pub(super) struct DownloadOutcome {
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

        // HLS resume is fragment-based and fully downloader-internal: the fragment
        // downloader persists/loads <output>.hls_state.json itself (see
        // rdlp_downloader::fragments). The byte-offset `resume_from` is therefore
        // unused for HLS — pass 0.
        let effective_resume = if is_hls { 0 } else { resume_from };

        let estimated_size = if is_hls {
            None // HLS uses segment-based progress, not byte-based
        } else {
            format.filesize.or(format.filesize_approx)
        };

        debug!(
            url = rdlp_security::sanitize_for_logging(&format.url).as_str(),
            headers = format.http_headers.as_ref().map_or(0, std::collections::HashMap::len),
            protocol = format.protocol.to_string().as_str();
            "Dispatching download"
        );

        // SSRF protection: validate primary URL before passing to downloader
        validate_url_security(&format.url).map_err(|e| {
            OrchestratorError::DownloadFailed(RdlpError::Network {
                message: format!("Security validation failed for format URL: {e}"),
                url: Some(RedactedUrlBuf::from(format.url.as_str())),
            })
        })?;

        // SSRF protection: validate every plugin-supplied fragment URL (and
        // per-fragment init segment URL) before dispatch. Plugins can populate
        // `Format.fragments` with arbitrary URLs via the WIT contract; without
        // this gate they could direct the downloader at private-network,
        // link-local, loopback, or cloud-metadata addresses. Mirrors the HLS
        // variant-URI validation from PR #257.
        if let Some(fragments) = &format.fragments {
            validate_fragment_urls(fragments)?;
        }

        // Find downloader (with extra HTTP headers if the format specifies them)
        let downloader = self
            .downloader_registry
            .find_downloader_with_headers(&format.url, format.http_headers.as_ref())
            .ok_or_else(|| OrchestratorError::NoDownloader {
                url: RedactedUrlBuf::from(format.url.clone()),
            })?;

        // Validate CDN token expiry
        Self::check_cdn_token_expiry(&format.url)?;

        // Build URL chain: primary + fallbacks (validate each fallback before use)
        let download_urls: Vec<&str> = std::iter::once(format.url.as_str())
            .chain(format.fallback_urls.iter().flatten().map(String::as_str))
            .collect();

        // Cloned (not borrowed) because `effective_format` only lives for its
        // loop iteration; a fallback CDN's own `filesize`/`url` — not the
        // original `format`'s — is what the size-mismatch report below must
        // compare against when a fallback is what actually served the bytes.
        // Paired in one `Option` (rather than two, each with its own
        // `.expect()`) so the "both set together or neither" loop invariant
        // is structural: there is exactly one success arm, and it sets this
        // one value.
        let mut outcome: Option<(DownloadStats, Format)> = None;
        let mut last_err = None;
        for (i, download_url) in download_urls.iter().enumerate() {
            if i > 0 {
                // SSRF protection: validate each fallback URL before use
                if let Err(e) = validate_url_security(download_url) {
                    warn!(fallback = i; "Skipping fallback URL that failed security validation: {e}");
                    last_err = Some(OrchestratorError::DownloadFailed(RdlpError::Network {
                        message: format!("Security validation failed for fallback URL: {e}"),
                        url: Some(RedactedUrlBuf::from(*download_url)),
                    }));
                    continue;
                }
                warn!(
                    fallback = i,
                    // Sanitize: fallback URLs may carry CDN tokens/credentials.
                    url:? = rdlp_security::sanitize_for_logging(download_url);
                    "Primary CDN failed, trying fallback"
                );
            }

            // For the primary URL we pass the full `format` so `download_format`
            // can use its pre-resolved fragments (HLS/DASH) and other fields.
            //
            // For fallback URLs the handling depends on the protocol:
            //
            // * Progressive HTTP — the fallback is a plain URL redirect, so a
            //   minimal `Format::new` carrying only the fallback URL is correct.
            //
            // * Fragment-based HLS — the primary `fragments` are absolute
            //   segment URLs bound to the primary CDN and cannot be reused on a
            //   different host. Re-expand the fallback playlist to obtain fresh,
            //   CDN-specific fragments. If re-expansion fails (network,
            //   encrypted, live, SSRF rejection, no segments), skip this
            //   fallback gracefully — never hand a fragments-less HLS Format to
            //   `HlsDownloader`, which would surface an internal-error message
            //   instead of an honest network failure.
            let fallback_format;
            let effective_format: &Format = if i == 0 {
                format
            } else if fallback_needs_hls_reexpansion(format) {
                // Sanitize for any log/message use — fallback URLs may carry
                // CDN credentials or signed tokens in userinfo/query.
                let safe_url = rdlp_security::sanitize_for_logging(download_url);
                if let Some(f) = self.reexpand_hls_fallback(format, download_url).await {
                    // Re-validate the re-expanded fragment URLs at the
                    // orchestrator trust boundary, mirroring the primary path's
                    // `validate_fragment_urls` gate. `expand_hls_in_place`
                    // already validates each URI internally, but routing the
                    // re-expanded list back through the boundary check keeps the
                    // invariant explicit and immune to future validator drift.
                    if let Some(frags) = &f.fragments
                        && let Err(e) = validate_fragment_urls(frags)
                    {
                        warn!(
                            fallback = i,
                            url:? = safe_url;
                            "Re-expanded HLS fallback fragment failed security validation; skipping"
                        );
                        last_err = Some(e);
                        continue;
                    }
                    fallback_format = f;
                    &fallback_format
                } else {
                    warn!(
                        fallback = i,
                        url:? = safe_url;
                        "HLS fallback playlist could not be re-expanded; skipping"
                    );
                    last_err = Some(OrchestratorError::DownloadFailed(RdlpError::Network {
                        message: format!(
                            "HLS fallback playlist could not be re-expanded: {safe_url}"
                        ),
                        url: Some(RedactedUrlBuf::from(*download_url)),
                    }));
                    continue;
                }
            } else {
                fallback_format = Format::new(
                    &format.format_id,
                    *download_url,
                    &format.ext,
                    format.protocol.clone(),
                );
                &fallback_format
            };

            match self
                .execute_download(
                    &downloader,
                    effective_format,
                    output_path,
                    effective_resume,
                    estimated_size,
                )
                .await
            {
                Ok(Some(s)) => {
                    outcome = Some((s, effective_format.clone()));
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
        #[allow(clippy::expect_used)] // loop invariant: last_err is None iff outcome was set
        let (stats, winning_format) = outcome.expect("outcome is Some when no error occurred");
        debug!("Downloaded successfully: {}", output_path.display());
        debug!("   Stats: {stats:?}");

        report_expected_size_mismatch(&winning_format, &stats);

        Ok(Some(DownloadOutcome { is_hls }))
    }

    /// Re-expand an HLS fallback playlist into a `Format` with fresh,
    /// CDN-specific pre-resolved fragments.
    ///
    /// Seeds a single-row `Format` at `fallback_url`, preserving the operator
    /// headers (e.g. `Referer`) the CDN requires for playlist + segment fetches,
    /// then runs the same `expand_hls_in_place` helper the extractors use.
    ///
    /// Returns `None` when expansion produces no usable row — network failure,
    /// encrypted/live stream, empty variants/segments, or SSRF rejection of the
    /// fallback or its variant/segment URIs — so the caller can fall through to
    /// the next fallback or an honest network error. `expand_hls_in_place` drops
    /// rows it cannot expand, so any surviving row is guaranteed to carry
    /// `fragments`.
    async fn reexpand_hls_fallback(&self, primary: &Format, fallback_url: &str) -> Option<Format> {
        let mut seed = Format::new(
            &primary.format_id,
            fallback_url,
            &primary.ext,
            primary.protocol.clone(),
        );
        seed.http_headers = primary.http_headers.clone();

        let http = Arc::clone(&self.extraction_context.http_client);
        let expanded = rdlp_extractor::hls::expand_hls_in_place(vec![seed], http).await;
        expanded.into_iter().find(|f| f.fragments.is_some())
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
        if let Some(ref fallbacks) = format.fallback_urls
            && !fallbacks.is_empty()
        {
            info!(
                count = fallbacks.len();
                "Format has fallback CDN URLs but they cannot be used \
                 with stdout (partial pipe writes are irreversible)"
            );
        }

        // SSRF protection: validate URL before passing to downloader
        validate_url_security(&format.url).map_err(|e| {
            OrchestratorError::DownloadFailed(RdlpError::Network {
                message: format!("Security validation failed for format URL: {e}"),
                url: Some(RedactedUrlBuf::from(format.url.as_str())),
            })
        })?;

        let downloader = self
            .downloader_registry
            .find_downloader_with_headers(&format.url, format.http_headers.as_ref())
            .ok_or_else(|| OrchestratorError::NoDownloader {
                url: RedactedUrlBuf::from(format.url.clone()),
            })?;

        Self::check_cdn_token_expiry(&format.url)?;

        let writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send> = Box::new(tokio::io::stdout());

        match self
            .execute_download_to_writer(&downloader, &format.url, writer)
            .await
        {
            Ok(Some(stats)) => {
                debug!("Stdout download completed: {stats:?}");
                Ok(Some(stats))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Check if the CDN token in a URL has expired.
    ///
    /// Supports two URL token formats:
    /// - `validto=TIMESTAMP` (`PornHub` direct MP4)
    /// - `~exp=TIMESTAMP~` (`PornHub` HLS / Akamai)
    ///
    /// Returns `Ok(())` if no token or token is still valid.
    fn check_cdn_token_expiry(url: &str) -> Result<()> {
        if let Some(expires) = parse_url_expiry(url) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now >= expires {
                return Err(OrchestratorError::DownloadFailed(RdlpError::Network {
                    message: format!(
                        "CDN token expired {}s ago — re-run the command to get a fresh URL",
                        now - expires
                    ),
                    url: Some(RedactedUrlBuf::from(url)),
                }));
            } else if expires - now < 60 {
                warn!("CDN token expires in less than 60 seconds — download may fail");
            }
        }
        Ok(())
    }
}

/// Compare the final downloaded byte count against `Format.filesize` as
/// reported by the extractor, and log any mismatch.
///
/// `format` MUST be the format that actually served the bytes — the caller
/// passes the winning `effective_format` (which may be a re-expanded or
/// minimally-rebuilt fallback CDN row, not the original `format` the
/// orchestrator started with), so a mismatch is never blamed on the wrong
/// CDN's metadata.
///
/// How thoroughly `stats.bytes_downloaded` has already been checked depends
/// on the transfer shape, not uniformly "server-verified": for an HTTP
/// single-file download it has been checked against the server's own
/// declared total (the per-chunk `Content-Range` span checks and the
/// `ExpectedTransfer` / `verify_output_size` byte-count and final-size checks
/// in `rdlp-downloader`, #674); for a fragmented (HLS/DASH) or unsized
/// (chunked, no `Content-Length`) transfer there is no such total to check
/// against, and `bytes_downloaded` is simply the byte count received. Either
/// way, a mismatch against `Format.filesize` here means the EXTRACTOR's size
/// estimate was wrong, not that the file is corrupt — this function is
/// reported-only: never deletes the file, never fails the download, and does
/// no file I/O of its own since `stats.bytes_downloaded` already carries the
/// actual final size (a resume's `downloaded` counter starts at the resume
/// offset, so it is the whole-file count, not just the resumed tail).
fn report_expected_size_mismatch(format: &Format, stats: &DownloadStats) {
    let actual = stats.bytes_downloaded;
    match format.filesize {
        Some(expected) if expected != actual => {
            warn!(
                "extractor reported {expected} bytes for {}; server delivered {actual}",
                RedactedUrl::new(&format.url)
            );
        }
        Some(_) => {}
        None => {
            debug!(
                "no extractor-reported size for {}; the server's {actual} bytes is the sole \
                 authority",
                RedactedUrl::new(&format.url)
            );
        }
    }
}

/// Parse CDN token expiry timestamp from a URL.
///
/// Supports two formats:
/// - `validto=TIMESTAMP` (`PornHub` direct MP4)
/// - `~exp=TIMESTAMP~` (`PornHub` HLS / Akamai)
fn parse_url_expiry(url: &str) -> Option<u64> {
    /// Extract a `u64` timestamp immediately following `prefix` in `url`.
    fn extract_ts_after(url: &str, prefix: &str) -> Option<u64> {
        let rest = &url[url.find(prefix)? + prefix.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
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

    /// Build a single fragment with the given `url` + optional `init_url` for tests.
    fn frag(url: &str, init_url: Option<&str>) -> rdlp_types::Fragment {
        rdlp_types::Fragment {
            url: url.to_string(),
            byte_range: None,
            init_url: init_url.map(str::to_string),
            init_byte_range: None,
            duration: None,
            filesize: None,
        }
    }

    /// Plugin-supplied fragment whose `init_url` points at the AWS instance
    /// metadata endpoint must be rejected before reaching the downloader.
    #[test]
    fn test_fragment_init_url_ssrf_rejected() {
        let frags = vec![frag(
            "https://cdn.example.com/seg1.m4s",
            Some("http://169.254.169.254/latest/meta-data/"),
        )];
        let res = validate_fragment_urls(&frags);
        assert!(
            res.is_err(),
            "Expected rejection of fragment with private-IP init_url"
        );
    }

    /// Plugin-supplied fragment whose primary `url` points at an RFC-1918
    /// address must be rejected before reaching the downloader.
    #[test]
    fn test_fragment_url_ssrf_rejected() {
        let frags = vec![
            frag("https://cdn.example.com/seg1.m4s", None),
            frag("http://10.0.0.1/segment.m4s", None),
        ];
        let res = validate_fragment_urls(&frags);
        assert!(
            res.is_err(),
            "Expected rejection of fragment with private-IP url"
        );
    }

    /// Build a format with the given protocol and optional fragments for the
    /// `fallback_needs_hls_reexpansion` decision tests.
    fn fmt_with(protocol: DownloadProtocol, with_fragments: bool) -> Format {
        let mut f = Format::new("id", "https://cdn.example.com/v", "mp4", protocol);
        if with_fragments {
            f.fragments = Some(vec![frag("https://cdn.example.com/seg0.ts", None)]);
        }
        f
    }

    /// Positive: a fragment-based HLS format (`M3u8` or `M3u8Native`) must be flagged
    /// for re-expansion — a minimal URL-swap would drop its CDN-specific
    /// fragments and trip the downloader's "pre-resolved fragments required" guard.
    #[test]
    fn test_fallback_reexpansion_needed_for_hls_with_fragments() {
        assert!(fallback_needs_hls_reexpansion(&fmt_with(
            DownloadProtocol::M3u8Native,
            true
        )));
        assert!(fallback_needs_hls_reexpansion(&fmt_with(
            DownloadProtocol::M3u8,
            true
        )));
    }

    /// Negative: progressive HTTP fallbacks are plain URL redirects with no
    /// fragments — the minimal-rebuild path is correct, no re-expansion.
    #[test]
    fn test_fallback_reexpansion_not_needed_for_progressive_http() {
        assert!(!fallback_needs_hls_reexpansion(&fmt_with(
            DownloadProtocol::Https,
            false
        )));
        assert!(!fallback_needs_hls_reexpansion(&fmt_with(
            DownloadProtocol::Http,
            false
        )));
    }

    /// Negative: an HLS-protocol row WITHOUT fragments cannot be re-expanded as
    /// a fallback (nothing to re-resolve against), so it must not be flagged —
    /// guards against an infinite/no-op re-expansion attempt.
    #[test]
    fn test_fallback_reexpansion_not_needed_for_hls_without_fragments() {
        assert!(!fallback_needs_hls_reexpansion(&fmt_with(
            DownloadProtocol::M3u8Native,
            false
        )));
    }

    /// Negative: DASH (`HttpDashSegments`) never carries `fallback_urls`, and its
    /// re-expansion is body-based (not URL-based), so even a fragment-bearing
    /// DASH row must not take the HLS re-expansion path.
    #[test]
    fn test_fallback_reexpansion_not_needed_for_dash() {
        assert!(!fallback_needs_hls_reexpansion(&fmt_with(
            DownloadProtocol::HttpDashSegments,
            true
        )));
    }

    /// Build a `DownloadStats` carrying only the byte count the mismatch
    /// check reads; the other fields are irrelevant to it.
    fn stats_with_bytes(bytes: u64) -> DownloadStats {
        DownloadStats::new(bytes, std::time::Duration::from_secs(1), 0)
    }

    /// #674: a known `filesize` that matches the actual bytes delivered is
    /// silently accepted — no assertion beyond "does not panic" is available
    /// without a log-capturing harness in this crate, so the log line this
    /// exercises is documented instead: at `expected == actual` neither the
    /// `warn!` nor the `debug!` branch in `report_expected_size_mismatch`
    /// fires.
    #[test]
    fn size_mismatch_report_accepts_a_matching_filesize() {
        let mut format = Format::new(
            "id",
            "https://cdn.example.com/v.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        format.filesize = Some(2048);
        report_expected_size_mismatch(&format, &stats_with_bytes(2048));
    }

    /// #674: a `filesize` that DISAGREES with the delivered byte count must
    /// still complete without failing or touching any file — this function
    /// takes no path and does no I/O, so "no failure" is the whole contract.
    /// Expected log line: `warn!("extractor reported 2048 bytes for
    /// https://cdn.example.com/v.mp4; server delivered 4096")`.
    #[test]
    fn size_mismatch_report_does_not_fail_on_a_mismatched_filesize() {
        let mut format = Format::new(
            "id",
            "https://cdn.example.com/v.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        format.filesize = Some(2048);
        // The call itself is the assertion: a panic or an `Err` return would
        // fail the test. There is nothing else to observe from outside.
        report_expected_size_mismatch(&format, &stats_with_bytes(4096));
    }

    /// #674: no extractor-reported size (`filesize: None`) takes the `debug!`
    /// branch and must not be treated as a mismatch. Expected log line:
    /// `debug!("no extractor-reported size for https://cdn.example.com/v.mp4;
    /// the server's 4096 bytes is the sole authority")`.
    #[test]
    fn size_mismatch_report_handles_no_reported_size() {
        let format = Format::new(
            "id",
            "https://cdn.example.com/v.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        assert_eq!(format.filesize, None);
        report_expected_size_mismatch(&format, &stats_with_bytes(4096));
    }

    /// Positive path: all-public-URL fragments validate successfully.
    #[test]
    fn test_fragment_all_public_urls_accepted() {
        let frags = vec![
            frag(
                "https://cdn.example.com/seg1.m4s",
                Some("https://cdn.example.com/init.mp4"),
            ),
            frag("https://cdn.example.com/seg2.m4s", None),
            frag("https://media.example.com/seg3.m4s", None),
        ];
        let res = validate_fragment_urls(&frags);
        assert!(
            res.is_ok(),
            "Expected all-public fragment set to pass, got {res:?}"
        );
    }
}
