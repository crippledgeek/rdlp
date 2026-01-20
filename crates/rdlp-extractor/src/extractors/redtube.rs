use async_trait::async_trait;
use once_cell::sync::Lazy;
use rdlp_core::{check_http_response, ExtractionContext, Format, InfoDict, InfoExtractor, Result, RdlpError};
use regex::Regex;
use scraper::Html;

use crate::base::tnaflix_network::TnaFlixNetworkBase;

/// Static URL pattern regex for RedTube (initialized once at first use)
///
/// Supports:
/// - Standard URLs: https://www.redtube.com/123456
/// - Brazilian domain: https://www.redtube.com.br/123456
/// - Embed URLs: https://embed.redtube.com/?id=123456
static REDTUBE_URL_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:(?:\w+\.)?redtube\.com(?:\.br)?/|embed\.redtube\.com/\?.*\bid=)(?P<id>\d+)")
        .expect("Valid RedTube URL pattern")
});

/// Regex to extract JavaScript sources object: sources: {"720": "url", ...}
static SOURCES_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"sources\s*:\s*(\{[^}]+\})"#)
        .expect("Valid sources pattern")
});

/// Regex to extract mediaDefinition array: mediaDefinition: [{...}, ...]
static MEDIA_DEF_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)mediaDefinition\s*:\s*(\[.+?\])")
        .expect("Valid mediaDefinition pattern")
});

/// RedTube extractor
///
/// Supports URLs like:
/// - https://www.redtube.com/123456
/// - https://www.redtube.com.br/123456
/// - https://embed.redtube.com/?id=123456
///
/// RedTube embeds video sources in JavaScript objects rather than HTML `<source>` tags,
/// so this extractor uses regex to extract JSON from the page source.
pub struct RedTubeExtractor {
    base: TnaFlixNetworkBase,
}

impl RedTubeExtractor {
    /// Create a new RedTube extractor
    pub fn new() -> Self {
        Self {
            base: TnaFlixNetworkBase::new(),
        }
    }

    /// Extract video ID from URL
    ///
    /// # Arguments
    /// * `url` - RedTube video URL
    ///
    /// # Returns
    /// Video ID if URL matches pattern, `None` otherwise
    fn extract_id(&self, url: &str) -> Option<String> {
        REDTUBE_URL_PATTERN
            .captures(url)
            .and_then(|cap| cap.name("id"))
            .map(|m| m.as_str().to_string())
    }

