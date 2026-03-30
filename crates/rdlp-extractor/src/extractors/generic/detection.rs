//! Detection pipeline and shared types for the generic extractor.
//!
//! Each detection strategy implements [`DetectionStrategy`] and extracts
//! formats from already-fetched page content. Strategies are sync — all
//! async I/O happens before the detection pipeline runs.

use scraper::Html;
use url::Url;

// ============================================================================
// Shared Types
// ============================================================================

/// Confidence level for a detected format.
///
/// Used for sorting when the same URL is detected by multiple strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Confidence {
    /// Low confidence — regex scan of page source, likely false positive
    Low = 0,
    /// Medium confidence — HTML element or JS player config
    Medium = 1,
    /// High confidence — structured data (JSON-LD, OpenGraph, Content-Type)
    High = 2,
}

/// A format detected by a strategy, before conversion to [`rdlp_types::Format`].
#[derive(Debug, Clone)]
pub(crate) struct DetectedFormat {
    /// Absolute URL of the media resource
    pub url: String,
    /// File extension (e.g., "mp4", "m3u8"), if detectable
    pub ext: Option<String>,
    /// Quality label (e.g., "720p", "1080p"), if available
    pub quality: Option<String>,
    /// Detection confidence
    pub confidence: Confidence,
    /// Strategy name that detected this format (for logging)
    pub source: &'static str,
}

/// Borrowed view of fetched page data, passed to all detection strategies.
///
/// All fields are borrowed — strategies run synchronously on already-fetched
/// data, so no `.await` points exist while `PageContext` is alive. This is
/// critical because [`scraper::Html`] is `!Send`.
pub(crate) struct PageContext<'a> {
    /// Parsed page URL
    #[allow(dead_code)]
    pub url: &'a Url,
    /// Base URL for resolving relative links (from `<base>` tag or page URL)
    pub base_url: &'a Url,
    /// Parsed HTML document
    pub html: &'a Html,
    /// Raw HTML source (needed for JS regex scanning)
    pub raw_html: &'a str,
}

// ============================================================================
// Detection Strategy Trait
// ============================================================================

/// A single detection strategy that can extract media formats from page content.
///
/// Strategies are **sync** and **infallible** — parse failures produce an
/// empty `Vec`, not an error. Only the top-level `extract()` method returns
/// errors (for network failures or zero total formats).
pub(crate) trait DetectionStrategy: Send + Sync {
    /// Human-readable name for logging (e.g., "JSON-LD", "OpenGraph")
    fn name(&self) -> &'static str;

