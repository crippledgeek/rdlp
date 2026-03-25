//! XTits extractor
//!
//! XTits is a KVS (Kernel Video Sharing) tube site serving direct MP4 downloads.
//!
//! Supports:
//! - Video pages: `https://www.xtits.xxx/videos/183207/slug/`
//! - Embed pages: `https://www.xtits.xxx/embed/183207`
//!
//! ## Module Structure
//!
//! - `patterns` - URL and flashvars regex patterns
//! - `formats` - KVS flashvars format extraction

mod formats;
mod patterns;

use async_trait::async_trait;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::InfoDict;
use regex::Regex;
use scraper::Html;

use crate::base::common::BaseExtractor;

/// Base URL for making relative hrefs absolute.
const XTITS_BASE_URL: &str = "https://www.xtits.xxx";

/// XTits extractor
pub struct XTitsExtractor;

impl XTitsExtractor {
    /// Create a new XTits extractor
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for XTitsExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract categories from the video detail section
fn extract_categories(html: &Html) -> Vec<String> {
    extract_link_texts(html, ".info-block .list-items", "Categories:")
}

/// Extract tags from the video detail section
fn extract_tags(html: &Html) -> Vec<String> {
    extract_link_texts(html, ".info-block .list-items", "Tags:")
}

/// Extract model names from the video detail section
fn extract_models(html: &Html) -> Vec<String> {
    extract_link_texts(html, ".info-block .list-items", "Models:")
}

/// Extract values from `<a>` links inside a labeled `.list-items` section.
///
/// KVS pages use `.list-items` divs with a `.title-row` label followed by `<a>` links.
/// This finds the section whose label matches `label_text` and applies `extract_fn`
/// to each link element.
fn extract_from_labeled_links<F>(
    html: &Html,
    container_sel: &str,
    label_text: &str,
    extract_fn: F,
) -> Vec<String>
where
    F: Fn(&scraper::ElementRef) -> Option<String>,
{
    let container_selector = match scraper::Selector::parse(container_sel) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let title_selector = match scraper::Selector::parse(".title-row") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let link_selector = match scraper::Selector::parse("a.link") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    for container in html.select(&container_selector) {
        if let Some(title_el) = container.select(&title_selector).next() {
            let title = title_el.text().collect::<String>();
            if title.trim() == label_text {
                return container
                    .select(&link_selector)
                    .filter_map(|el| extract_fn(&el))
                    .collect();
            }
        }
    }

    Vec::new()
}

/// Extract link texts from a labeled list-items section.
fn extract_link_texts(html: &Html, container_sel: &str, label_text: &str) -> Vec<String> {
    extract_from_labeled_links(html, container_sel, label_text, |el| {
        let text = el.text().collect::<String>().trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    })
}

/// Extract rating percentage from the voters section
fn extract_rating(html: &Html) -> Option<f64> {
    BaseExtractor::extract_element_text_str(html, ".voters .bold")
        .and_then(|text| text.trim().trim_end_matches('%').parse().ok())
}

/// Extract like/vote count from the voters section.
///
/// The page shows "(241 votes)" next to the rating percentage.
fn extract_like_count(html: &Html) -> Option<u64> {
    let selector = scraper::Selector::parse(".voters").ok()?;
    let text: String = html.select(&selector).next()?.text().collect();
    // Extract number from "(241 votes)" pattern
    let start = text.find('(')? + 1;
    let end = text.find("vote")?;
    text[start..end]
        .trim()
        .replace([' ', ',', '\u{a0}'], "")
        .parse()
        .ok()
}

/// Extract first model's profile URL.
///
/// The "Models:" section contains `<a href="/models/jenni-lee/">Jenni Lee</a>`.
fn extract_model_url(html: &Html) -> Option<String> {
    extract_from_labeled_links(html, ".info-block .list-items", "Models:", |el| {
        el.value()
            .attr("href")
            .map(|href| crate::utils::make_absolute_url(XTITS_BASE_URL, href))
    })
    .into_iter()
    .next()
}

/// Extract text from a `.buttons-info .item` element identified by its icon class.
///
/// KVS info bars use `.item` divs with an icon element (e.g. `.icon-eye`, `.icon-clock`)
/// followed by a text value. This returns the combined text of the matching item.
fn extract_info_bar_text(html: &Html, icon_class: &str) -> Option<String> {
    let item_selector = scraper::Selector::parse(".buttons-info .item").ok()?;
    let icon_selector = scraper::Selector::parse(icon_class).ok()?;

    for item in html.select(&item_selector) {
        if item.select(&icon_selector).next().is_some() {
            return Some(item.text().collect());
        }
    }

    None
}

/// Extract view count from the page
fn extract_view_count(html: &Html) -> Option<u64> {
    extract_info_bar_text(html, ".icon-eye")
        .and_then(|text| text.trim().replace([' ', ',', '\u{a0}'], "").parse().ok())
}

/// Parse duration text like "28min 18sec" into seconds
fn parse_duration_text(html: &Html) -> Option<f64> {
    extract_info_bar_text(html, ".icon-clock")
        .and_then(|text| BaseExtractor::parse_text_duration(text.trim()))
}

#[async_trait]
impl InfoExtractor for XTitsExtractor {
    fn name(&self) -> &str {
        "XTits"
    }

