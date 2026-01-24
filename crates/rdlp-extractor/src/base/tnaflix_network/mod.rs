//! Base extractor for TNAFlix network sites
//!
//! Provides shared extraction logic for TNAFlix, EMPFlix, and MovieFap sites
//! which all use similar HTML structures and metadata patterns.

mod formats;
mod json_ld;

#[cfg(test)]
mod tests;

use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, Format, Result, RdlpError};
use scraper::{Html, Selector};

// Re-export VideoMetadata type for external use
pub use formats::VideoMetadata;

// Re-export internal functions for testing
#[cfg(test)]
pub(crate) use json_ld::{
    extract_categories, extract_json_ld, extract_tags, extract_thumbnails, extract_view_count,
};

// ============================================================================
// Static CSS Selectors
// ============================================================================

/// Selector for title input field: <input name="title" value="...">
static TITLE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"input[name="title"]"#).expect("Valid CSS selector")
});

/// Selector for h1 title fallback
static H1_SELECTOR: Lazy<Selector> =
    Lazy::new(|| Selector::parse("h1").expect("Valid CSS selector"));

/// Selector for description input field
static DESC_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"input[name="description"]"#).expect("Valid CSS selector")
});

/// Selector for uploader input field
static UPLOADER_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"input[name="username"]"#).expect("Valid CSS selector")
});

/// Selector for Open Graph title
static OG_TITLE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:title"]"#).expect("Valid OG title selector")
});

/// Selector for Open Graph description
static OG_DESC_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:description"]"#).expect("Valid OG description selector")
});

/// Selector for meta description
static META_DESC_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="description"]"#).expect("Valid meta description selector")
});

/// Selector for HTML title tag
static TITLE_TAG_SELECTOR: Lazy<Selector> =
    Lazy::new(|| Selector::parse("title").expect("Valid title selector"));

/// Selector for Open Graph thumbnail
static THUMBNAIL_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:image"]"#).expect("Valid CSS selector")
});

/// Selector for Twitter card image
static TWITTER_IMAGE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="twitter:image"]"#).expect("Valid Twitter image selector")
});

/// Selector for link rel image_src
static LINK_IMAGE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"link[rel="image_src"]"#).expect("Valid link image selector")
});

// ============================================================================
// Extracted Metadata Structure
// ============================================================================

/// Container for all extracted metadata from HTML/JSON-LD
#[derive(Debug, Clone)]
pub struct ExtractedMetadata {
    pub title: String,
    pub description: Option<String>,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    pub thumbnails: Option<Vec<rdlp_core::Thumbnail>>,
    pub duration: Option<f64>,
    pub upload_date: Option<String>,
    pub view_count: Option<u64>,
    pub tags: Option<Vec<String>>,
    pub categories: Option<Vec<String>>,
}

// ============================================================================
// Base Extractor
// ============================================================================

/// Base extractor for TNAFlix network sites
///
/// Provides shared functionality for:
/// - Metadata extraction (title, description, uploader)
/// - Thumbnail extraction
/// - Format building with filesize detection
/// - Video source parsing from HTML
pub struct TnaFlixNetworkBase;

impl TnaFlixNetworkBase {
    /// Create a new base extractor
    pub fn new() -> Self {
        Self
    }

    // ========================================================================
    // Metadata Extraction (delegates to json_ld module for JSON-LD parsing)
    // ========================================================================

