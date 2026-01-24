//! Base extractor for TNAFlix network sites
//!
//! Provides shared extraction logic for TNAFlix, EMPFlix, and MovieFap sites
//! which all use similar HTML structures and metadata patterns.

#[cfg(test)]
mod tests;

use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, Format, Result, RdlpError};
use regex::Regex;
use scraper::{Html, Selector};
use serde::Deserialize;

/// Video metadata extracted from HTML: (format_id, video_url, ext, height, width)
pub type VideoMetadata = (String, String, String, Option<u32>, Option<u32>);

// ============================================================================
// Static CSS Selectors (initialized once at first use)
// ============================================================================

/// Selector for video source tags: <source src="..." type="video/mp4">
pub static SOURCE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("source[src][type='video/mp4']").expect("Valid CSS selector")
});

/// Selector for title input field: <input name="title" value="...">
pub static TITLE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"input[name="title"]"#).expect("Valid CSS selector")
});

/// Selector for h1 title fallback
pub static H1_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("h1").expect("Valid CSS selector")
});

/// Selector for description input field: <input name="description" value="...">
pub static DESC_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"input[name="description"]"#).expect("Valid CSS selector")
});

/// Selector for uploader input field: <input name="username" value="...">
pub static UPLOADER_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"input[name="username"]"#).expect("Valid CSS selector")
});

/// Selector for Open Graph thumbnail: <meta property="og:image" content="...">
pub static THUMBNAIL_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:image"]"#).expect("Valid CSS selector")
});

// ============================================================================
// Static Regex Patterns (initialized once at first use)
// ============================================================================

/// Regex to extract CDN URL from MovieFap JavaScript: url: 'http://.../cdn.php...'
pub static CDN_URL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"url:\s*['"]([^'"]+/cdn\.php[^'"]+)['"]"#).expect("Valid CDN URL regex")
});

/// Regex to extract video items from MovieFap XML
pub static MOVIEFAP_XML_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?s)<item>.*?<res>([^<]+)</res>.*?<videoLink>([^<]+)</videoLink>.*?</item>")
        .expect("Valid MovieFap XML regex")
});

/// Regex patterns for extracting config URLs (multiple fallback strategies)
///
/// **Strategy 1**: flashvars.config = escape("URL")
/// **Strategy 2**: <input name="config..." value="URL">
/// **Strategy 3**: config = "URL" or config = 'URL'
pub static CONFIG_URL_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r#"flashvars\.config\s*=\s*escape\("([^"]+)""#).expect("Valid config pattern 1"),
        Regex::new(r#"<input[^>]+name="config\d?"[^>]+value="([^"]+)""#).expect("Valid config pattern 2"),
        Regex::new(r#"config\s*=\s*["']([^"']+)["']"#).expect("Valid config pattern 3"),
    ]
});

// ============================================================================
// JSON-LD Structures
// ============================================================================

/// JSON-LD VideoObject structure for structured metadata extraction
#[derive(Debug, Deserialize)]
pub struct JsonLdVideo {
    #[serde(rename = "@type")]
    pub json_type: String,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "thumbnailUrl")]
    pub thumbnail_url: Option<JsonLdThumbnail>,
    #[serde(rename = "uploadDate")]
    pub upload_date: Option<String>, // ISO 8601 date (2024-01-15T10:30:00Z)
    pub duration: Option<String>,     // ISO 8601 duration (PT1H2M3S)
    pub author: Option<JsonLdAuthor>,
    #[serde(rename = "interactionStatistic")]
    pub interaction_statistic: Option<JsonLdInteractionStatistic>,
    pub keywords: Option<JsonLdKeywords>, // Can be string or array
    pub genre: Option<JsonLdGenre>,       // Can be string or array
    #[serde(rename = "contentUrl")]
    pub content_url: Option<String>,
}

/// JSON-LD thumbnail can be string or array
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdThumbnail {
    Single(String),
    Multiple(Vec<String>),
}

/// JSON-LD author structure
#[derive(Debug, Deserialize)]
pub struct JsonLdAuthor {
    #[serde(rename = "@type")]
    pub author_type: Option<String>,
    pub name: Option<String>,
}

/// JSON-LD interaction statistic for view counts, likes, etc.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdInteractionStatistic {
    Single(JsonLdInteraction),
    Multiple(Vec<JsonLdInteraction>),
}