    /// Extract formats from page content. Returns empty `Vec` on no match.
    fn detect(&self, ctx: &PageContext<'_>) -> Vec<DetectedFormat>;
}

// ============================================================================
// Pipeline Runner
// ============================================================================

/// Run all detection strategies and return deduplicated formats.
pub(crate) fn run_detection_pipeline(
    strategies: &[Box<dyn DetectionStrategy>],
    ctx: &PageContext<'_>,
) -> Vec<DetectedFormat> {
    let mut formats: Vec<DetectedFormat> = Vec::new();

    for strategy in strategies {
        let detected = strategy.detect(ctx);
        if !detected.is_empty() {
            log::debug!(
                "generic extractor: {} detected {} format(s)",
                strategy.name(),
                detected.len()
            );
        }
        formats.extend(detected);
    }

    // Deduplicate by URL, keeping highest-confidence entry
    formats.sort_by(|a, b| a.url.cmp(&b.url).then(b.confidence.cmp(&a.confidence)));
    formats.dedup_by(|a, b| a.url == b.url);

    formats
}

// ============================================================================
// URL Resolution Helper
// ============================================================================

/// Resolve a possibly-relative URL against the page's base URL.
///
/// Returns `None` for empty strings, `data:` URIs, and `javascript:` pseudo-URLs.
pub(crate) fn resolve_url(base: &Url, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("data:") || trimmed.starts_with("javascript:") {
        return None;
    }
    // Handle protocol-relative URLs
    if trimmed.starts_with("//") {
        return Some(format!("https:{trimmed}"));
    }
    base.join(trimmed).ok().map(|u| u.to_string())
}

/// Infer file extension from a URL path.
pub(crate) fn ext_from_url(url: &str) -> Option<String> {
    let path = Url::parse(url).ok()?.path().to_string();
    // Strip query params that might be appended after the extension
    let path = path.split('?').next().unwrap_or(&path);
    let ext = path.rsplit('.').next()?;
    let ext = ext.to_lowercase();
    match ext.as_str() {
        "mp4" | "webm" | "mkv" | "m3u8" | "mpd" | "ts" | "flv" | "avi" | "mov" | "m4v"
        | "mp3" | "m4a" | "ogg" | "opus" | "wav" | "flac" | "aac" => Some(ext),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_absolute_url() {
        let base = Url::parse("https://example.com/page").unwrap();
        assert_eq!(
            resolve_url(&base, "https://cdn.example.com/video.mp4"),
            Some("https://cdn.example.com/video.mp4".to_string())
        );
    }

    #[test]
    fn resolve_relative_url() {
        let base = Url::parse("https://example.com/page/").unwrap();
        assert_eq!(
            resolve_url(&base, "/video.mp4"),
            Some("https://example.com/video.mp4".to_string())
        );
    }

    #[test]
    fn resolve_protocol_relative_url() {
        let base = Url::parse("https://example.com/page").unwrap();
        assert_eq!(
            resolve_url(&base, "//cdn.example.com/video.mp4"),
            Some("https://cdn.example.com/video.mp4".to_string())
        );
    }

    #[test]
    fn resolve_rejects_data_uri() {
        let base = Url::parse("https://example.com/").unwrap();
        assert_eq!(resolve_url(&base, "data:video/mp4;base64,AAAA"), None);
    }

    #[test]
    fn resolve_rejects_javascript_uri() {
        let base = Url::parse("https://example.com/").unwrap();
        assert_eq!(resolve_url(&base, "javascript:void(0)"), None);
    }

    #[test]
    fn resolve_rejects_empty() {
        let base = Url::parse("https://example.com/").unwrap();
        assert_eq!(resolve_url(&base, ""), None);
        assert_eq!(resolve_url(&base, "  "), None);
    }

    #[test]
    fn ext_from_url_extracts_known() {
        assert_eq!(
            ext_from_url("https://cdn.example.com/video.mp4"),
            Some("mp4".to_string())
        );
        assert_eq!(
            ext_from_url("https://cdn.example.com/stream.m3u8"),
            Some("m3u8".to_string())
        );
        assert_eq!(
            ext_from_url("https://cdn.example.com/audio.opus"),
            Some("opus".to_string())
        );
    }

    #[test]
    fn ext_from_url_rejects_unknown() {
        assert_eq!(ext_from_url("https://example.com/page.html"), None);
        assert_eq!(ext_from_url("https://example.com/page.php"), None);
        assert_eq!(ext_from_url("https://example.com/page"), None);
    }

    #[test]
    fn ext_from_url_case_insensitive() {
        assert_eq!(
            ext_from_url("https://cdn.example.com/VIDEO.MP4"),
            Some("mp4".to_string())
        );
    }

    #[test]
    fn dedup_keeps_highest_confidence() {
        let formats = vec![
            DetectedFormat {
                url: "https://example.com/video.mp4".to_string(),
                ext: Some("mp4".to_string()),
                quality: None,
                confidence: Confidence::Low,
                source: "link_scan",
            },
            DetectedFormat {
                url: "https://example.com/video.mp4".to_string(),
                ext: Some("mp4".to_string()),
                quality: None,
                confidence: Confidence::High,
                source: "og:video",
            },
        ];

        // Simulate dedup logic
        let mut formats = formats;
        formats.sort_by(|a, b| a.url.cmp(&b.url).then(b.confidence.cmp(&a.confidence)));
        formats.dedup_by(|a, b| a.url == b.url);

        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].confidence, Confidence::High);
        assert_eq!(formats[0].source, "og:video");
    }
}
