//! KoreanPornMovie extractor and search.
//!
//! WordPress site (RetroTube theme) hosting Korean adult films. Videos are
//! served via `clean-tube-player` plugin which wraps content in a
//! `player-x.php?q=<base64>` iframe. Decoded content is either:
//! - `type=video` — direct MP4 on `koreanporn.stream` CDN (self-hosted)
//! - `type=iframe` — PornHub or other external embed
//!
//! Metadata comes from Schema.org `itemprop` meta tags in the article.

mod html;
mod patterns;

use html::*;

use async_trait::async_trait;
use lazy_regex::{Lazy, Regex, lazy_regex};
use log::debug;
use scraper::{Html, Selector};
use std::sync::LazyLock;

use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result, SearchExtractor};
use rdlp_types::{
    Codec, DownloadProtocol, Format, InfoDict, SearchFilterDescriptor, SearchPageResponse,
    SearchQuery, SearchResultPreview,
};

use crate::base::common::BaseExtractor;
use crate::base::common::{PagedSearch, SearchPage};

// ============================================================================
// Selectors
// ============================================================================

static META_NAME_SELECTOR: LazyLock<Selector> = crate::static_selector!(r#"meta[itemprop="name"]"#);

static META_DESCRIPTION_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[itemprop="description"]"#);

static META_DURATION_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[itemprop="duration"]"#);

static META_THUMBNAIL_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[itemprop="thumbnailUrl"]"#);

static META_CONTENT_URL_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[itemprop="contentURL"]"#);

static META_EMBED_URL_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[itemprop="embedURL"]"#);

static META_UPLOAD_DATE_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"meta[itemprop="uploadDate"]"#);

static PLAYER_IFRAME_SELECTOR: LazyLock<Selector> =
    crate::static_selector!(r#"iframe[src*="player-x.php"]"#);

static ACTOR_LINK_SELECTOR: LazyLock<Selector> = crate::static_selector!(r#"a[href*="/actor/"]"#);

static TAG_LINK_SELECTOR: LazyLock<Selector> = crate::static_selector!(r#"a[href*="/tag/"]"#);

// Search result selectors
static SEARCH_ARTICLE_SELECTOR: LazyLock<Selector> = crate::static_selector!("article.loop-video");

static SEARCH_LINK_SELECTOR: LazyLock<Selector> = crate::static_selector!("a[href]");

#[allow(dead_code)]
static SEARCH_IMG_SELECTOR: LazyLock<Selector> = crate::static_selector!("img");

static SEARCH_DURATION_SELECTOR: LazyLock<Selector> = crate::static_selector!(".duration");

#[allow(dead_code)]
static SEARCH_NEXT_PAGE_SELECTOR: LazyLock<Selector> = crate::static_selector!("a.next");

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
            RdlpError::extraction(
                format!(
                    "Could not extract slug from URL: {}",
                    rdlp_redact::RedactedUrl::new(url)
                ),
                url,
            )
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

        // Pre-resolve HLS variant playlists into per-variant Format rows so
        // the downloader can take the Format.fragments fast path. Non-HLS
        // rows pass through unchanged; expand failures keep the original row
        // (graceful fallback to the legacy variant-URL path).
        let formats = crate::hls::expand_hls_in_place(formats, ctx.http_client.clone()).await;
        let (formats, _hls_flags) =
            crate::hls::detect_format_sizes_lazy(formats, ctx, InfoExtractor::name(self)).await;

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
        // KPM is single-page: opt OUT of the shared multi-page loop (which would
        // re-run the taxonomy term-lookup per page). Preserves the old single-call
        // search() exactly.
        Ok(self.search_page_response(query, ctx).await?.results)
    }

    async fn search_page(
        &self,
        query: &SearchQuery,
        ctx: &ExtractionContext,
    ) -> Result<SearchPageResponse> {
        self.search_page_response(query, ctx).await
    }
}

impl PagedSearch for KoreanPornMovieExtractor {
    fn search_log_tag(&self) -> &'static str {
        "[KoreanPornMovie]"
    }

    // KPM validates no filters today (the bespoke search_page never validated). Ok(()) preserves that.
    fn validate_search_filters(&self, _filters: &[rdlp_types::SearchFilter]) -> Result<()> {
        Ok(())
    }

    // Custom multi-endpoint hook (no SearchPageSpec): browse-mode dispatch → two concurrent
    // ctx.http_client fetches + X-WP-TotalPages pagination. Opts out of the loop via the
    // search() override above, so it is only ever called once per query.
    async fn fetch_page(
        &self,
        query: &SearchQuery,
        page: u32,
        ctx: &ExtractionContext,
    ) -> Result<SearchPage> {
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
    ) -> Result<SearchPage> {
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

        Ok(SearchPage {
            results,
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
    ) -> Result<SearchPage> {
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

        Ok(SearchPage {
            results,
            has_more: page < total_pages,
            total_estimate: Some(term.count),
        })
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
    fn extract_urls_from_decoded_tag_rejects_substring_in_query() {
        // Regression: substring matching admitted non-media URLs whose
        // query/fragment contained a media extension.
        let html = r#"<iframe src="https://host/page?embed=video.mp4"></iframe>"#;
        let urls = extract_urls_from_decoded_tag(html);
        assert!(
            urls.is_empty(),
            "expected empty (non-media URL), got {urls:?}"
        );
    }

    #[test]
    fn extract_urls_from_decoded_tag_accepts_media_extensions() {
        let html =
            r#"<source src="https://host/video.mp4"><source src="https://host/master.m3u8">"#;
        let urls = extract_urls_from_decoded_tag(html);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().any(|u| u.ends_with(".mp4")));
        assert!(urls.iter().any(|u| u.ends_with(".m3u8")));
    }

    #[test]
    fn extract_urls_from_decoded_tag_accepts_koreanporn_stream_host() {
        let html = r#"<source src="https://koreanporn.stream/embed/abc123">"#;
        let urls = extract_urls_from_decoded_tag(html);
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn extract_urls_from_decoded_tag_rejects_subdomain_squatting() {
        // Security: a hostile domain that contains "koreanporn.stream" as a
        // substring (e.g. `koreanporn.stream.evil.com`, or any URL whose
        // query references it) MUST NOT bypass the path-extension
        // allow-list via the host carve-out.
        let html = r#"<source src="https://koreanporn.stream.evil.com/page">"#;
        assert!(extract_urls_from_decoded_tag(html).is_empty());

        let html = r#"<source src="https://attacker.com/page?ref=koreanporn.stream">"#;
        assert!(extract_urls_from_decoded_tag(html).is_empty());
    }

    #[test]
    fn extract_urls_from_decoded_tag_accepts_koreanporn_stream_subdomain() {
        // Legitimate subdomains (e.g. cdn.koreanporn.stream) remain allowed.
        let html = r#"<source src="https://cdn.koreanporn.stream/embed/abc">"#;
        assert_eq!(extract_urls_from_decoded_tag(html).len(), 1);
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

    /// Regression guard for #258 — confirm an `M3u8` HLS row produced by
    /// `build_format` (protocol inferred from `.m3u8` in URL) is expanded
    /// into per-variant fragments by `expand_hls_in_place`, and that
    /// non-HLS rows pass through unchanged. Catches a wiring break where
    /// the helper call is removed from `extract` (the M3u8 row would
    /// arrive at the downloader without pre-resolved fragments).
    #[tokio::test]
    async fn hls_row_expanded_and_mp4_pass_through() {
        let mut server = mockito::Server::new_async().await;
        let _media = server
            .mock("GET", "/kpm-master.m3u8")
            .with_body(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n\
                 #EXTINF:6.0,\nseg-1.ts\n#EXTINF:6.0,\nseg-2.ts\n#EXT-X-ENDLIST\n",
            )
            .create_async()
            .await;

        let hls_url = format!("{}/kpm-master.m3u8", server.url());
        let hls = Format::new("hls", &hls_url, "m3u8", DownloadProtocol::M3u8);
        let mp4 = Format::new(
            "mp4",
            "https://koreanporn.stream/Movie.mp4",
            "mp4",
            DownloadProtocol::Https,
        );

        let formats = vec![hls, mp4];
        let http = std::sync::Arc::new(wreq::Client::new());
        let expanded = crate::hls::expand_hls_in_place(formats, http).await;

        assert_eq!(expanded.len(), 2);
        assert!(
            expanded[0].fragments.is_some(),
            "M3u8 row must carry pre-resolved fragments after expand"
        );
        assert_eq!(expanded[0].fragments.as_ref().unwrap().len(), 2);
        assert!(
            expanded[1].fragments.is_none(),
            "Https MP4 row must pass through untouched"
        );
        assert_eq!(expanded[1].url, "https://koreanporn.stream/Movie.mp4");
    }

    /// Regression guard for #258 + #279 — confirms koreanpornmovie's HLS row
    /// expansion happens BEFORE size probing.
    ///
    /// Mirrors the helper pair invoked by `extract` (`expand_hls_in_place` then
    /// `detect_format_sizes_lazy`). The master `expect(1)` assertion fails if
    /// the order is reverted: `detect_hls_variants` would re-fetch the master,
    /// pushing the GET count to >= 2.
    #[tokio::test]
    async fn test_koreanpornmovie_helper_pair_fetches_master_exactly_once() {
        use crate::hls::test_support::{MASTER_TWO_VARIANTS, VARIANT_MEDIA, test_ctx};
        use std::sync::Arc;

        let mut server = mockito::Server::new_async().await;
        let master = server
            .mock("GET", "/master.m3u8")
            .with_body(MASTER_TWO_VARIANTS)
            .expect(1)
            .create_async()
            .await;
        let _v720 = server
            .mock("GET", "/v720.m3u8")
            .with_body(VARIANT_MEDIA)
            .expect_at_least(1)
            .create_async()
            .await;
        let _v360 = server
            .mock("GET", "/v360.m3u8")
            .with_body(VARIANT_MEDIA)
            .expect_at_least(1)
            .create_async()
            .await;

        let master_url = format!("{}/master.m3u8", server.url());
        let hls = Format::new("hls", &master_url, "m3u8", DownloadProtocol::M3u8);

        let ctx = test_ctx();
        let http: Arc<wreq::Client> = ctx.http_client.clone();

        let formats = crate::hls::expand_hls_in_place(vec![hls], http).await;
        let (formats, _flags) =
            crate::hls::detect_format_sizes_lazy(formats, &ctx, "KoreanPornMovie").await;

        assert!(
            formats.iter().all(|fmt| fmt.fragments.is_some()),
            "expanded formats must carry fragments"
        );
        master.assert_async().await;
    }

    #[test]
    fn make_video_format_rejects_m3u8_substring_in_query() {
        // Regression: issue #268. A crafted MP4 URL with `.m3u8` in the
        // query parameter must NOT classify as HLS.
        let f = make_video_format("test", "https://host/clip.mp4?ref=foo.m3u8");
        assert!(
            matches!(f.protocol, rdlp_types::DownloadProtocol::Https),
            "expected Https, got {:?}",
            f.protocol
        );
    }

    #[test]
    fn make_video_format_classifies_m3u8_path_as_hls() {
        let f = make_video_format("test", "https://host/master.m3u8");
        assert!(
            matches!(f.protocol, rdlp_types::DownloadProtocol::M3u8),
            "expected M3u8, got {:?}",
            f.protocol
        );
    }
}
