//! KoreanPornMovie extractor and search.
//!
//! WordPress site (RetroTube theme) hosting Korean adult films. Videos are
//! served via `clean-tube-player` plugin which wraps content in a
//! `player-x.php?q=<base64>` iframe. Decoded content is either:
//! - `type=video` — direct MP4 on `koreanporn.stream` CDN (self-hosted)
//! - `type=iframe` — PornHub or other external embed
//!
//! Metadata comes from Schema.org `itemprop` meta tags in the article.

mod patterns;

use async_trait::async_trait;
use log::debug;
use regex::Regex;
use scraper::{Html, Selector};
use std::sync::LazyLock;

use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, SearchExtractor};
use rdlp_types::{
    DownloadProtocol, Format, InfoDict, SearchFilterDescriptor, SearchPageResponse, SearchQuery,
    SearchResultPreview,
};

use crate::base::common::BaseExtractor;

// ============================================================================
// Selectors
// ============================================================================

static META_NAME_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="name"]"#).expect("valid selector")
});

static META_DESCRIPTION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="description"]"#).expect("valid selector")
});

static META_DURATION_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="duration"]"#).expect("valid selector")
});

static META_THUMBNAIL_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="thumbnailUrl"]"#).expect("valid selector")
});

static META_CONTENT_URL_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="contentURL"]"#).expect("valid selector")
});

static META_EMBED_URL_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="embedURL"]"#).expect("valid selector")
});

static META_UPLOAD_DATE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"meta[itemprop="uploadDate"]"#).expect("valid selector")
});

static PLAYER_IFRAME_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"iframe[src*="player-x.php"]"#).expect("valid selector")
});

static ACTOR_LINK_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"a[href*="/actor/"]"#).expect("valid selector")
});

static TAG_LINK_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"a[href*="/tag/"]"#).expect("valid selector")
});

// Search result selectors
static SEARCH_ARTICLE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse("article.loop-video").expect("valid selector")
});

static SEARCH_LINK_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href]").expect("valid selector"));

static SEARCH_IMG_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("img").expect("valid selector"));

static SEARCH_DURATION_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".duration").expect("valid selector"));

static SEARCH_NEXT_PAGE_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a.next").expect("valid selector"));

// ============================================================================
// Extractor
// ============================================================================

/// KoreanPornMovie extractor.
pub struct KoreanPornMovieExtractor;

impl KoreanPornMovieExtractor {
    /// Create a new extractor instance.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for KoreanPornMovieExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for KoreanPornMovieExtractor {
    fn name(&self) -> &str {
        "KoreanPornMovie"
    }

