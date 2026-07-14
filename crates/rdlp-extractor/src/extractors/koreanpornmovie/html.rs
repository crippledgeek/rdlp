//! HTML scraping helpers for the KoreanPornMovie extractor.
//!
//! Extracted from mod.rs per spec
//! docs/superpowers/specs/2026-05-22-file-cohesion-policy-design.md
//! to keep the extractor module focused on the InfoExtractor /
//! SearchExtractor implementations.

use super::*;

/// Convert a WP REST API post to a SearchResultPreview with all available data.
pub(super) fn wp_post_to_preview(post: WpPost, duration: Option<f64>) -> SearchResultPreview {
    let (thumbnail_url, actors) = match post.embedded {
        Some(embed) => {
            let thumb = embed
                .featured_media
                .into_iter()
                .next()
                .and_then(|m| m.source_url);

            // Extract actor names from embedded wp:term (taxonomy = "actors")
            let actors: Vec<String> = embed
                .terms
                .into_iter()
                .flatten()
                .filter(|t| t.taxonomy == "actors")
                .map(|t| html_entities_decode(&t.name))
                .collect();

            (thumb, actors)
        }
        None => (None, Vec::new()),
    };

    let upload_date = post.date.split('T').next().map(|s| s.to_string());

    let title = html_entities_decode(&post.title.rendered);

    SearchResultPreview {
        title,
        video_url: post.link,
        thumbnail_url,
        duration,
        uploader: None,
        uploader_url: None,
        actors,
        view_count: None,
        upload_date,
    }
}

