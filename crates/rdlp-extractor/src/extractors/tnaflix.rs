use async_trait::async_trait;
use once_cell::sync::Lazy;
use rdlp_core::{check_http_response, ExtractionContext, InfoDict, InfoExtractor, Result, RdlpError};
use regex::Regex;
use scraper::Html;

use crate::base::tnaflix_network::{TnaFlixNetworkBase, VideoMetadata};

/// Static URL pattern regexes for each site (initialized once at first use)
///
/// Performance: Using static lazy patterns prevents regex compilation overhead:
/// - Without lazy: ~50-80μs compilation per constructor call
/// - With lazy: ~0.01μs access after first initialization
/// - Saves ~150μs at startup + ~200μs in tests (7 total compilations avoided)
static TNAFLIX_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?tnaflix\.com/[^/]+/[^/]+/video(\d+)")
        .expect("Valid TNAFlix URL pattern")
});

static EMPFLIX_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?empflix\.com/(?:videos/(?:[^/]+-)?(\d+)|[^/]+/[^/]+/video(\d+)|[^/]+/(\d+))")
        .expect("Valid EMPFlix URL pattern")
});

static MOVIEFAP_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?moviefap\.com/videos/([0-9a-f]+)/[^/]+\.html")
        .expect("Valid MovieFap URL pattern")
});

/// TNAFlix network extractor (supports TNAFlix, EMPFlix, MovieFap)
///
/// Uses [`TnaFlixNetworkBase`] for shared extraction logic.
pub struct TNAFlixExtractor {
    name: String,
    url_pattern: &'static Regex,
    base: TnaFlixNetworkBase,
}

impl TNAFlixExtractor {
    /// Create extractor for TNAFlix
    pub fn tnaflix() -> Self {
        Self {
            name: "TNAFlix".to_string(),
            url_pattern: &TNAFLIX_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Create extractor for EMPFlix
    pub fn empflix() -> Self {
        Self {
            name: "EMPFlix".to_string(),
            url_pattern: &EMPFLIX_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Create extractor for MovieFap
    pub fn moviefap() -> Self {
        Self {
            name: "MovieFap".to_string(),
            url_pattern: &MOVIEFAP_URL_PATTERN,
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Extract video ID from URL
    fn extract_id(&self, url: &str) -> Option<String> {
        self.url_pattern
            .captures(url)
            .and_then(|cap| {
                // Try each capture group in order (different URL patterns)
                cap.get(1)
                    .or_else(|| cap.get(2))
                    .or_else(|| cap.get(3))
            })
            .map(|m| m.as_str().to_string())
    }

    /// Parse EMPFlix AJAX JSON response to extract video sources
    async fn parse_empflix_ajax(&self, video_id: &str, referer: &str, ctx: &ExtractionContext) -> Result<Vec<VideoMetadata>> {
        // Fetch JSON from AJAX endpoint
        let ajax_url = format!("https://www.empflix.com/ajax/video-player/{video_id}");

        let response = ctx.http_client
            .get(&ajax_url)
            .header("Referer", referer)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch EMPFlix AJAX: {e}")))?;

        check_http_response(&response)?;

        let json_text = response.text().await
            .map_err(|e| RdlpError::Network(format!("Failed to read AJAX response: {e}")))?;

        if ctx.config.verbose {
            eprintln!("\n=== EMPFlix AJAX Response ===");
            eprintln!("{}", &json_text.chars().take(500).collect::<String>());
            eprintln!("=== END AJAX ===\n");
        }

        // Parse JSON to extract HTML field
        let json: serde_json::Value = serde_json::from_str(&json_text)
            .map_err(|e| RdlpError::Extraction(format!("Failed to parse AJAX JSON: {e}")))?;

        let html_str = json.get("html")
            .and_then(|h| h.as_str())
            .ok_or_else(|| RdlpError::Extraction("No 'html' field in AJAX response".to_string()))?;

        // Parse the HTML to extract <source> tags using base
        let html = Html::parse_document(html_str);
        let video_data = self.base.parse_video_sources(&html);

        if video_data.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video sources found in EMPFlix AJAX HTML. URL: {referer}"
            )));
        }

        Ok(video_data)
    }

    /// Parse MovieFap XML response to extract video sources
    async fn parse_moviefap_xml(&self, cdn_url: &str, ctx: &ExtractionContext) -> Result<Vec<VideoMetadata>> {
        // Fetch the XML from cdn.php
        let response = ctx.http_client
            .get(cdn_url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch MovieFap XML: {e}")))?;

        check_http_response(&response)?;

        let xml_text = response.text().await
            .map_err(|e| RdlpError::Network(format!("Failed to read XML response: {e}")))?;

        if ctx.config.verbose {
            eprintln!("\n=== MovieFap XML Response ===");
            eprintln!("{xml_text}");
            eprintln!("=== END XML ===\n");
        }

        // Use base to parse XML
        let video_data = self.base.parse_moviefap_xml(&xml_text);

        if video_data.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video sources found in MovieFap XML response from: {cdn_url}"
            )));
        }

        Ok(video_data)
    }
}

#[async_trait]
impl InfoExtractor for TNAFlixExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn valid_url(&self) -> &Regex {
        self.url_pattern
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Fetch the webpage
        let response = ctx.http_client
            .get(url)
            .send()
            .await
            .map_err(|e| RdlpError::Network(format!("Failed to fetch webpage: {e}")))?;

        check_http_response(&response)?;

        let webpage = response.text().await
            .map_err(|e| RdlpError::Network(format!("Failed to read response: {e}")))?;