    fn valid_url(&self) -> &Regex {
        &patterns::VIDEO_URL_PATTERN
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_video_url(url)
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let slug = patterns::extract_slug(url).ok_or_else(|| {
            RdlpError::extraction(format!("Could not extract slug from URL: {url}"), url)
        })?;

        debug!("[KoreanPornMovie] Extracting: {slug}");

        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        // Parse metadata and formats synchronously (Html is !Send)
        let (mut info, formats) = {
            let html = Html::parse_document(&webpage);

            let title = meta_content(&html, &META_NAME_SELECTOR)
                .or_else(|| BaseExtractor::extract_title_multi_strategy(&html))
                .unwrap_or_else(|| slug.replace('-', " "));

            let description = meta_content(&html, &META_DESCRIPTION_SELECTOR)
                .or_else(|| BaseExtractor::extract_description_multi_strategy(&html));

            let thumbnail = meta_content(&html, &META_THUMBNAIL_SELECTOR)
                .or_else(|| BaseExtractor::extract_thumbnail_multi_strategy(&html));

            let upload_date = meta_content(&html, &META_UPLOAD_DATE_SELECTOR);
            let duration = meta_content(&html, &META_DURATION_SELECTOR)
                .and_then(|d| parse_iso8601_duration(&d));

            let actors: Vec<String> = html
                .select(&ACTOR_LINK_SELECTOR)
                .map(|a| a.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let tags: Vec<String> = html
                .select(&TAG_LINK_SELECTOR)
                .map(|a| a.text().collect::<String>().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            // Extract video formats from clean-tube-player iframe
            let formats = extract_formats_from_html(&html, url);

            let mut info = InfoDict::new(&slug, &title, "KoreanPornMovie", url);
            info.description = description;
            info.thumbnail = thumbnail;
            info.upload_date = upload_date;
            info.duration = duration;
            if !actors.is_empty() {
                info.uploader = Some(actors.join(", "));
            }
            info.tags = if tags.is_empty() { None } else { Some(tags) };

            (info, formats)
        }; // html dropped

        if formats.is_empty() {
            return Err(RdlpError::extraction(
                "No video formats found. The video may require login or is an external embed only.",
                url,
            ));
        }

        info.formats = formats;
        Ok(info)
    }
}

// ============================================================================
// Search
// ============================================================================

#[async_trait]
impl SearchExtractor for KoreanPornMovieExtractor {
    fn name(&self) -> &str {
        "KoreanPornMovie"
    }

    fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
        vec![
            SearchFilterDescriptor {
                key: "browse".to_string(),
                display_name: "Browse mode".to_string(),
                allowed_values: vec![
                    rdlp_types::SearchFilterValue {
                        value: "search".to_string(),
                        label: "Keyword search".to_string(),
                    },
                    rdlp_types::SearchFilterValue {
                        value: "actor".to_string(),
                        label: "Browse by actor".to_string(),
                    },
                    rdlp_types::SearchFilterValue {
                        value: "tag".to_string(),
                        label: "Browse by tag".to_string(),
                    },
                ],
                default: Some("search".to_string()),
            },
            SearchFilterDescriptor {
                key: "ordering".to_string(),
                display_name: "Sort order".to_string(),
                allowed_values: vec![
                    rdlp_types::SearchFilterValue {
                        value: "latest".to_string(),
                        label: "Latest".to_string(),
                    },
                    rdlp_types::SearchFilterValue {
                        value: "longest".to_string(),
                        label: "Longest".to_string(),
                    },
                    rdlp_types::SearchFilterValue {
                        value: "random".to_string(),
                        label: "Random".to_string(),
                    },
                ],
                default: Some("latest".to_string()),
            },
        ]
    }

    async fn search(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<Vec<SearchResultPreview>> {
        let response = self.search_page(query, ctx).await?;
        Ok(response.results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        let page = query.page.unwrap_or(1);
        let browse_mode = query
            .filters
            .iter()
            .find(|f| f.key == "browse")
            .map(|f| f.value.as_str())
            .unwrap_or("search");

        let slug = query.query.to_lowercase().replace(' ', "-");
        let search_url = match browse_mode {
            "actor" => {
                // Browse videos by actor: /actor/<slug>/page/N/
                if page > 1 {
                    format!("https://koreanpornmovie.com/actor/{slug}/page/{page}/")
                } else {
                    format!("https://koreanpornmovie.com/actor/{slug}/")
                }
            }
            "tag" => {
                // Browse videos by tag: /tag/<slug>/page/N/
                if page > 1 {
                    format!("https://koreanpornmovie.com/tag/{slug}/page/{page}/")
                } else {
                    format!("https://koreanpornmovie.com/tag/{slug}/")
                }
            }
            _ => {
                // Default: keyword search
                format!(
                    "https://koreanpornmovie.com/?s={}&paged={}",
                    urlencoding::encode(&query.query),
                    page
                )
            }
        };

        debug!("[KoreanPornMovie] {} '{}' (page {})", browse_mode, query.query, page);

        let webpage = BaseExtractor::fetch_webpage(&search_url, ctx).await?;

        let (results, has_more) = {
            let html = Html::parse_document(&webpage);

            let results: Vec<SearchResultPreview> = html
                .select(&SEARCH_ARTICLE_SELECTOR)
                .filter_map(|article| {
                    let link = article.select(&SEARCH_LINK_SELECTOR).next()?;
                    let href = link.value().attr("href")?;

                    // Skip non-video links
                    if !patterns::VIDEO_URL_PATTERN.is_match(href) {
                        return None;
                    }

                    let img = article.select(&SEARCH_IMG_SELECTOR).next();
                    let thumbnail = img.and_then(|i| {
                        i.value()
                            .attr("src")
                            .or_else(|| i.value().attr("data-src"))
                            .map(|s| s.to_string())
                    });

                    let title = img
                        .and_then(|i| i.value().attr("alt"))
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    let duration = article
                        .select(&SEARCH_DURATION_SELECTOR)
                        .next()
                        .map(|d| d.text().collect::<String>().trim().to_string())
                        .and_then(|d| parse_hms_duration(&d));

                    Some(SearchResultPreview {
                        title,
                        video_url: href.to_string(),
                        thumbnail_url: thumbnail,
                        duration,
                        view_count: None,
                        upload_date: None,
                    })
                })
                .collect();

            let has_more = html.select(&SEARCH_NEXT_PAGE_SELECTOR).next().is_some();

            (results, has_more)
        }; // html dropped

        Ok(SearchPageResponse {
            results,
            page,
            has_more,
            total_estimate: None,
        })
    }
}

// ============================================================================
// Format Extraction Helpers
// ============================================================================

/// Extract video formats from the page HTML.
///
/// Looks for the `clean-tube-player` iframe, decodes its base64 `q` parameter,
/// and extracts the video source URL.
fn extract_formats_from_html(html: &Html, _page_url: &str) -> Vec<Format> {
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

    // Strategy 2: Decode clean-tube-player iframe base64
    if let Some(ref decoded) = decoded_tag {
        for url in extract_urls_from_decoded_tag(decoded) {
            if !formats.iter().any(|f| f.url == url) {
                let mut format = make_video_format("kpm-player", &url);
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
fn make_video_format(format_id: &str, url: &str) -> Format {
    let protocol = if url.contains(".m3u8") {
        DownloadProtocol::M3u8
    } else {
        DownloadProtocol::Https
    };
    let ext = url
        .split('.')
        .next_back()
        .unwrap_or("mp4")
        .split('?')
        .next()
        .unwrap_or("mp4");

    let mut format = Format::new(format_id, url, ext, protocol);
    // Mark as video (not audio-only) — actual codec determined at download time
    format.vcodec = Some("video".to_string());
    format.acodec = Some("audio".to_string());
    format
}

/// Extract width/height from a decoded `<video width="640" height="264">` tag.
fn extract_dimensions_from_tag(tag: &str) -> (Option<u32>, Option<u32>) {
    static WIDTH_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"width=["'](\d+)["']"#).expect("valid"));
    static HEIGHT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"height=["'](\d+)["']"#).expect("valid"));

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
fn decode_player_iframe(iframe_src: &str) -> Option<String> {
    let url = url::Url::parse(iframe_src).ok()?;
    let q = url.query_pairs().find(|(k, _)| k == "q")?.1;

    // Base64 decode
    let decoded_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        q.as_bytes(),
    )
    .ok()?;
    let decoded = String::from_utf8(decoded_bytes).ok()?;

    // URL-decode the tag parameter
    let params: Vec<(String, String)> = url::form_urlencoded::parse(decoded.as_bytes())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    params
        .iter()
        .find(|(k, _)| k == "tag")
        .map(|(_, v)| v.clone())
}

/// Extract media URLs from the decoded player tag HTML.
///
/// Handles both `<video><source src="...">` and `<iframe src="...">` patterns.
fn extract_urls_from_decoded_tag(tag_html: &str) -> Vec<String> {
    use regex::Regex;

    static SRC_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"src=["']([^"']+)["']"#).expect("valid src pattern")
    });

    SRC_PATTERN
        .captures_iter(tag_html)
        .filter_map(|caps| {
            let url = caps.get(1)?.as_str();
            // Only return actual media URLs, not embed pages
            if url.contains(".mp4")
                || url.contains(".m3u8")
                || url.contains(".webm")
                || url.contains("koreanporn.stream")
            {
                Some(url.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Extract `content` attribute from a meta[itemprop] element.
fn meta_content(html: &Html, selector: &Selector) -> Option<String> {
    html.select(selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Parse ISO 8601 duration (e.g., "P0DT1H2M3S") to seconds.
fn parse_iso8601_duration(duration: &str) -> Option<f64> {
    // Extended format: P0DT1H2M3S
    static DURATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"P(?:\d+D)?T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?")
            .expect("valid duration pattern")
    });

    let caps = DURATION_PATTERN.captures(duration)?;
    let hours: f64 = caps.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
    let minutes: f64 = caps.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);
    let seconds: f64 = caps.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0.0);

    let total = hours * 3600.0 + minutes * 60.0 + seconds;
    if total > 0.0 { Some(total) } else { None }
}

/// Parse "HH:MM:SS" or "MM:SS" duration string to seconds.
fn parse_hms_duration(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let s: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + s)
        }
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let s: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + s)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_player_iframe_video_type() {
        // Base64 of: post_id=9815&type=video&tag=<video ...><source src="https://koreanporn.stream/test.mp4" type="video/mp4" /></video>
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "post_id=9815&type=video&tag=%3Cvideo%3E%3Csource%20src%3D%22https%3A%2F%2Fkoreanporn.stream%2Ftest.mp4%22%20type%3D%22video%2Fmp4%22%20%2F%3E%3C%2Fvideo%3E",
        );
        let iframe_src = format!(
            "https://koreanpornmovie.com/wp-content/plugins/clean-tube-player/public/player-x.php?q={encoded}"
        );
        let decoded = decode_player_iframe(&iframe_src).unwrap();
        assert!(decoded.contains("koreanporn.stream/test.mp4"));
    }

