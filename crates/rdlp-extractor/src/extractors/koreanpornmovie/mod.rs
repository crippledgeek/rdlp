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

static META_NAME_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="name"]"#).expect("valid selector"));

static META_DESCRIPTION_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="description"]"#).expect("valid selector"));

static META_DURATION_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="duration"]"#).expect("valid selector"));

static META_THUMBNAIL_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="thumbnailUrl"]"#).expect("valid selector"));

static META_CONTENT_URL_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="contentURL"]"#).expect("valid selector"));

static META_EMBED_URL_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="embedURL"]"#).expect("valid selector"));

static META_UPLOAD_DATE_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"meta[itemprop="uploadDate"]"#).expect("valid selector"));

static PLAYER_IFRAME_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"iframe[src*="player-x.php"]"#).expect("valid selector"));

static ACTOR_LINK_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"a[href*="/actor/"]"#).expect("valid selector"));

static TAG_LINK_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(r#"a[href*="/tag/"]"#).expect("valid selector"));

// Search result selectors
static SEARCH_ARTICLE_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("article.loop-video").expect("valid selector"));

static SEARCH_LINK_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a[href]").expect("valid selector"));

#[allow(dead_code)]
static SEARCH_IMG_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("img").expect("valid selector"));

static SEARCH_DURATION_SELECTOR: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".duration").expect("valid selector"));

#[allow(dead_code)]
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
                .as_deref()
                .and_then(BaseExtractor::parse_iso8601_duration);

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
            info.actors = actors;
            info.tags = if tags.is_empty() { None } else { Some(tags) };

            (info, formats)
        }; // html dropped

        if formats.is_empty() {
            return Err(RdlpError::extraction(
                "No video formats found. The video may require login or is an external embed only.",
                url,
            ));
        }

        // === Phase 3: Probe filesize via HEAD request (async) ===
        let mut formats = formats;
        for format in &mut formats {
            if let Some(size) = probe_filesize(&format.url, ctx).await {
                format.filesize = Some(size);
            }
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

        debug!(
            "[KoreanPornMovie] {} '{}' (page {})",
            browse_mode, query.query, page
        );

        match browse_mode {
            "actor" => self.search_api_taxonomy(query, ctx, "actors", page).await,
            "tag" => self.search_api_taxonomy(query, ctx, "tags", page).await,
            _ => self.search_api(query, ctx, page).await,
        }
    }
}

// ============================================================================
// Search Implementations
// ============================================================================

/// WP REST API response for a post.
#[derive(serde::Deserialize)]
struct WpPost {
    #[allow(dead_code)]
    id: u64,
    date: String,
    link: String,
    title: WpTitle,
    #[serde(rename = "_embedded")]
    embedded: Option<WpEmbedded>,
}

/// WP REST API taxonomy term (actor or tag).
#[derive(serde::Deserialize)]
struct WpTerm {
    id: u64,
    #[allow(dead_code)]
    name: String,
    count: u64,
}

#[derive(serde::Deserialize)]
struct WpTitle {
    rendered: String,
}

#[derive(serde::Deserialize)]
struct WpEmbedded {
    #[serde(rename = "wp:featuredmedia", default)]
    featured_media: Vec<WpMedia>,
    #[serde(rename = "wp:term", default)]
    terms: Vec<Vec<WpEmbeddedTerm>>,
}

#[derive(serde::Deserialize)]
struct WpEmbeddedTerm {
    taxonomy: String,
    name: String,
}

#[derive(serde::Deserialize)]
struct WpMedia {
    source_url: Option<String>,
}

impl KoreanPornMovieExtractor {
    /// Keyword search: REST API + HTML listing in parallel.
    ///
    /// REST API provides: title, date, thumbnail, actors (embedded terms).
    /// HTML listing provides: duration (only source for this field).
    async fn search_api(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
        page: u32,
    ) -> Result<SearchPageResponse> {
        let per_page = 20;
        let encoded_query = urlencoding::encode(&query.query);
        let api_url = format!(
            "https://koreanpornmovie.com/wp-json/wp/v2/posts?search={encoded_query}&page={page}&per_page={per_page}&_embed",
        );
        let html_url = format!("https://koreanpornmovie.com/?s={encoded_query}&paged={page}",);

        // Fire both requests concurrently
        let (api_result, html_result) = tokio::join!(
            ctx.http_client.get(&api_url).send(),
            ctx.http_client.get(&html_url).send(),
        );

        // Parse REST API (primary source)
        let response = api_result
            .map_err(|e| RdlpError::network(format!("REST API failed: {e}"), &api_url))?;
        let total_pages = extract_total_pages(&response);
        let posts: Vec<WpPost> = response.json().await.map_err(|e| {
            RdlpError::extraction(format!("Failed to parse API response: {e}"), &api_url)
        })?;

        // Parse HTML for durations (best-effort, non-fatal)
        let durations = scrape_durations_from_html_response(html_result).await;

        let results = posts
            .into_iter()
            .map(|post| {
                let duration = durations.get(post.link.as_str()).copied();
                wp_post_to_preview(post, duration)
            })
            .collect();

        Ok(SearchPageResponse {
            results,
            page,
            has_more: page < total_pages,
            total_estimate: None,
        })
    }