/// Individual interaction statistic entry
#[derive(Debug, Deserialize)]
pub struct JsonLdInteraction {
    #[serde(rename = "@type")]
    pub interaction_type: String,
    #[serde(rename = "interactionType")]
    pub interaction_type_url: Option<String>,
    #[serde(rename = "userInteractionCount")]
    pub user_interaction_count: Option<u64>,
}

/// JSON-LD keywords can be string (comma-separated) or array
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdKeywords {
    Single(String),
    Multiple(Vec<String>),
}

/// JSON-LD genre can be string or array
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdGenre {
    Single(String),
    Multiple(Vec<String>),
}

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
// Additional Static Selectors for Multi-Strategy Extraction
// ============================================================================

/// Selector for JSON-LD script tags: <script type="application/ld+json">
static JSONLD_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"script[type="application/ld+json"]"#).expect("Valid JSON-LD selector")
});

/// Selector for Open Graph title: <meta property="og:title" content="...">
static OG_TITLE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:title"]"#).expect("Valid OG title selector")
});

/// Selector for Open Graph description: <meta property="og:description" content="...">
static OG_DESC_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[property="og:description"]"#).expect("Valid OG description selector")
});

/// Selector for meta description: <meta name="description" content="...">
static META_DESC_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="description"]"#).expect("Valid meta description selector")
});

/// Selector for HTML title tag: <title>...</title>
static TITLE_TAG_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse("title").expect("Valid title selector")
});

/// Selector for Twitter card image: <meta name="twitter:image" content="...">
static TWITTER_IMAGE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"meta[name="twitter:image"]"#).expect("Valid Twitter image selector")
});

/// Selector for link rel image_src: <link rel="image_src" href="...">
static LINK_IMAGE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
    Selector::parse(r#"link[rel="image_src"]"#).expect("Valid link image selector")
});

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
///
/// # Usage
///
/// ```ignore
/// use rdlp_extractor::base::tnaflix_network::TnaFlixNetworkBase;
///
/// let base = TnaFlixNetworkBase::new();
///
/// // Extract title from HTML
/// let html = scraper::Html::parse_document("<input name='title' value='Test Video'>");
/// let title = base.extract_title(&html);
/// ```
pub struct TnaFlixNetworkBase;

impl TnaFlixNetworkBase {
    /// Create a new base extractor
    pub fn new() -> Self {
        Self
    }

    // ========================================================================
    // JSON-LD and Open Graph Helpers
    // ========================================================================

    /// Extract JSON-LD VideoObject from HTML
    ///
    /// Looks for `<script type="application/ld+json">` tags and parses them
    /// as VideoObject structures.
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// VideoObject if found and successfully parsed, `None` otherwise
    pub(crate) fn extract_json_ld(&self, html: &Html) -> Option<JsonLdVideo> {
        for script_elem in html.select(&JSONLD_SELECTOR) {
            let json_text = script_elem.text().collect::<String>();

            // Try to parse as JSON-LD
            if let Ok(json_ld) = serde_json::from_str::<JsonLdVideo>(&json_text) {
                // Only return if it's actually a VideoObject
                if json_ld.json_type == "VideoObject" {
                    return Some(json_ld);
                }
            }
        }
        None
    }

    /// Extract Open Graph property content
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the meta tag
    ///
    /// # Returns
    /// Content attribute value if found, `None` otherwise
    fn extract_og_property(&self, html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .and_then(|meta| meta.value().attr("content"))
            .map(|s| s.to_string())
    }

    /// Extract meta tag content by name
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the meta tag
    ///
    /// # Returns
    /// Content attribute value if found, `None` otherwise
    fn extract_meta_content(&self, html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .and_then(|meta| meta.value().attr("content"))
            .map(|s| s.to_string())
    }

    /// Parse ISO 8601 duration string to seconds
    ///
    /// Supports formats like:
    /// - PT30S (30 seconds)
    /// - PT5M (5 minutes = 300 seconds)
    /// - PT1H (1 hour = 3600 seconds)
    /// - PT1H30M45S (1 hour, 30 minutes, 45 seconds = 5445 seconds)
    ///
    /// # Arguments
    /// * `duration_str` - ISO 8601 duration string (e.g., "PT1H2M3S")
    ///
    /// # Returns
    /// Duration in seconds as f64, or `None` if parsing fails
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
        // ISO 8601 duration format: PT[hours]H[minutes]M[seconds]S
        if !duration_str.starts_with("PT") {
            return None;
        }

