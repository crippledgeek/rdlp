use async_trait::async_trait;
use rdlp_core::{check_http_response, ExtractionContext, Format, InfoDict, InfoExtractor, Result, RdlpError};
use regex::Regex;
use scraper::{Html, Selector};

/// Video metadata extracted from HTML: (format_id, video_url, ext, height, width)
type VideoMetadata = (String, String, String, Option<u32>, Option<u32>);

/// TNAFlix network extractor (supports TNAFlix, EMPFlix, MovieFap)
pub struct TNAFlixExtractor {
    name: String,
    url_pattern: Regex,
}

impl TNAFlixExtractor {
    /// Create extractor for TNAFlix
    pub fn tnaflix() -> Self {
        Self {
            name: "TNAFlix".to_string(),
            url_pattern: Regex::new(
                r"https?://(?:www\.)?tnaflix\.com/[^/]+/[^/]+/video(\d+)"
            ).expect("Valid TNAFlix URL pattern"),
        }
    }

    /// Create extractor for EMPFlix
    pub fn empflix() -> Self {
        Self {
            name: "EMPFlix".to_string(),
            url_pattern: Regex::new(
                r"https?://(?:www\.)?empflix\.com/(?:videos/(?:[^/]+-)?\d+|[^/]+/\d+)"
            ).expect("Valid EMPFlix URL pattern"),
        }
    }

    /// Create extractor for MovieFap
    pub fn moviefap() -> Self {
        Self {
            name: "MovieFap".to_string(),
            url_pattern: Regex::new(
                r"https?://(?:www\.)?moviefap\.com/videos/([0-9a-f]+)/[^/]+\.html"
            ).expect("Valid MovieFap URL pattern"),
        }
    }