    #[test]
    fn decode_player_iframe_embed_type() {
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "post_id=987&type=iframe&tag=%3Ciframe%20src%3D%22https%3A%2F%2Fwww.pornhub.com%2Fembed%2Fph123%22%3E%3C%2Fiframe%3E",
        );
        let iframe_src = format!(
            "https://koreanpornmovie.com/wp-content/plugins/clean-tube-player/public/player-x.php?q={encoded}"
        );
        let decoded = decode_player_iframe(&iframe_src).unwrap();
        assert!(decoded.contains("pornhub.com/embed/ph123"));
    }

    #[test]
    fn extract_urls_from_video_tag() {
        let tag = r#"<video class="video-js"><source src="https://koreanporn.stream/Movie.mp4" type="video/mp4" /></video>"#;
        let urls = extract_urls_from_decoded_tag(tag);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://koreanporn.stream/Movie.mp4");
    }

    #[test]
    fn extract_urls_skips_embed_iframe() {
        let tag = r#"<iframe src="https://www.pornhub.com/embed/ph123"></iframe>"#;
        let urls = extract_urls_from_decoded_tag(tag);
        assert!(urls.is_empty()); // PornHub embed is not a direct media URL
    }

    #[test]
    fn parse_duration_full() {
        assert_eq!(parse_iso8601_duration("P0DT1H1M14S"), Some(3674.0));
        assert_eq!(parse_iso8601_duration("P0DT0H1M57S"), Some(117.0));
        assert_eq!(parse_iso8601_duration("PT30S"), Some(30.0));
    }

    #[test]
    fn extractor_name_and_priority() {
        let ext = KoreanPornMovieExtractor::new();
        assert_eq!(InfoExtractor::name(&ext), "KoreanPornMovie");
        assert_eq!(ext.priority(), 0);
    }

    #[test]
    fn extractor_suitable() {
        let ext = KoreanPornMovieExtractor::new();
        assert!(ext.suitable("https://koreanpornmovie.com/gangnam-full-salon-2024/"));
        assert!(ext.suitable("https://koreanpornmovie.com/taste-of-a-young-woman-2025/"));
        assert!(!ext.suitable("https://koreanpornmovie.com/tags/"));
        assert!(!ext.suitable("https://koreanpornmovie.com/privacy-policy/"));
        assert!(!ext.suitable("https://pornhub.com/view_video.php?viewkey=ph123"));
    }

    #[test]
    fn meta_content_extraction() {
        let html_str = r#"<html><head>
            <meta itemprop="name" content="Test Video">
            <meta itemprop="duration" content="P0DT1H30M0S">
            <meta itemprop="contentURL" content="https://koreanporn.stream/test.mp4">
        </head></html>"#;
        let html = Html::parse_document(html_str);

        assert_eq!(
            meta_content(&html, &META_NAME_SELECTOR),
            Some("Test Video".to_string())
        );
        assert_eq!(
            meta_content(&html, &META_CONTENT_URL_SELECTOR),
            Some("https://koreanporn.stream/test.mp4".to_string())
        );
    }

    #[test]
    fn format_extraction_with_content_url() {
        let html_str = r#"<html><head>
            <meta itemprop="contentURL" content="https://koreanporn.stream/Movie.mp4">
        </head><body></body></html>"#;
        let html = Html::parse_document(html_str);

        let formats = extract_formats_from_html(&html, "https://koreanpornmovie.com/test/");
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].url, "https://koreanporn.stream/Movie.mp4");
        assert_eq!(formats[0].ext, "mp4");
    }
}
