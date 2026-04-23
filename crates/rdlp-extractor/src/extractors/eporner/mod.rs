//! EPorner extractor (XHR-authenticated primary path + /dload/ DOM-scrape fallback).
//!
//! ## Format paths
//! - **Primary**: `GET /xhr/video/{id}?hash={calc_hash}&device=generic&domain=www.eporner.com`
//!   Returns JSON with a `sources.mp4` map keyed by label (e.g. `"1080p HD"`) each having
//!   a `src` URL.  Also may contain an `hls` key.
//! - **Fallback**: `<a href="/dload/{id}/{height}/{filename}">` links scraped from the page.
//!
//! ## Performers
//! EPorner embeds `actor[]` inside the page's JSON-LD `VideoObject`.

pub mod hash;
pub mod patterns;
pub mod search;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::{DownloadProtocol, Format, InfoDict};
use regex::Regex;
use scraper::Html;
use serde_json::Value;
use std::sync::LazyLock;

use crate::base::common::BaseExtractor;
use hash::calc_hash;

/// Extractor name.
const EPORNER_NAME: &str = "EPorner";
/// Extractor priority (higher than generic fallback).
const EPORNER_PRIORITY: i32 = 100;
/// EPorner root URL.
const EPORNER_ROOT: &str = "https://www.eporner.com";

/// Regex that extracts the 32-char hex page hash from a script block.
///
/// Matches both `hash = "…"` and `hash: "…"` variants and supports single quotes.
static PAGE_HASH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"hash\s*[:=]\s*["']([0-9a-f]{32})"#).unwrap());

/// Regex for `/dload/` download links.
///
/// Captures: (1) relative path, (2) height digits.
/// The `-av1` suffix is optional.
static DLOAD_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href="(/dload/[^"]+?-(\d+)p(?:-av1)?\.mp4)""#).unwrap());

// ============================================================================
// EPornerExtractor struct
// ============================================================================

/// EPorner video extractor.
#[derive(Default)]
pub struct EPornerExtractor;