        let duration_part = &duration_str[2..]; // Remove "PT" prefix
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
                _ => return None, // Invalid character
            }
        }

        Some(hours * 3600.0 + minutes * 60.0 + seconds)
    }

    /// Parse ISO 8601 date/datetime string to YYYYMMDD format
    ///
    /// Supports formats like:
    /// - 2024-01-15
    /// - 2024-01-15T10:30:00Z
    /// - 2024-01-15T10:30:00+00:00
    ///
    /// # Arguments
    /// * `date_str` - ISO 8601 date string
    ///
    /// # Returns
    /// Date in YYYYMMDD format, or `None` if parsing fails
    ///
    /// # Examples
    /// ```
    /// # use rdlp_extractor::base::tnaflix_network::TnaFlixNetworkBase;
    /// let base = TnaFlixNetworkBase::new();
    /// assert_eq!(base.parse_iso8601_date("2024-01-15"), Some("20240115".to_string()));
    /// assert_eq!(base.parse_iso8601_date("2024-01-15T10:30:00Z"), Some("20240115".to_string()));
    /// ```
    pub fn parse_iso8601_date(&self, date_str: &str) -> Option<String> {
        // Extract just the date part (YYYY-MM-DD)
        let date_part = date_str.split('T').next().unwrap_or(date_str);

        // Parse YYYY-MM-DD format
        let parts: Vec<&str> = date_part.split('-').collect();
        if parts.len() != 3 {
            return None;
        }

        let year = parts[0];
        let month = parts[1];
        let day = parts[2];

        // Validate lengths
        if year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return None;
        }

        // Validate they're all numeric
        if !year.chars().all(|c| c.is_ascii_digit())
            || !month.chars().all(|c| c.is_ascii_digit())
            || !day.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }

        Some(format!("{year}{month}{day}"))
    }

    /// Extract view count from JSON-LD interaction statistics
    ///
    /// # Arguments
    /// * `json_ld` - Parsed JSON-LD VideoObject
    ///
    /// # Returns
    /// View count if found, `None` otherwise
    pub(crate) fn extract_view_count(&self, json_ld: &JsonLdVideo) -> Option<u64> {
        json_ld.interaction_statistic.as_ref().and_then(|stats| {
            let interactions = match stats {
                JsonLdInteractionStatistic::Single(interaction) => vec![interaction],
                JsonLdInteractionStatistic::Multiple(interactions) => {
                    interactions.iter().collect()
                }
            };

            // Look for WatchAction type
            for interaction in interactions {
                if interaction.interaction_type == "WatchAction"
                    || interaction.interaction_type_url
                        .as_ref()
                        .is_some_and(|url| url.contains("WatchAction"))
                {
                    return interaction.user_interaction_count;
                }
            }

            None
        })
    }

    /// Extract tags/keywords from JSON-LD
    ///
    /// # Arguments
    /// * `json_ld` - Parsed JSON-LD VideoObject
    ///
    /// # Returns
    /// Vector of tags if found, `None` otherwise
    pub(crate) fn extract_tags(&self, json_ld: &JsonLdVideo) -> Option<Vec<String>> {
        json_ld.keywords.as_ref().map(|keywords| match keywords {
            JsonLdKeywords::Single(s) => {
                // Split comma-separated string
                s.split(',')
                    .map(|tag| tag.trim().to_string())
                    .filter(|tag| !tag.is_empty())
                    .collect()
            }
            JsonLdKeywords::Multiple(vec) => vec.clone(),
        })
    }

    /// Extract categories/genres from JSON-LD
    ///
    /// # Arguments
    /// * `json_ld` - Parsed JSON-LD VideoObject
    ///
    /// # Returns
    /// Vector of categories if found, `None` otherwise
    pub(crate) fn extract_categories(&self, json_ld: &JsonLdVideo) -> Option<Vec<String>> {
        json_ld.genre.as_ref().map(|genre| match genre {
            JsonLdGenre::Single(s) => vec![s.clone()],
            JsonLdGenre::Multiple(vec) => vec.clone(),
        })
    }

    /// Create thumbnail list from JSON-LD thumbnailUrl field
    ///
    /// # Arguments
    /// * `json_ld` - Parsed JSON-LD VideoObject
    ///
    /// # Returns
    /// Vector of Thumbnail structs if found, `None` otherwise
    pub(crate) fn extract_thumbnails(&self, json_ld: &JsonLdVideo) -> Option<Vec<rdlp_core::Thumbnail>> {
        json_ld.thumbnail_url.as_ref().map(|thumb| {
            let urls = match thumb {
                JsonLdThumbnail::Single(url) => vec![url.clone()],
                JsonLdThumbnail::Multiple(urls) => urls.clone(),
            };

            urls.iter()
                .enumerate()
                .filter(|(_, url)| !url.is_empty())
                .map(|(idx, url)| rdlp_core::Thumbnail {
                    url: url.clone(),
                    id: Some(format!("jsonld_{idx}")),
                    width: None,
                    height: None,
                    preference: Some(-(idx as i32)), // Earlier thumbnails are preferred
                })
                .collect()
        })
    }

    // ========================================================================
    // Metadata Extraction
    // ========================================================================

    /// Extract video title from HTML
    ///
    /// Tries multiple strategies in order:
    /// 1. JSON-LD VideoObject name
    /// 2. Open Graph title (`og:title`)
    /// 3. Input field: `<input name="title" value="...">`
    /// 4. H1 tag: `<h1>...</h1>`
    /// 5. HTML title tag: `<title>...</title>`
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Video title if found using any strategy, `None` otherwise
    pub fn extract_title(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD
        if let Some(json_ld) = self.extract_json_ld(html) {
            if let Some(name) = json_ld.name {
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }

        // Strategy 2: Open Graph title
        if let Some(og_title) = self.extract_og_property(html, &OG_TITLE_SELECTOR) {
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

    /// Extract video description from HTML
    ///
    /// Tries multiple strategies in order:
    /// 1. JSON-LD VideoObject description
    /// 2. Open Graph description (`og:description`)
    /// 3. Meta description tag
    /// 4. Input field: `<input name="description" value="...">`
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Video description if found using any strategy, `None` otherwise
    pub fn extract_description(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD
        if let Some(json_ld) = self.extract_json_ld(html) {
            if let Some(desc) = json_ld.description {
                if !desc.is_empty() {
                    return Some(desc);
                }
            }
        }

        // Strategy 2: Open Graph description
        if let Some(og_desc) = self.extract_og_property(html, &OG_DESC_SELECTOR) {
            if !og_desc.is_empty() {
                return Some(og_desc);
            }
        }

        // Strategy 3: Meta description
        if let Some(meta_desc) = self.extract_meta_content(html, &META_DESC_SELECTOR) {
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
    ///
    /// Tries multiple strategies in order:
    /// 1. JSON-LD VideoObject author name
    /// 2. Input field: `<input name="username" value="...">`
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Uploader username if found using any strategy, `None` otherwise
    pub fn extract_uploader(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD author
        if let Some(json_ld) = self.extract_json_ld(html) {
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

    /// Extract all metadata from HTML (title, description, uploader)
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Tuple of (title, description, uploader). Only title is required.
    ///
    /// # Errors
    /// Returns error if title cannot be extracted
    pub fn extract_metadata(&self, html: &Html) -> Result<ExtractedMetadata> {
        let title = self
            .extract_title(html)
            .ok_or_else(|| RdlpError::Extraction("Could not find video title".to_string()))?;

        let description = self.extract_description(html);
        let uploader = self.extract_uploader(html);
        let thumbnail = self.extract_thumbnail(html);

        // Try to extract enhanced metadata from JSON-LD
        let json_ld = self.extract_json_ld(html);
        let (thumbnails, duration, upload_date, view_count, tags, categories) = if let Some(ref json_ld) = json_ld {
            (
                self.extract_thumbnails(json_ld),
                json_ld.duration.as_ref().and_then(|d| self.parse_iso8601_duration(d)),
                json_ld.upload_date.as_ref().and_then(|d| self.parse_iso8601_date(d)),
                self.extract_view_count(json_ld),
                self.extract_tags(json_ld),
                self.extract_categories(json_ld),
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

    /// Extract thumbnail URL from HTML
    ///
    /// Tries multiple strategies in order:
    /// 1. JSON-LD VideoObject thumbnailUrl
    /// 2. Open Graph image (`og:image`)
    /// 3. Twitter card image (`twitter:image`)
    /// 4. Link rel image_src
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Thumbnail URL if found using any strategy, `None` otherwise
    pub fn extract_thumbnail(&self, html: &Html) -> Option<String> {
        // Strategy 1: JSON-LD thumbnailUrl
        if let Some(json_ld) = self.extract_json_ld(html) {
            if let Some(thumbnail) = json_ld.thumbnail_url {
                let url = match thumbnail {
                    JsonLdThumbnail::Single(url) => Some(url),
                    JsonLdThumbnail::Multiple(urls) => urls.into_iter().next(),
                };
                if let Some(url_str) = url {
                    if !url_str.is_empty() {
                        return Some(url_str);
                    }
                }
            }
        }

        // Strategy 2: Open Graph image
        if let Some(og_image) = self.extract_og_property(html, &THUMBNAIL_SELECTOR) {
            if !og_image.is_empty() {
                return Some(og_image);
            }
        }

        // Strategy 3: Twitter card image
        if let Some(twitter_image) = self.extract_meta_content(html, &TWITTER_IMAGE_SELECTOR) {
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

    // ========================================================================
    // Config URL Extraction (Multi-Pattern)
    // ========================================================================

    /// Extract config URL from HTML using multiple fallback patterns
    ///
    /// Tries three strategies in order:
    /// 1. `flashvars.config = escape("URL")`
    /// 2. `<input name="config..." value="URL">`
    /// 3. `config = "URL"` or `config = 'URL'`
    ///
    /// # Arguments
    /// * `html_text` - Raw HTML text
    ///
    /// # Returns
    /// Config URL if found using any pattern, `None` otherwise
    ///
    /// # Example
    ///
    /// ```ignore
    /// # use rdlp_extractor::base::tnaflix_network::TnaFlixNetworkBase;
    /// let base = TnaFlixNetworkBase::new();
    ///
    /// let html = r#"flashvars.config = escape("http://example.com/config.xml");"#;
    /// let url = base.extract_config_url(html);
    /// assert_eq!(url, Some("http://example.com/config.xml".to_string()));
    /// ```
    pub fn extract_config_url(&self, html_text: &str) -> Option<String> {
        for pattern in CONFIG_URL_PATTERNS.iter() {
            if let Some(caps) = pattern.captures(html_text) {
                if let Some(url_match) = caps.get(1) {
                    return Some(url_match.as_str().to_string());
                }
            }
        }
        None
    }

    // ========================================================================
    // Video Source Parsing
    // ========================================================================

    /// Parse video source tags from HTML and extract format metadata
    ///
    /// Looks for: `<source src="..." type="video/mp4" size="720">`
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Vector of video metadata tuples (format_id, url, ext, height, width).
    /// Returns empty vector if no sources found (allows caller to try fallback).
    ///
    /// # Format ID Generation
    /// - With size attribute: `http-720`, `http-480`, etc.
    /// - Without size: `http-default`
    pub fn parse_video_sources(&self, html: &Html) -> Vec<VideoMetadata> {
        let mut video_data = Vec::new();

        for source_elem in html.select(&SOURCE_SELECTOR) {
            // Extract video URL from src attribute
            let video_url = match source_elem.value().attr("src") {
                Some(url) => url,
                None => continue, // Skip sources without src
            };

            // Extract quality from size attribute (e.g., "720", "480")
            let quality_str = source_elem.value().attr("size").unwrap_or("unknown");

            // Parse quality as integer height
            let height = quality_str.parse::<u32>().ok();

            // Calculate approximate width based on 16:9 aspect ratio
            let width = height.map(|h| (h * 16) / 9);

            // Determine extension from URL path (not from type attribute)
            let ext = if let Ok(parsed_url) = url::Url::parse(video_url) {
                if let Some(mut path_segments) = parsed_url.path_segments() {
                    if let Some(last_segment) = path_segments.next_back() {
                        // Extract extension from filename
                        if let Some(ext_start) = last_segment.rfind('.') {
                            let extension = &last_segment[ext_start + 1..];
                            match extension {
                                "mp4" => "mp4",
                                "flv" => "flv",
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
            }
            .to_string();

            // Create format ID based on quality
            let format_id = if quality_str != "unknown" {
                format!("http-{quality_str}")
            } else {
                "http-default".to_string()
            };

            video_data.push((format_id, video_url.to_string(), ext, height, width));
        }

        video_data
    }

    // ========================================================================
    // Format Building
    // ========================================================================

    /// Build format list from video metadata and fetch filesizes
    ///
    /// For each video source:
    /// 1. Creates Format struct with quality metadata
    /// 2. Sets video/audio codecs (h264/aac for MP4)
    /// 3. Fetches filesize via HEAD request
    /// 4. Falls back to Range request if HEAD doesn't return size
    ///
    /// # Arguments
    /// * `video_data` - Vector of video metadata tuples
    /// * `ctx` - Extraction context with HTTP client and config
    ///
    /// # Returns
    /// Vector of Format objects with filesizes populated
    ///
    /// # Filesize Detection Strategy
    /// 1. **HEAD request**: Fast, gets Content-Length header
    /// 2. **Range request**: Fallback, parses Content-Range header (bytes 0-0/total)
    /// 3. **Skip**: If both fail, continues without filesize
    pub async fn build_formats(
        &self,
        video_data: Vec<VideoMetadata>,
        ctx: &ExtractionContext,
    ) -> Vec<Format> {
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

            // Fetch filesize via HEAD request
            match ctx.http_client.head(&video_url).send().await {
                Ok(response) => {
                    if ctx.config.verbose {
                        eprintln!("HEAD response status: {}", response.status());
                        eprintln!("HEAD Content-Length: {:?}", response.content_length());
                    }

                    format.filesize = response.content_length();

                    // Fallback: If HEAD didn't give us content-length, try Range request
                    if format.filesize.is_none() || format.filesize == Some(0) {
                        if ctx.config.verbose {
                            eprintln!("HEAD request returned no size, trying Range request...");
                        }

                        if let Ok(range_response) = ctx
                            .http_client
                            .get(&video_url)
                            .header("Range", "bytes=0-0")
                            .send()
                            .await
                        {
                            if ctx.config.verbose {
                                eprintln!("Range response status: {}", range_response.status());
                            }

                            // Parse Content-Range header: "bytes 0-0/123456"
                            if let Some(content_range) =
                                range_response.headers().get("content-range")
                            {
                                if let Ok(range_str) = content_range.to_str() {
                                    if let Some(total) = range_str.split('/').nth(1) {
                                        format.filesize = total.parse::<u64>().ok();
                                    }
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

    // ========================================================================
    // MovieFap-specific helpers
    // ========================================================================

    /// Extract cdn.php URL from MovieFap JavaScript
    ///
    /// Looks for: `url: 'https://www.moviefap.com/cdn.php?file=....',`
    ///
    /// # Arguments
    /// * `webpage` - Raw HTML text
    ///
    /// # Returns
    /// CDN URL if found, `None` otherwise
    pub fn extract_cdn_url(&self, webpage: &str) -> Option<String> {
        CDN_URL_REGEX
            .captures(webpage)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Parse MovieFap XML response to extract video sources
    ///
    /// XML structure:
    /// ```xml
    /// <quality>
    ///   <item>
    ///     <res>720p</res>
    ///     <videoLink>http://example.com/video.mp4</videoLink>
    ///   </item>
    /// </quality>
    /// ```
    ///
    /// # Arguments
    /// * `xml_text` - Raw XML response from cdn.php
    ///
    /// # Returns
    /// Vector of video metadata tuples
    pub fn parse_moviefap_xml(&self, xml_text: &str) -> Vec<VideoMetadata> {
        let mut video_data = Vec::new();

        for cap in MOVIEFAP_XML_REGEX.captures_iter(xml_text) {
            let quality_str = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("unknown");
            let video_url = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");

            if video_url.is_empty() {
                continue;
            }

            // Decode HTML entities (&amp; -> &)
            let video_url = video_url.replace("&amp;", "&");

            // Parse quality (e.g., "720p" -> 720)
            let height = quality_str.trim_end_matches('p').parse::<u32>().ok();
            let width = height.map(|h| (h * 16) / 9);

            // Determine extension from URL path
            let ext = if let Ok(parsed_url) = url::Url::parse(&video_url) {
                if let Some(mut path_segments) = parsed_url.path_segments() {
                    if let Some(last_segment) = path_segments.next_back() {
                        // Extract extension from filename
                        if let Some(ext_start) = last_segment.rfind('.') {
                            let extension = &last_segment[ext_start + 1..];
                            match extension {
                                "mp4" => "mp4",
                                "flv" => "flv",
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
            }
            .to_string();

            // Create format ID based on quality
            let format_id = if let Some(h) = height {
                format!("http-{h}")
            } else {
                "http-default".to_string()
            };

            video_data.push((format_id, video_url, ext, height, width));
        }

        video_data
    }
}

impl Default for TnaFlixNetworkBase {
    fn default() -> Self {
        Self::new()
    }
}