    /// Actor/tag browse: resolve slug → term ID, then REST API + HTML in parallel.
    async fn search_api_taxonomy(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
        taxonomy: &str,
        page: u32,
    ) -> Result<SearchPageResponse> {
        let slug = query.query.to_lowercase().replace(' ', "-");

        // Step 1: Resolve slug to term ID
        let term_url = format!(
            "https://koreanpornmovie.com/wp-json/wp/v2/{taxonomy}?slug={slug}&_fields=id,name,count"
        );
        let terms: Vec<WpTerm> = ctx
            .http_client
            .get(&term_url)
            .send()
            .await
            .map_err(|e| RdlpError::network(format!("term lookup failed: {e}"), &term_url))?
            .json()
            .await
            .map_err(|e| RdlpError::extraction(format!("failed to parse term: {e}"), &term_url))?;

        let term = terms.first().ok_or_else(|| {
            RdlpError::extraction(format!("No {taxonomy} found with slug '{slug}'"), &term_url)
        })?;

        debug!(
            "[KoreanPornMovie] Resolved {taxonomy} '{slug}' → id={}, count={}",
            term.id, term.count
        );

        // Step 2: REST API + HTML listing in parallel
        let per_page = 20;
        let api_url = format!(
            "https://koreanpornmovie.com/wp-json/wp/v2/posts?{taxonomy}={}&page={page}&per_page={per_page}&_embed",
            term.id,
        );
        let browse_prefix = if taxonomy == "actors" { "actor" } else { "tag" };
        let html_url = if page > 1 {
            format!("https://koreanpornmovie.com/{browse_prefix}/{slug}/page/{page}/")
        } else {
            format!("https://koreanpornmovie.com/{browse_prefix}/{slug}/")
        };

        let (api_result, html_result) = tokio::join!(
            ctx.http_client.get(&api_url).send(),
            ctx.http_client.get(&html_url).send(),
        );

        let response = api_result
            .map_err(|e| RdlpError::network(format!("post query failed: {e}"), &api_url))?;
        let total_pages = extract_total_pages(&response);
        let posts: Vec<WpPost> = response
            .json()
            .await
            .map_err(|e| RdlpError::extraction(format!("failed to parse posts: {e}"), &api_url))?;

        let durations = scrape_durations_from_html_response(html_result).await;

        let results = posts
            .into_iter()
            .map(|post| {
                let duration = durations.get(post.link.as_str()).copied();
                wp_post_to_preview(post, duration)
            })
            .collect();

        Ok(SearchPageResponse {
            results,
            page,
            has_more: page < total_pages,
            total_estimate: Some(term.count),
        })
    }
}

/// Convert a WP REST API post to a SearchResultPreview with all available data.
fn wp_post_to_preview(post: WpPost, duration: Option<f64>) -> SearchResultPreview {
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
        actors,
        view_count: None,
        upload_date,
    }
}

/// Extract X-WP-TotalPages from response headers.
fn extract_total_pages(response: &wreq::Response) -> u32 {
    response
        .headers()
        .get("x-wp-totalpages")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Scrape durations from an HTML listing response (best-effort, non-fatal).
async fn scrape_durations_from_html_response(
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
fn scrape_durations_from_html(html_text: &str) -> std::collections::HashMap<String, f64> {
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
fn html_entities_decode(s: &str) -> String {
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
    let decoded_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, q.as_bytes()).ok()?;
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

    static SRC_PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"src=["']([^"']+)["']"#).expect("valid src pattern"));

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

/// Probe a video URL via HEAD request to get Content-Length (filesize).
///
/// Returns `None` on any error (timeout, 403, etc.) — non-fatal.
async fn probe_filesize(url: &str, ctx: &ExtractionContext) -> Option<u64> {
    let response = ctx
        .http_client
        .head(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Extract `content` attribute from a meta[itemprop] element.
fn meta_content(html: &Html, selector: &Selector) -> Option<String> {
    html.select(selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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
    fn extract_urls_multi_source() {
        // Multi-quality: multiple <source> tags in one <video>
        let tag = r#"<video class="video-js" controls>
            <source src="https://koreanporn.stream/Movie-480p.mp4" type="video/mp4" />
            <source src="https://koreanporn.stream/Movie-720p.mp4" type="video/mp4" />
            <source src="https://koreanporn.stream/Movie-1080p.mp4" type="video/mp4" />
        </video>"#;
        let urls = extract_urls_from_decoded_tag(tag);
        assert_eq!(urls.len(), 3);
        assert!(urls[0].contains("480p"));
        assert!(urls[1].contains("720p"));
        assert!(urls[2].contains("1080p"));
    }

    #[test]
    fn extract_urls_skips_embed_iframe() {
        let tag = r#"<iframe src="https://www.pornhub.com/embed/ph123"></iframe>"#;
        let urls = extract_urls_from_decoded_tag(tag);
        assert!(urls.is_empty()); // PornHub embed is not a direct media URL
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