    /// Extract video ID from URL
    fn extract_id(&self, url: &str) -> Option<String> {
        self.url_pattern
            .captures(url)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Parse video source tags from HTML and extract formats
    fn parse_video_sources(&self, html: &Html, url: &str) -> Result<Vec<VideoMetadata>> {
        let mut video_data = Vec::new();

        // Parse <source> tags from the video player
        // Example: <source src="https://cdnl.tnaflix.com/.../video-720p.mp4" type="video/mp4" size="720">
        let source_selector = Selector::parse("source[src][type='video/mp4']")
            .expect("Valid CSS selector");

        for source_elem in html.select(&source_selector) {
            let video_url = source_elem.value().attr("src")
                .ok_or_else(|| RdlpError::Extraction(format!(
                    "Source tag missing src attribute. URL: {url}"
                )))?;

            // Extract quality from size attribute (e.g., "720", "480")
            let quality_str = source_elem.value().attr("size").unwrap_or("unknown");

            // Parse quality as integer height
            let height = quality_str.parse::<u32>().ok();

            // Calculate approximate width based on 16:9 aspect ratio
            let width = height.map(|h| (h * 16) / 9);

            // Determine extension from URL
            let ext = if video_url.contains(".mp4") {
                "mp4"
            } else if video_url.contains(".flv") {
                "flv"
            } else {
                "mp4" // default
            }.to_string();

            // Create format ID based on quality
            let format_id = if quality_str != "unknown" {
                format!("http-{quality_str}")
            } else {
                "http-default".to_string()
            };

            video_data.push((
                format_id,
                video_url.to_string(),
                ext,
                height,
                width,
            ));
        }

        if video_data.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video source tags found in HTML. Video may be unavailable. URL: {url}"
            )));
        }

        Ok(video_data)
    }

    /// Build formats from video data and fetch filesizes
    async fn build_formats(&self, video_data: Vec<VideoMetadata>, ctx: &ExtractionContext) -> Vec<Format> {
        let mut formats = Vec::new();

        for (format_id, video_url, ext, height, width) in video_data {
            // Create format with quality metadata
            let mut format = Format::new(
                format_id.clone(),
                video_url.clone(),
                ext.clone(),
                "https".to_string(),
            );

            // Set quality metadata
            format.height = height;
            format.width = width;
            format.format_note = height.map(|h| format!("{h}p"));

            // Set video and audio codecs (assume h264/aac for mp4)
            if ext == "mp4" {
                format.vcodec = Some("h264".to_string());
                format.acodec = Some("aac".to_string());
            }

            // Fetch filesize via HEAD request (or Range request if HEAD doesn't work)
            match ctx.http_client.head(&video_url).send().await {
                Ok(response) => {
                    if ctx.config.verbose {
                        eprintln!("HEAD response status: {}", response.status());
                        eprintln!("HEAD Content-Length: {:?}", response.content_length());
                        eprintln!("HEAD headers: {:#?}", response.headers());
                    }

                    format.filesize = response.content_length();

                    // If HEAD didn't give us content-length, try a Range request
                    if format.filesize.is_none() || format.filesize == Some(0) {
                        if ctx.config.verbose {
                            eprintln!("HEAD request returned no size, trying Range request...");
                        }

                        match ctx.http_client
                            .get(&video_url)
                            .header("Range", "bytes=0-0")
                            .send()
                            .await
                        {
                            Ok(range_response) => {
                                if ctx.config.verbose {
                                    eprintln!("Range response status: {}", range_response.status());
                                    eprintln!("Range Content-Range: {:?}", range_response.headers().get("content-range"));
                                }

                                // Parse Content-Range header: "bytes 0-0/123456"
                                if let Some(content_range) = range_response.headers().get("content-range") {
                                    if let Ok(range_str) = content_range.to_str() {
                                        if let Some(total) = range_str.split('/').nth(1) {
                                            format.filesize = total.parse::<u64>().ok();
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                if ctx.config.verbose {
                                    eprintln!("Range request also failed: {e}");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if ctx.config.verbose {
                        eprintln!("Warning: HEAD request failed for {video_url}: {e}");
                    }
                    // Continue without filesize
                }
            }

            formats.push(format);
        }

        formats
    }

    /// Extract metadata from HTML
    fn extract_metadata(&self, html: &Html) -> Result<(String, Option<String>, Option<String>)> {
        // Try to extract title from input field or h1
        let title_selector = Selector::parse(r#"input[name="title"]"#)
            .expect("Valid CSS selector for title");
        let h1_selector = Selector::parse("h1")
            .expect("Valid CSS selector for h1");

        let title = if let Some(input) = html.select(&title_selector).next() {
            input.value().attr("value").map(|s| s.to_string())
        } else { html.select(&h1_selector).next().map(|h1| h1.text().collect::<String>().trim().to_string()) }.ok_or_else(|| RdlpError::Extraction("Could not find video title".to_string()))?;

        // Try to extract description
        let desc_selector = Selector::parse(r#"input[name="description"]"#)
            .expect("Valid CSS selector for description");
        let description = html.select(&desc_selector).next()
            .and_then(|input| input.value().attr("value"))
            .map(|s| s.to_string());

        // Try to extract uploader
        let uploader_selector = Selector::parse(r#"input[name="username"]"#)
            .expect("Valid CSS selector for uploader");
        let uploader = html.select(&uploader_selector).next()
            .and_then(|input| input.value().attr("value"))
            .map(|s| s.to_string());

        Ok((title, description, uploader))
    }
}

#[async_trait]
impl InfoExtractor for TNAFlixExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn valid_url(&self) -> &Regex {
        &self.url_pattern
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
        let (title, description, uploader, thumbnail, video_data) = {
            let html = Html::parse_document(&webpage);

            // Extract metadata
            let (title, description, uploader) = self.extract_metadata(&html)?;

            // Extract thumbnail
            let thumb_selector = Selector::parse(r#"meta[property="og:image"]"#)
                .expect("Valid CSS selector for thumbnail");
            let thumbnail = html.select(&thumb_selector).next()
                .and_then(|thumb| thumb.value().attr("content"))
                .map(|s| s.to_string());

            // Parse video data from HTML (synchronous)
            let video_data = self.parse_video_sources(&html, url)?;

            (title, description, uploader, thumbnail, video_data)
        }; // html is dropped here

        // Build formats and fetch filesizes (asynchronous)
        let formats = self.build_formats(video_data, ctx).await;

        if formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found for URL: {url}"
            )));
        }

        // Build InfoDict
        let mut info = InfoDict::new(video_id, title, self.name.clone(), url.to_string());
        info.description = description;
        info.uploader = uploader;
        info.formats = formats;
        info.thumbnail = thumbnail;

        Ok(info)
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tnaflix_url_pattern() {
        let extractor = TNAFlixExtractor::tnaflix();
        assert!(extractor.suitable("https://www.tnaflix.com/hd-videos/test/video123456"));
        assert!(extractor.suitable("https://tnaflix.com/amateur-porn/title/video999"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
    }

    #[test]
    fn test_empflix_url_pattern() {
        let extractor = TNAFlixExtractor::empflix();
        assert!(extractor.suitable("https://www.empflix.com/videos/title-123"));
        assert!(extractor.suitable("https://empflix.com/view/123"));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_moviefap_url_pattern() {
        let extractor = TNAFlixExtractor::moviefap();
        assert!(extractor.suitable("https://www.moviefap.com/videos/abc123def/title.html"));
        assert!(!extractor.suitable("https://tnaflix.com/video/123"));
    }

    #[test]
    fn test_extract_id() {
        let extractor = TNAFlixExtractor::tnaflix();
        let id = extractor.extract_id("https://www.tnaflix.com/hd-videos/test/video123456");
        assert_eq!(id, Some("123456".to_string()));
    }
}