impl EPornerExtractor {
    /// Create a new EPorner extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

// ============================================================================
// XHR format parsing
// ============================================================================

/// Build the XHR URL for a given video id and computed hash.
fn xhr_url(id: &str, hash: &str) -> String {
    format!(
        "{EPORNER_ROOT}/xhr/video/{id}?hash={hash}&device=generic&domain=www.eporner.com&fallback=false"
    )
}

/// Parse formats from the XHR JSON response.
///
/// The real EPorner XHR response nests MP4 formats under `sources.mp4` as a map
/// keyed by human-readable label (e.g. `"1080p HD"`).  Each entry has a `src`
/// and an optional `labelShort` field.  An HLS stream would appear as a plain
/// object with a `src` under `sources.hls`.
pub(crate) fn parse_xhr_formats(value: &Value) -> Vec<Format> {
    let mut formats = Vec::new();

    let sources = match value.get("sources") {
        Some(s) => s,
        None => return formats,
    };

    // HLS stream (if present)
    if let Some(hls) = sources.get("hls")
        && let Some(src) = hls.get("src").and_then(|v| v.as_str())
        && !src.is_empty()
    {
        formats.push(Format::new(
            "hls",
            src,
            "m3u8",
            DownloadProtocol::M3u8Native,
        ));
    }

    // MP4 streams — nested object keyed by label
    if let Some(mp4_obj) = sources.get("mp4").and_then(|v| v.as_object()) {
        for (label, entry) in mp4_obj {
            let src = match entry.get("src").and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            // Use labelShort if present, else derive from key
            let short = entry
                .get("labelShort")
                .and_then(|v| v.as_str())
                .unwrap_or(label.as_str());
            let height: Option<u32> = short
                .trim_end_matches('p')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .parse()
                .ok();
            let format_id = format!("mp4-{short}");
            let mut f = Format::new(format_id, src, "mp4", DownloadProtocol::Https);
            f.height = height;
            formats.push(f);
        }
    }

    formats
}

// ============================================================================
// /dload/ fallback parsing
// ============================================================================

/// Parse `/dload/` download links from the page HTML.
///
/// Returns one `Format` per unique link found.  AV1 variants are included and
/// identified by the `-av1` suffix in the filename.
pub(crate) fn parse_dload_formats(page_url: &str, html: &str) -> Vec<Format> {
    let origin = url::Url::parse(page_url)
        .ok()
        .and_then(|u| {
            let scheme = u.scheme();
            let host = u.host_str()?;
            Some(format!("{scheme}://{host}"))
        })
        .unwrap_or_else(|| EPORNER_ROOT.to_string());

    let mut formats = Vec::new();
    for cap in DLOAD_LINK.captures_iter(html) {
        let rel_path = &cap[1];
        let height: Option<u32> = cap[2].parse().ok();
        let is_av1 = rel_path.contains("-av1.");
        let codec_tag = if is_av1 { "av1" } else { "h264" };
        let format_id = format!(
            "dload-{}p-{codec_tag}",
            height.map_or_else(|| "unknown".to_string(), |h| h.to_string())
        );
        let absolute_url = format!("{origin}{rel_path}");
        let mut f = Format::new(format_id, absolute_url, "mp4", DownloadProtocol::Https);
        f.height = height;
        f.vcodec = Some(codec_tag.to_string());
        formats.push(f);
    }
    formats
}

// ============================================================================
// JSON-LD metadata
// ============================================================================

/// Parse title, ISO 8601 duration, view count, and actors from the page's JSON-LD.
///
/// Uses raw `serde_json::Value` parsing to be tolerant of EPorner's non-standard
/// `interactionType` object (which breaks the shared typed deserializer).
///
/// Returns `(title, duration_iso, views, actors)`.
pub(crate) fn parse_json_ld(
    html: &str,
) -> (Option<String>, Option<String>, Option<u64>, Vec<String>) {
    use crate::base::common::JSONLD_SELECTOR;

    let document = Html::parse_document(html);
    for script_elem in document.select(&JSONLD_SELECTOR) {
        let json_text = script_elem.text().collect::<String>();
        let Ok(v) = serde_json::from_str::<Value>(&json_text) else {
            continue;
        };

        let video = find_video_object(&v);
        let Some(obj) = video else { continue };

        let title = obj.get("name").and_then(|v| v.as_str()).map(str::to_string);
        let duration_iso = obj
            .get("duration")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let views = obj
            .get("interactionStatistic")
            .and_then(extract_watch_count);

        let actors = obj
            .get("actor")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        return (title, duration_iso, views, actors);
    }
    (None, None, None, vec![])
}

/// Find a `VideoObject` in a JSON-LD value (single or `@graph`).
fn find_video_object(v: &Value) -> Option<&Value> {
    if v.get("@type").and_then(|t| t.as_str()) == Some("VideoObject") {
        return Some(v);
    }
    if let Some(graph) = v.get("@graph").and_then(|g| g.as_array()) {
        return graph
            .iter()
            .find(|o| o.get("@type").and_then(|t| t.as_str()) == Some("VideoObject"));
    }
    None
}

/// Extract `userInteractionCount` for a WatchAction from `interactionStatistic`.
///
/// Handles both a single object and an array.
fn extract_watch_count(stat: &Value) -> Option<u64> {
    let check = |item: &Value| -> Option<u64> {
        let action_type_matches = item
            .get("interactionType")
            .map(|it| {
                // interactionType can be a string OR {"@type": "…WatchAction"}
                it.as_str()
                    .map(|s| s.contains("WatchAction"))
                    .unwrap_or_else(|| {
                        it.get("@type")
                            .and_then(|t| t.as_str())
                            .map(|t| t.contains("WatchAction"))
                            .unwrap_or(false)
                    })
            })
            .unwrap_or(false);
        if action_type_matches {
            item.get("userInteractionCount").and_then(|c| c.as_u64())
        } else {
            None
        }
    };

    if let Some(arr) = stat.as_array() {
        arr.iter().find_map(check)
    } else {
        check(stat)
    }
}

// ============================================================================
// InfoExtractor impl
// ============================================================================

/// Build an `InfoDict` from an already-fetched page HTML.
///
/// Tries the XHR path first; falls back to `/dload/` scraping if XHR returned
/// no usable formats.
async fn build_info(
    id: &str,
    page_url: &str,
    html: &str,
    ctx: &ExtractionContext,
) -> Result<InfoDict> {
    let (title, duration_iso, views, actors) = parse_json_ld(html);
    let title = title.unwrap_or_else(|| "Untitled".to_string());

    // --- Primary path: XHR ---
    let xhr_formats = if let Some(raw_hash) = PAGE_HASH
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
    {
        if let Some(computed_hash) = calc_hash(&raw_hash) {
            let url = xhr_url(id, &computed_hash);
            debug!("[eporner] XHR: {url}");
            match BaseExtractor::fetch_webpage(&url, ctx).await {
                Ok(body) => match serde_json::from_str::<Value>(&body) {
                    Ok(json) => {
                        let fmts = parse_xhr_formats(&json);
                        debug!("[eporner] XHR returned {} formats", fmts.len());
                        fmts
                    }
                    Err(e) => {
                        debug!("[eporner] XHR JSON parse error: {e}");
                        vec![]
                    }
                },
                Err(e) => {
                    debug!("[eporner] XHR fetch error: {e:#}");
                    vec![]
                }
            }
        } else {
            debug!("[eporner] calc_hash failed for raw hash");
            vec![]
        }
    } else {
        debug!("[eporner] no page hash found, skipping XHR");
        vec![]
    };

    // --- Fallback: /dload/ links ---
    let formats = if xhr_formats.is_empty() {
        debug!("[eporner] falling back to /dload/ scraping");
        let dload = parse_dload_formats(page_url, html);
        if dload.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!("No formats found for EPorner video: {page_url}"),
                url: Some(page_url.to_string()),
            });
        }
        dload
    } else {
        xhr_formats
    };

    let mut info = InfoDict::new(id, title, EPORNER_NAME, page_url);
    info.view_count = views;
    info.duration = duration_iso
        .as_deref()
        .and_then(BaseExtractor::parse_iso8601_duration);
    info.age_limit = Some(18);
    info.formats = formats;
    info.propagate_duration();

    if !actors.is_empty() {
        info.actors = actors;
    }

    Ok(info)
}