    /// Extract video title from HTML using multiple strategies
    pub fn extract_title(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD
        if let Some(json_ld) = json_ld::extract_json_ld(html) {
            if let Some(name) = json_ld.name {
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }

        // Strategy 2: Open Graph title
        if let Some(og_title) = extract_meta_content(html, &OG_TITLE_SELECTOR) {
            if !og_title.is_empty() {
                return Some(og_title);
            }
        }

        // Strategy 3: Input field
        if let Some(input) = html.select(&TITLE_SELECTOR).next() {
            if let Some(value) = input.value().attr("value") {
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }

        // Strategy 4: H1 tag
        if let Some(h1) = html.select(&H1_SELECTOR).next() {
            let text = h1.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }

        // Strategy 5: HTML title tag
        html.select(&TITLE_TAG_SELECTOR)
            .next()
            .map(|title| title.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract video description from HTML using multiple strategies
    pub fn extract_description(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD
        if let Some(json_ld) = json_ld::extract_json_ld(html) {
            if let Some(desc) = json_ld.description {
                if !desc.is_empty() {
                    return Some(desc);
                }
            }
        }

        // Strategy 2: Open Graph description
        if let Some(og_desc) = extract_meta_content(html, &OG_DESC_SELECTOR) {
            if !og_desc.is_empty() {
                return Some(og_desc);
            }
        }

        // Strategy 3: Meta description
        if let Some(meta_desc) = extract_meta_content(html, &META_DESC_SELECTOR) {
            if !meta_desc.is_empty() {
                return Some(meta_desc);
            }
        }

        // Strategy 4: Input field
        html.select(&DESC_SELECTOR)
            .next()
            .and_then(|input| input.value().attr("value"))
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract uploader username from HTML
    pub fn extract_uploader(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD author
        if let Some(json_ld) = json_ld::extract_json_ld(html) {
            if let Some(author) = json_ld.author {
                if let Some(name) = author.name {
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }

        // Strategy 2: Input field
        html.select(&UPLOADER_SELECTOR)
            .next()
            .and_then(|input| input.value().attr("value"))
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract thumbnail URL from HTML using multiple strategies
    pub fn extract_thumbnail(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD thumbnailUrl
        if let Some(json_ld) = json_ld::extract_json_ld(html) {
            if let Some(url) = json_ld::get_thumbnail_url(&json_ld) {
                return Some(url);
            }
        }

        // Strategy 2: Open Graph image
        if let Some(og_image) = extract_meta_content(html, &THUMBNAIL_SELECTOR) {
            if !og_image.is_empty() {
                return Some(og_image);
            }
        }

        // Strategy 3: Twitter card image
        if let Some(twitter_image) = extract_meta_content(html, &TWITTER_IMAGE_SELECTOR) {
            if !twitter_image.is_empty() {
                return Some(twitter_image);
            }
        }

        // Strategy 4: Link rel image_src
        html.select(&LINK_IMAGE_SELECTOR)
            .next()
            .and_then(|link| link.value().attr("href"))
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract all metadata from HTML
    pub fn extract_metadata(&self, html: &Html) -> Result<ExtractedMetadata> {
        let title = self
            .extract_title(html)
            .ok_or_else(|| RdlpError::Extraction("Could not find video title".to_string()))?;

        let description = self.extract_description(html);
        let uploader = self.extract_uploader(html);
        let thumbnail = self.extract_thumbnail(html);

        // Try to extract enhanced metadata from JSON-LD
        let json_ld_opt = json_ld::extract_json_ld(html);
        let (thumbnails, duration, upload_date, view_count, tags, categories) =
            if let Some(ref json_ld) = json_ld_opt {
                (
                    json_ld::extract_thumbnails(json_ld),
                    json_ld
                        .duration
                        .as_ref()
                        .and_then(|d| self.parse_iso8601_duration(d)),
                    json_ld
                        .upload_date
                        .as_ref()
                        .and_then(|d| self.parse_iso8601_date(d)),
                    json_ld::extract_view_count(json_ld),
                    json_ld::extract_tags(json_ld),
                    json_ld::extract_categories(json_ld),
                )
            } else {
                (None, None, None, None, None, None)
            };

        Ok(ExtractedMetadata {
            title,
            description,
            uploader,
            thumbnail,
            thumbnails,
            duration,
            upload_date,
            view_count,
            tags,
            categories,
        })
    }

    // ========================================================================
    // ISO 8601 Parsing
    // ========================================================================

    /// Parse ISO 8601 duration string to seconds
    ///
    /// # Examples
    /// ```
    /// # use rdlp_extractor::base::tnaflix_network::TnaFlixNetworkBase;
    /// let base = TnaFlixNetworkBase::new();
    /// assert_eq!(base.parse_iso8601_duration("PT1H2M3S"), Some(3723.0));
    /// assert_eq!(base.parse_iso8601_duration("PT30M"), Some(1800.0));
    /// assert_eq!(base.parse_iso8601_duration("PT45S"), Some(45.0));
    /// ```
    pub fn parse_iso8601_duration(&self, duration_str: &str) -> Option<f64> {
        if !duration_str.starts_with("PT") {
            return None;
        }

        let duration_part = &duration_str[2..];
        let mut hours = 0.0;
        let mut minutes = 0.0;
        let mut seconds = 0.0;

        let mut current_num = String::new();
        for ch in duration_part.chars() {
            match ch {
                '0'..='9' | '.' => current_num.push(ch),
                'H' => {
                    if let Ok(h) = current_num.parse::<f64>() {
                        hours = h;
                    }
                    current_num.clear();
                }
                'M' => {
                    if let Ok(m) = current_num.parse::<f64>() {
                        minutes = m;
                    }
                    current_num.clear();
                }
                'S' => {
                    if let Ok(s) = current_num.parse::<f64>() {
                        seconds = s;
                    }
                    current_num.clear();
                }
                _ => return None,
            }
        }

        Some(hours * 3600.0 + minutes * 60.0 + seconds)
    }

    /// Parse ISO 8601 date/datetime string to YYYYMMDD format
    ///
    /// # Examples
    /// ```
    /// # use rdlp_extractor::base::tnaflix_network::TnaFlixNetworkBase;
    /// let base = TnaFlixNetworkBase::new();
    /// assert_eq!(base.parse_iso8601_date("2024-01-15"), Some("20240115".to_string()));
    /// assert_eq!(base.parse_iso8601_date("2024-01-15T10:30:00Z"), Some("20240115".to_string()));
    /// ```
    pub fn parse_iso8601_date(&self, date_str: &str) -> Option<String> {
        let date_part = date_str.split('T').next().unwrap_or(date_str);
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() != 3 {
            return None;
        }

        let year = parts[0];
        let month = parts[1];
        let day = parts[2];

        if year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return None;
        }

        if !year.chars().all(|c| c.is_ascii_digit())
            || !month.chars().all(|c| c.is_ascii_digit())
            || !day.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }

        Some(format!("{year}{month}{day}"))
    }

    // ========================================================================
    // Format Building (delegates to formats module)
    // ========================================================================

    /// Parse video source tags from HTML
    pub fn parse_video_sources(&self, html: &Html) -> Vec<VideoMetadata> {
        formats::parse_video_sources(html)
    }

    /// Extract config URL from HTML
    pub fn extract_config_url(&self, html_text: &str) -> Option<String> {
        formats::extract_config_url(html_text)
    }

    /// Extract cdn.php URL from MovieFap JavaScript
    pub fn extract_cdn_url(&self, webpage: &str) -> Option<String> {
        formats::extract_cdn_url(webpage)
    }

    /// Parse MovieFap XML response to extract video sources
    pub fn parse_moviefap_xml(&self, xml_text: &str) -> Vec<VideoMetadata> {
        formats::parse_moviefap_xml(xml_text)
    }

    /// Build format list from video metadata and fetch filesizes
    pub async fn build_formats(
        &self,
        video_data: Vec<VideoMetadata>,
        ctx: &ExtractionContext,
    ) -> Vec<Format> {
        formats::build_formats(video_data, ctx).await
    }
}

impl Default for TnaFlixNetworkBase {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract meta tag content attribute
fn extract_meta_content(html: &Html, selector: &Selector) -> Option<String> {
    html.select(selector)
        .next()
        .and_then(|meta| meta.value().attr("content"))
        .map(|s| s.to_string())
}