    /// Extract quality string from JSON value (handles both string and number types)
    fn parse_quality(item: &serde_json::Value) -> String {
        item.get("quality")
            .and_then(|q| {
                q.as_str().map(String::from)
                    .or_else(|| q.as_i64().map(|i| i.to_string()))
            })
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Build Format from quality string and URL
    fn build_format(&self, quality_str: &str, url: String, format_type: &str) -> Format {
        let mut format = Format::new(
            quality_str.to_string(),
            url,
            format_type.to_string(),
            "https".to_string(),
        );

        if let Ok(height) = quality_str.parse::<u32>() {
            format.height = Some(height);
            format.quality = Some((height / 100) as i32);
            format.format_note = Some(format!("{height}p"));
            format.width = Some((height * 16) / 9);
        } else {
            format.format_note = Some(quality_str.to_string());
        }

        format.vcodec = Some("h264".to_string());
        format.acodec = Some("aac".to_string());

        format
    }

    /// Extract video formats from JavaScript sources object
    ///
    /// Looks for: sources: {"720": "https://...", "1080": "https://...", ...}
    fn extract_from_sources(&self, webpage: &str, verbose: bool) -> Vec<Format> {
        let mut formats = Vec::new();

        if let Some(caps) = SOURCES_PATTERN.captures(webpage) {
            if let Some(sources_str) = caps.get(1) {
                if verbose {
                    eprintln!("\n[RedTube] Found sources object: {}", sources_str.as_str());
                }

                // Try to parse as JSON
                match serde_json::from_str::<serde_json::Value>(sources_str.as_str()) {
                    Ok(sources) => {
                        if let Some(obj) = sources.as_object() {
                            for (quality, url) in obj {
                                if let Some(url_str) = url.as_str() {
                                    // Extract format type from URL path
                                    let format_type = if let Ok(parsed_url) = url::Url::parse(url_str) {
                                        if let Some(mut path_segments) = parsed_url.path_segments() {
                                            if let Some(last_segment) = path_segments.next_back() {
                                                if let Some(ext_start) = last_segment.rfind('.') {
                                                    match &last_segment[ext_start + 1..] {
                                                        "mp4" => "mp4",
                                                        "m3u8" => "hls",
                                                        "webm" => "webm",
                                                        "mkv" => "mkv",
                                                        _ => "unknown",
                                                    }
                                                } else {
                                                    "unknown"
                                                }
                                            } else {
                                                "unknown"
                                            }
                                        } else {
                                            "unknown"
                                        }
                                    } else {
                                        "unknown"
                                    };
                                    let format = self.build_format(quality, url_str.to_string(), format_type);

                                    if verbose {
                                        eprintln!("[RedTube] Extracted format: {} ({})",
                                            format.format_id,
                                            format.format_note.as_deref().unwrap_or("unknown"));
                                    }

                                    formats.push(format);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("[RedTube] Failed to parse sources JSON at {}:{}: {}",
                                e.line(), e.column(), e);
                        }
                    }
                }
            }
        }

        formats
    }

    /// Extract video formats from mediaDefinition array
    ///
    /// Looks for: mediaDefinition: [{videoUrl: "...", format: "...", quality: "..."}, ...]
    ///
    /// Note: If format is "mp4" without quality field, videoUrl is a JSON endpoint
    /// that needs to be fetched to get the actual format list
    async fn extract_from_media_definition(&self, webpage: &str, ctx: &ExtractionContext) -> Vec<Format> {
        let mut formats = Vec::new();

        if let Some(caps) = MEDIA_DEF_PATTERN.captures(webpage) {
            if let Some(media_def_str) = caps.get(1) {
                if ctx.config.verbose {
                    eprintln!("\n[RedTube] Found mediaDefinition array: {}",
                        &media_def_str.as_str().chars().take(200).collect::<String>());
                }

                // Try to parse as JSON
                match serde_json::from_str::<serde_json::Value>(media_def_str.as_str()) {
                    Ok(media_def) => {
                        let Some(arr) = media_def.as_array() else {
                            if ctx.config.verbose {
                                eprintln!("[RedTube] mediaDefinition is not an array");
                            }
                            return formats;
                        };
                        if ctx.config.verbose {
                            eprintln!("[RedTube] Found {} media items", arr.len());
                        }

                        for (idx, item) in arr.iter().enumerate() {
                            if ctx.config.verbose {
                                eprintln!("[RedTube] Processing item {idx}: {item:?}");
                            }

                            if let Some(video_url) = item.get("videoUrl").and_then(|v| v.as_str()) {
                                let format_type = item.get("format")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("mp4");

                                let has_quality = item.get("quality").is_some();

                                // If format is mp4/hls without quality, fetch JSON to get actual formats
                                if (format_type == "mp4" || format_type == "hls") && !has_quality {
                                    // Convert relative URL to absolute
                                    let base_url = url::Url::parse("https://www.redtube.com")
                                        .expect("Valid base URL");

                                    if let Ok(absolute_url) = base_url.join(video_url) {
                                        if ctx.config.verbose {
                                            eprintln!("[RedTube] Fetching format JSON from: {absolute_url}");
                                        }

                                        // Fetch the JSON endpoint
                                        match ctx.http_client.get(absolute_url.as_str()).send().await {
                                            Ok(response) => {
                                                // Validate HTTP status code
                                                if !response.status().is_success() {
                                                    if ctx.config.verbose {
                                                        eprintln!("[RedTube] HTTP {} for URL: {}",
                                                            response.status(), absolute_url);
                                                    }
                                                    continue; // Skip this format source
                                                }

                                                match response.text().await {
                                                    Ok(json_text) => {
                                                        if ctx.config.verbose {
                                                            eprintln!("[RedTube] Got JSON response: {}",
                                                                &json_text.chars().take(500).collect::<String>());
                                                        }

                                                        // Parse JSON array of formats
                                                        match serde_json::from_str::<serde_json::Value>(&json_text) {
                                                            Ok(more_media) => {
                                                                let Some(more_arr) = more_media.as_array() else {
                                                                    if ctx.config.verbose {
                                                                        eprintln!("[RedTube] Response is not a JSON array");
                                                                    }
                                                                    continue;
                                                                };
                                                        for media_item in more_arr {
                                                            if let Some(media_url) = media_item.get("videoUrl").and_then(|v| v.as_str()) {
                                                                let quality_str = Self::parse_quality(media_item);
                                                                // Extract format type from JSON (hls, mp4, etc.)
                                                                let format_type = media_item.get("format")
                                                                    .and_then(|f| f.as_str())
                                                                    .unwrap_or("unknown");
                                                                let format = self.build_format(&quality_str, media_url.to_string(), format_type);

                                                                if ctx.config.verbose {
                                                                    eprintln!("[RedTube] Extracted format from JSON: {} - {} ({}x{}) [{}]",
                                                                        format.format_id,
                                                                        format.format_note.as_deref().unwrap_or("unknown"),
                                                                        format.width.unwrap_or(0),
                                                                        format.height.unwrap_or(0),
                                                                        format.ext.to_uppercase());
                                                                }

                                                                formats.push(format);
                                                            }
                                                        }
                                                            }
                                                            Err(e) => {
                                                                if ctx.config.verbose {
                                                                    eprintln!("[RedTube] Failed to parse formats JSON at {}:{}: {}",
                                                                        e.line(), e.column(), e);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        if ctx.config.verbose {
                                                            eprintln!("[RedTube] Failed to read response body: {e}");
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                if ctx.config.verbose {
                                                    if e.is_timeout() {
                                                        eprintln!("[RedTube] Request timed out for URL: {absolute_url}");
                                                    } else if e.is_connect() {
                                                        eprintln!("[RedTube] Connection failed for URL: {absolute_url}: {e}");
                                                    } else {
                                                        eprintln!("[RedTube] Request failed: {e}");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Has quality field, process directly
                                    let quality_str = Self::parse_quality(item);
                                    let format = self.build_format(&quality_str, video_url.to_string(), format_type);

                                    if ctx.config.verbose {
                                        eprintln!("[RedTube] Extracted format: {} - {} ({}x{})",
                                            format.format_id,
                                            format.format_note.as_deref().unwrap_or("unknown"),
                                            format.width.unwrap_or(0),
                                            format.height.unwrap_or(0));
                                    }

                                    formats.push(format);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if ctx.config.verbose {
                            eprintln!("[RedTube] Failed to parse mediaDefinition JSON at {}:{}: {}",
                                e.line(), e.column(), e);
                        }
                    }
                }
            }
        }

        formats
    }
}

impl Default for RedTubeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InfoExtractor for RedTubeExtractor {
    fn name(&self) -> &str {
        "RedTube"
    }

    fn valid_url(&self) -> &Regex {
        &REDTUBE_URL_PATTERN
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

        // Extract all data from HTML before any async operations
        let metadata = {
            let html = Html::parse_document(&webpage);

            // Extract metadata using base (includes title, description, uploader, thumbnail, and enhanced JSON-LD fields)
            self.base.extract_metadata(&html)?
        }; // html is dropped here

        // Try to extract video formats from JavaScript sources
        let mut formats = self.extract_from_sources(&webpage, ctx.config.verbose);

        // If sources didn't work, try mediaDefinition
        if formats.is_empty() {
            formats = self.extract_from_media_definition(&webpage, ctx).await;
        }

        // If both JavaScript methods failed, fall back to HTML <source> tags
        if formats.is_empty() {
            let video_data = {
                let html = Html::parse_document(&webpage);
                self.base.parse_video_sources(&html)
            };

            if !video_data.is_empty() {
                formats = self.base.build_formats(video_data, ctx).await;
            }
        }

        // Return error if still no sources found
        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video sources found in JavaScript or HTML. Video may be unavailable. URL: {url}"
            )));
        }

        // Convert relative URLs to absolute URLs
        let base_url = url::Url::parse(url)
            .map_err(|e| RdlpError::Extraction(format!("Invalid URL: {e}")))?;

        for format in &mut formats {
            // If URL doesn't start with http/https, it's relative
            if !format.url.starts_with("http://") && !format.url.starts_with("https://") {
                // Join with base URL
                if let Ok(absolute_url) = base_url.join(&format.url) {
                    format.url = absolute_url.to_string();
                }
            }
        }

        // Fetch filesizes for all formats
        for format in &mut formats {
            // HLS formats require special handling (parse m3u8 playlist)
            if format.ext == "hls" || format.url.contains(".m3u8") {
                let hls_detector = crate::hls::HlsSizeDetector::new(
                    ctx.http_client.clone(),
                    ctx.config.verbose,
                );

                match hls_detector.detect_size(&format.url).await {
                    Ok(Some(size)) => {
                        format.filesize = Some(size);
                        if ctx.config.verbose {
                            eprintln!(
                                "[RedTube] HLS {} size: {} MB",
                                format.format_id,
                                size / 1_000_000
                            );
                        }
                    }
                    Ok(None) => {
                        if ctx.config.verbose {
                            eprintln!(
                                "[RedTube] Could not detect HLS size for {}",
                                format.format_id
                            );
                        }
                    }
                    Err(e) => {
                        if ctx.config.verbose {
                            eprintln!("[RedTube] HLS size detection failed: {e}");
                        }
                    }
                }
            } else {
                // MP4 and other direct formats: use HEAD request
                if let Ok(response) = ctx.http_client.head(&format.url).send().await {
                    format.filesize = response.content_length();

                    // Fallback to Range request if HEAD returns no size
                    if format.filesize.is_none() || format.filesize == Some(0) {
                        if let Ok(range_response) = ctx.http_client
                            .get(&format.url)
                            .header("Range", "bytes=0-0")
                            .send()
                            .await
                        {
                            if let Some(content_range) = range_response.headers().get("content-range") {
                                if let Ok(range_str) = content_range.to_str() {
                                    if let Some(total) = range_str.split('/').nth(1) {
                                        format.filesize = total.parse::<u64>().ok();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Build InfoDict with all extracted metadata
        let mut info = InfoDict::new(video_id, metadata.title, self.name().to_string(), url.to_string());
        info.description = metadata.description;
        info.uploader = metadata.uploader;
        info.thumbnail = metadata.thumbnail;
        info.thumbnails = metadata.thumbnails;
        info.duration = metadata.duration;
        info.upload_date = metadata.upload_date;
        info.view_count = metadata.view_count;
        info.tags = metadata.tags;
        info.categories = metadata.categories;
        info.age_limit = Some(18); // RedTube is adult content
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

    /// Shared test fixture (compiled once, reused across all tests)
    static TEST_REDTUBE: Lazy<RedTubeExtractor> = Lazy::new(|| RedTubeExtractor::new());

    #[test]
    fn test_redtube_url_pattern() {
        let extractor = &*TEST_REDTUBE;
        assert!(extractor.suitable("https://www.redtube.com/123456"));
        assert!(extractor.suitable("https://redtube.com/12345678"));
        assert!(extractor.suitable("https://www.redtube.com.br/987654"));
        assert!(extractor.suitable("https://embed.redtube.com/?id=123456"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://www.tnaflix.com/video/123"));
    }

    #[test]
    fn test_extract_id() {
        let extractor = &*TEST_REDTUBE;

        let id1 = extractor.extract_id("https://www.redtube.com/123456");
        assert_eq!(id1, Some("123456".to_string()));

        let id2 = extractor.extract_id("https://redtube.com/12345678");
        assert_eq!(id2, Some("12345678".to_string()));

        let id3 = extractor.extract_id("https://www.redtube.com.br/987654");
        assert_eq!(id3, Some("987654".to_string()));

        let id4 = extractor.extract_id("https://embed.redtube.com/?id=555555");
        assert_eq!(id4, Some("555555".to_string()));
    }

    #[test]
    fn test_extract_from_sources() {
        let extractor = &*TEST_REDTUBE;

        let webpage = r#"
            var playerConfig = {
                sources: {"720": "https://example.com/720.mp4", "1080": "https://example.com/1080.mp4"},
                title: "Test Video"
            };
        "#;

        let formats = extractor.extract_from_sources(webpage, false);
        assert_eq!(formats.len(), 2);

        // Check that we got both formats
        assert!(formats.iter().any(|f| f.format_id == "720"));
        assert!(formats.iter().any(|f| f.format_id == "1080"));

        // Check format_note is set
        assert!(formats.iter().any(|f| f.format_note == Some("720p".to_string())));
        assert!(formats.iter().any(|f| f.format_note == Some("1080p".to_string())));
    }

    // Note: test_extract_from_media_definition is now an integration test
    // because it requires a full ExtractionContext with JsEngine and CookieJar.
    // The extraction is tested end-to-end with real RedTube URLs.

    #[test]
    fn test_extractor_name() {
        let extractor = &*TEST_REDTUBE;
        assert_eq!(extractor.name(), "RedTube");
    }

    #[test]
    fn test_extractor_priority() {
        let extractor = &*TEST_REDTUBE;
        assert_eq!(extractor.priority(), 0);
    }
}