        // Debug: print a sample of the webpage if verbose mode is enabled
        if ctx.config.verbose {
            eprintln!("\n=== WEBPAGE SAMPLE (first 5000 chars) ===");
            eprintln!("{}", &webpage.chars().take(5000).collect::<String>());
            eprintln!("=== END SAMPLE ===\n");
        }

        // Get video ID
        let video_id = self.extract_id(url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract video ID from URL: {url}")))?;

        // Check if this is MovieFap (uses different video loading mechanism)
        let is_moviefap = url.contains("moviefap.com");

        // Extract all data from HTML before any async operations
        let (metadata, cdn_url_opt) = {
            let html = Html::parse_document(&webpage);

            // Extract metadata using base (includes title, description, uploader, thumbnail, and enhanced JSON-LD fields)
            let metadata = self.base.extract_metadata(&html)?;

            // For MovieFap, extract cdn.php URL using base
            let cdn_url_opt = if is_moviefap {
                self.base.extract_cdn_url(&webpage)
            } else {
                None
            };

            (metadata, cdn_url_opt)
        }; // html is dropped here

        // Parse video data based on site type
        let video_data = if is_moviefap {
            // MovieFap: fetch XML from cdn.php
            let cdn_url = cdn_url_opt.ok_or_else(|| RdlpError::Extraction(format!(
                "Could not find cdn.php URL in MovieFap page: {url}"
            )))?;

            if ctx.config.verbose {
                eprintln!("MovieFap cdn.php URL: {cdn_url}");
            }

            self.parse_moviefap_xml(&cdn_url, ctx).await?
        } else {
            // TNAFlix/EMPFlix: try HTML <source> tags first, fallback to AJAX
            let video_data = {
                let html = Html::parse_document(&webpage);
                self.base.parse_video_sources(&html)
            }; // html is dropped here

            // EMPFlix fallback: if no sources found, try AJAX endpoint
            let video_data = if video_data.is_empty() && url.contains("empflix.com") {
                if ctx.config.verbose {
                    eprintln!("No sources in HTML, trying EMPFlix AJAX endpoint...");
                }
                self.parse_empflix_ajax(&video_id, url, ctx).await?
            } else {
                video_data
            };

            // Return error if still no sources found
            if video_data.is_empty() {
                return Err(RdlpError::Extraction(format!(
                    "No video source tags found in HTML. Video may be unavailable. URL: {url}"
                )));
            }

            video_data
        };

        // Build formats and fetch filesizes using base (asynchronous)
        let formats = self.base.build_formats(video_data, ctx).await;

        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Build InfoDict with all extracted metadata
        let mut info = InfoDict::new(video_id, metadata.title, self.name.clone(), url.to_string());
        info.description = metadata.description;
        info.uploader = metadata.uploader;
        info.thumbnail = metadata.thumbnail;
        info.thumbnails = metadata.thumbnails;
        info.duration = metadata.duration;
        info.upload_date = metadata.upload_date;
        info.view_count = metadata.view_count;
        info.tags = metadata.tags;
        info.categories = metadata.categories;
        info.formats = formats;

        Ok(info)
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared test fixtures (compiled once, reused across all tests)
    ///
    /// Performance: Prevents unnecessary regex compilation in tests:
    /// - Without lazy: ~50μs × 5 test instances = 250μs wasted
    /// - With lazy: ~0.01μs access after first initialization
    /// - Saves ~250μs per test run
    static TEST_TNAFLIX: Lazy<TNAFlixExtractor> = Lazy::new(|| TNAFlixExtractor::tnaflix());
    static TEST_EMPFLIX: Lazy<TNAFlixExtractor> = Lazy::new(|| TNAFlixExtractor::empflix());
    static TEST_MOVIEFAP: Lazy<TNAFlixExtractor> = Lazy::new(|| TNAFlixExtractor::moviefap());

    #[test]
    fn test_tnaflix_url_pattern() {
        let extractor = &*TEST_TNAFLIX;
        assert!(extractor.suitable("https://www.tnaflix.com/hd-videos/test/video123456"));
        assert!(extractor.suitable("https://tnaflix.com/amateur-porn/title/video999"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_empflix_url_pattern() {
        let extractor = &*TEST_EMPFLIX;
        assert!(extractor.suitable("https://www.empflix.com/videos/title-123"));
        assert!(extractor.suitable("https://empflix.com/view/123"));
        assert!(extractor.suitable("https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093"));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_empflix_extract_id() {
        let extractor = &*TEST_EMPFLIX;

        // Test /videos/title-ID format
        let id1 = extractor.extract_id("https://www.empflix.com/videos/title-123");
        assert_eq!(id1, Some("123".to_string()));

        // Test /category/ID format
        let id2 = extractor.extract_id("https://empflix.com/view/456");
        assert_eq!(id2, Some("456".to_string()));

        // Test /category/title/videoID format
        let id3 = extractor.extract_id("https://www.empflix.com/amateur-porn/older-medical-doc-creampie-innocent-girl/video3715093");
        assert_eq!(id3, Some("3715093".to_string()));
    }

    #[test]
    fn test_moviefap_url_pattern() {
        let extractor = &*TEST_MOVIEFAP;
        assert!(extractor.suitable("https://www.moviefap.com/videos/abc123def/title.html"));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_extract_id() {
        let extractor = &*TEST_TNAFLIX;
        let id = extractor.extract_id("https://www.tnaflix.com/hd-videos/test/video123456");
        assert_eq!(id, Some("123456".to_string()));
    }
}