/// Extract X-WP-TotalPages from response headers.
pub(super) fn extract_total_pages(response: &wreq::Response) -> u32 {
    response
        .headers()
        .get("x-wp-totalpages")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Scrape durations from an HTML listing response (best-effort, non-fatal).
pub(super) async fn scrape_durations_from_html_response(
    result: std::result::Result<wreq::Response, wreq::Error>,
) -> std::collections::HashMap<String, f64> {
    let text = match result {
        Ok(resp) => resp.text().await.unwrap_or_default(),
        Err(e) => {
            // Network failure was previously indistinguishable from "page
            // had no durations" — surface it at debug so investigators
            // can find why the duration map is empty.
            log::debug!("[KoreanPornMovie] duration scrape failed: {e}");
            return std::collections::HashMap::new();
        }
    };
    scrape_durations_from_html(&text)
}

/// Parse HTML listing page and extract a URL → duration map.
pub(super) fn scrape_durations_from_html(
    html_text: &str,
) -> std::collections::HashMap<String, f64> {
    let html = Html::parse_document(html_text);
    let mut map = std::collections::HashMap::new();

    for article in html.select(&SEARCH_ARTICLE_SELECTOR) {
        let url = article
            .select(&SEARCH_LINK_SELECTOR)
            .next()
            .and_then(|a| a.value().attr("href"));
        let duration = article
            .select(&SEARCH_DURATION_SELECTOR)
            .next()
            .map(|d| d.text().collect::<String>())
            .and_then(|d| BaseExtractor::parse_duration(d.trim()));

        if let (Some(url), Some(dur)) = (url, duration) {
            map.insert(url.to_string(), dur);
        }
    }

    map
}

/// Decode basic HTML entities in WP REST API title (e.g., `&#8211;` → `–`).
pub(super) fn html_entities_decode(s: &str) -> String {
    s.replace("&#8211;", "–")
        .replace("&#8212;", "—")
        .replace("&#8216;", "'")
        .replace("&#8217;", "'")
        .replace("&#8220;", "\u{201c}")
        .replace("&#8221;", "\u{201d}")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
}

// ============================================================================
// Format Extraction Helpers
// ============================================================================

/// Extract video formats from the page HTML.
///
/// Looks for the `clean-tube-player` iframe, decodes its base64 `q` parameter,
/// and extracts the video source URL.
pub(super) fn extract_formats_from_html(html: &Html, _page_url: &str) -> Vec<Format> {
    let mut formats = Vec::new();

    // Try to get width/height from the decoded player tag
    let decoded_tag = html
        .select(&PLAYER_IFRAME_SELECTOR)
        .next()
        .and_then(|iframe| iframe.value().attr("src"))
        .and_then(decode_player_iframe);

    let (tag_width, tag_height) = decoded_tag
        .as_deref()
        .map(extract_dimensions_from_tag)
        .unwrap_or((None, None));

    // Strategy 1: Schema.org contentURL meta tag (direct MP4)
    if let Some(content_url) = meta_content(html, &META_CONTENT_URL_SELECTOR)
        && !content_url.is_empty()
    {
        let mut format = make_video_format("kpm-direct", &content_url);
        format.width = tag_width;
        format.height = tag_height;
        formats.push(format);
    }

    // Strategy 2: Decode clean-tube-player iframe base64 (may contain multiple sources)
    if let Some(ref decoded) = decoded_tag {
        for (i, url) in extract_urls_from_decoded_tag(decoded)
            .into_iter()
            .enumerate()
        {
            if !formats.iter().any(|f| f.url == url) {
                let format_id = if i == 0 {
                    "kpm-player".to_string()
                } else {
                    format!("kpm-player-{}", i + 1)
                };
                let mut format = make_video_format(&format_id, &url);
                format.width = tag_width;
                format.height = tag_height;
                formats.push(format);
            }
        }
    }

    // Strategy 3: embedURL (external embed — log but don't add as format)
    if formats.is_empty()
        && let Some(embed_url) = meta_content(html, &META_EMBED_URL_SELECTOR)
    {
        log::info!(
            "[KoreanPornMovie] Video is an external embed: {} — try that URL directly",
            embed_url
        );
        log::info!(
            "[KoreanPornMovie] Use: rdlp \"{}\" instead",
            embed_url.replace("/embed/", "/view_video.php?viewkey=")
        );
    }

    formats
}

/// Create a Format with video codec markers set so the UI shows it as video, not audio-only.
pub(super) fn make_video_format(format_id: &str, url: &str) -> Format {
    let protocol = url::Url::parse(url)
        .map(|u| crate::base::common::protocol_for_url(&u))
        .unwrap_or(DownloadProtocol::Https);
    let ext = url
        .split('.')
        .next_back()
        .unwrap_or("mp4")
        .split('?')
        .next()
        .unwrap_or("mp4");

    let mut format = Format::new(format_id, url, ext, protocol);
    // Mark as video (not audio-only) — actual codec determined at download time
    format.vcodec = Codec::from("video".to_string());
    format.acodec = Codec::from("audio".to_string());
    format
}

/// Extract width/height from a decoded `<video width="640" height="264">` tag.
pub(super) fn extract_dimensions_from_tag(tag: &str) -> (Option<u32>, Option<u32>) {
    static WIDTH_RE: Lazy<Regex> = lazy_regex!(r#"width=["'](\d+)["']"#);
    static HEIGHT_RE: Lazy<Regex> = lazy_regex!(r#"height=["'](\d+)["']"#);

    let width = WIDTH_RE
        .captures(tag)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok());
    let height = HEIGHT_RE
        .captures(tag)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok());

    (width, height)
}

/// Decode the `player-x.php?q=<base64>` iframe URL.
///
/// Returns the URL-decoded `tag` parameter value (the actual player HTML).
pub(super) fn decode_player_iframe(iframe_src: &str) -> Option<String> {
    let url = url::Url::parse(iframe_src).ok()?;
    let q = url.query_pairs().find(|(k, _)| k == "q")?.1;

    // Base64 decode
    let decoded_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, q.as_bytes()).ok()?;
    let decoded = String::from_utf8(decoded_bytes).ok()?;

    // URL-decode the tag parameter — read the first `tag` pair directly instead
    // of collecting every param into a Vec<(String, String)> just to scan it.
    url::form_urlencoded::parse(decoded.as_bytes())
        .find(|(k, _)| k == "tag")
        .map(|(_, v)| v.into_owned())
}

/// Extract media URLs from the decoded player tag HTML.
///
/// Handles both `<video><source src="...">` and `<iframe src="...">` patterns.
pub(super) fn extract_urls_from_decoded_tag(tag_html: &str) -> Vec<String> {
    static SRC_PATTERN: Lazy<Regex> = lazy_regex!(r#"src=["']([^"']+)["']"#);

    SRC_PATTERN
        .captures_iter(tag_html)
        .filter_map(|caps| {
            let url = caps.get(1)?.as_str();
            looks_like_media(url).then(|| url.to_string())
        })
        .collect()
}

/// Allow-list filter for media URLs found in decoded player iframes.
///
/// Matches by parsed path-segment extension (NOT substring). The
/// `koreanporn.stream` host carve-out is preserved because some embeds
/// link to opaque embed pages on that host that resolve to media at
/// load time.
pub(super) fn looks_like_media(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed
        .host_str()
        .is_some_and(|h| h == "koreanporn.stream" || h.ends_with(".koreanporn.stream"))
    {
        return true;
    }
    let ext = parsed
        .path()
        .rsplit('/')
        .next()
        .and_then(|s| s.rsplit_once('.').map(|(_, e)| e))
        .map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("mp4" | "m3u8" | "m3u" | "webm"))
}

/// Extract `content` attribute from a meta[itemprop] element.
pub(super) fn meta_content(html: &Html, selector: &Selector) -> Option<String> {
    html.select(selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