    fn valid_url(&self) -> &Regex {
        &patterns::XTITS_URL_PATTERN
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;

        let video_id = patterns::extract_video_id(url)
            .ok_or_else(|| RdlpError::Extraction(format!("Could not extract video ID: {url}")))?;

        // Extract flashvars block
        let flashvars_content = patterns::FLASHVARS_PATTERN
            .captures(&webpage)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
            .ok_or_else(|| {
                RdlpError::Extraction(format!(
                    "Could not find KVS flashvars on page. Video may be unavailable. URL: {url}"
                ))
            })?;

        // Extract formats from flashvars
        let video_formats = formats::extract_formats(&flashvars_content);

        if video_formats.is_empty() {
            return Err(RdlpError::Extraction(format!(
                "No video formats found in flashvars. URL: {url}"
            )));
        }

        // Parse flashvars for metadata
        let flashvars = formats::parse_flashvars_public(&flashvars_content);

        // Detect file sizes for all formats
        let (formats_with_size, _hls_flags) =
            crate::hls::detect_format_sizes_lazy(video_formats, ctx, self.name()).await;

        // Extract metadata from HTML (drop Html before any await)
        let (
            title,
            description,
            thumbnail,
            categories,
            tags,
            models,
            model_url,
            rating,
            like_count,
            view_count,
            duration,
        ) = {
            let html = Html::parse_document(&webpage);
            let title = BaseExtractor::extract_element_text_str(&html, "h1.title")
                .or_else(|| BaseExtractor::extract_title_multi_strategy(&html))
                .unwrap_or_else(|| "Untitled".to_string());
            let description = BaseExtractor::extract_description_multi_strategy(&html);

            // Thumbnail: prefer flashvars, fall back to og:image
            let thumbnail = formats::extract_thumbnail(&flashvars)
                .or_else(|| BaseExtractor::extract_thumbnail_multi_strategy(&html));

            let categories = extract_categories(&html);
            let tags = extract_tags(&html);
            let models = extract_models(&html);
            let model_url = extract_model_url(&html);
            let rating = extract_rating(&html);
            let like_count = extract_like_count(&html);
            let view_count = extract_view_count(&html);

            // Duration: prefer flashvars, fall back to page text
            let duration =
                formats::extract_duration(&flashvars).or_else(|| parse_duration_text(&html));

            (
                title,
                description,
                thumbnail,
                categories,
                tags,
                models,
                model_url,
                rating,
                like_count,
                view_count,
                duration,
            )
        };

        let mut info = InfoDict::new(video_id, title, self.name(), url);
        info.description = description;
        info.thumbnail = thumbnail;
        info.categories = Some(categories);
        info.tags = Some(tags);
        info.average_rating = rating;
        info.like_count = like_count;
        info.view_count = view_count;
        info.duration = duration;
        info.age_limit = Some(18);
        info.formats = formats_with_size;
        info.propagate_duration();

        // Store first model as uploader (KVS convention)
        if !models.is_empty() {
            info.uploader = Some(models.join(", "));
            info.uploader_url = model_url;
        }

        Ok(info)
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extractor_creation() {
        let extractor = XTitsExtractor::new();
        assert_eq!(extractor.name(), "XTits");
    }

    #[test]
    fn test_suitable_urls() {
        let extractor = XTitsExtractor::new();

        assert!(extractor.suitable(
            "https://www.xtits.xxx/videos/183207/spicy-lesbians-and-straight-girl-smutty-adult-movie/"
        ));
        assert!(extractor.suitable("https://xtits.xxx/videos/12345/test-title/"));
        assert!(extractor.suitable("https://www.xtits.xxx/embed/183207"));

        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://www.pornhub.com/view_video.php?viewkey=ph123"));
    }

    #[test]
    fn test_parse_text_duration() {
        // Tests use BaseExtractor::parse_text_duration now
        assert_eq!(
            BaseExtractor::parse_text_duration("28min 18sec"),
            Some(1698.0)
        );
        assert_eq!(BaseExtractor::parse_text_duration("5min"), Some(300.0));
        assert_eq!(BaseExtractor::parse_text_duration("30sec"), Some(30.0));
        assert_eq!(BaseExtractor::parse_text_duration("1min 1sec"), Some(61.0));
        assert_eq!(BaseExtractor::parse_text_duration(""), None);
    }

    #[test]
    fn test_extract_link_texts_from_html() {
        let html_str = r#"
        <div class="info-block">
            <div class="row list-items">
                <p class="title-row">Categories:</p>
                <a class="link" href="/categories/big-tits/">Big Tits</a>
                <a class="link" href="/categories/brunette/">Brunette</a>
            </div>
            <div class="row list-items">
                <p class="title-row">Tags:</p>
                <a class="link" href="/tags/hardcore/">hardcore</a>
            </div>
            <div class="row list-items">
                <p class="title-row">Models:</p>
                <a class="link" href="/models/jenni-lee/">Jenni Lee</a>
            </div>
        </div>
        "#;

        let html = Html::parse_document(html_str);
        let cats = extract_categories(&html);
        assert_eq!(cats, vec!["Big Tits", "Brunette"]);

        let tags = extract_tags(&html);
        assert_eq!(tags, vec!["hardcore"]);

        let models = extract_models(&html);
        assert_eq!(models, vec!["Jenni Lee"]);
    }

    #[test]
    fn test_extract_rating() {
        let html_str = r#"
        <div class="rating-container">
            <span class="voters">
                <span class="bold">85%</span> (241 votes)
            </span>
        </div>
        "#;
        let html = Html::parse_document(html_str);
        assert_eq!(extract_rating(&html), Some(85.0));
    }

    #[test]
    fn test_extract_like_count() {
        let html_str = r#"
        <div class="rating-container">
            <span class="voters">
                <span class="bold">85%</span> (241 votes)
            </span>
        </div>
        "#;
        let html = Html::parse_document(html_str);
        assert_eq!(extract_like_count(&html), Some(241));
    }

    #[test]
    fn test_extract_model_url() {
        let html_str = r#"
        <div class="info-block">
            <div class="row list-items">
                <p class="title-row">Models:</p>
                <a class="link" href="/models/jenni-lee/">Jenni Lee</a>
            </div>
        </div>
        "#;
        let html = Html::parse_document(html_str);
        assert_eq!(
            extract_model_url(&html),
            Some("https://www.xtits.xxx/models/jenni-lee/".to_string())
        );
    }
}