#[async_trait]
impl InfoExtractor for EPornerExtractor {
    fn name(&self) -> &str {
        EPORNER_NAME
    }

    fn valid_url(&self) -> &Regex {
        &patterns::VIDEO_URL
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::VIDEO_URL.is_match(url)
    }

    fn priority(&self) -> i32 {
        EPORNER_PRIORITY
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let id = patterns::VIDEO_URL
            .captures(url)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| RdlpError::Extraction {
                message: format!("EPorner: could not extract video id from URL: {url}"),
                url: Some(url.to_string()),
            })?;

        let html =
            BaseExtractor::fetch_webpage(url, ctx)
                .await
                .map_err(|e| RdlpError::Extraction {
                    message: format!("EPorner: page fetch failed: {e:#}"),
                    url: Some(url.to_string()),
                })?;

        build_info(&id, url, &html, ctx).await.map_err(|e| match e {
            RdlpError::Extraction { .. } | RdlpError::Network { .. } | RdlpError::Http { .. } => e,
            other => RdlpError::Extraction {
                message: format!("{other:#}"),
                url: Some(url.to_string()),
            },
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures recorded live on 2026-04-23 from www.eporner.com.
    const XHR_FIXTURE: &str = include_str!("tests/eporner_xhr.json");
    const PAGE_FIXTURE: &str = include_str!("tests/eporner_video_page.html");

    #[test]
    fn matches_url_patterns() {
        let ext = EPornerExtractor::new();
        assert!(ext.suitable("https://www.eporner.com/video-svXh0Ne27Ig/harleysummers/"));
        assert!(ext.suitable("https://www.eporner.com/hd-porn/95008/some-slug/"));
        assert!(ext.suitable("https://www.eporner.com/embed/svXh0Ne27Ig/"));
        assert!(!ext.suitable("https://www.xvideos.com/video123/slug/"));
    }

    #[test]
    fn parse_xhr_formats_finds_mp4() {
        let json: Value = serde_json::from_str(XHR_FIXTURE).expect("valid json");
        let formats = parse_xhr_formats(&json);
        assert!(
            !formats.is_empty(),
            "Expected at least one format from XHR fixture"
        );
        // All should be mp4 or m3u8
        for f in &formats {
            assert!(
                f.ext == "mp4" || f.ext == "m3u8",
                "Unexpected ext: {}",
                f.ext
            );
        }
    }

    #[test]
    fn parse_dload_formats_from_page() {
        let formats = parse_dload_formats(
            "https://www.eporner.com/video-svXh0Ne27Ig/harleysummers/",
            PAGE_FIXTURE,
        );
        assert!(
            !formats.is_empty(),
            "Expected /dload/ formats from page fixture"
        );
        assert!(
            formats.iter().any(|f| f.height.is_some()),
            "Expected at least one format with a height"
        );
    }

    #[test]
    fn parse_json_ld_exposes_actors() {
        let (title, _duration, _views, actors) = parse_json_ld(PAGE_FIXTURE);
        assert!(title.is_some(), "Expected a title from JSON-LD");
        assert!(
            !actors.is_empty(),
            "Expected at least one actor from live JSON-LD fixture"
        );
    }
}
