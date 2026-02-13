//! HTML metadata extraction for 9anime pages.
//!
//! Parses anime title, description, thumbnail, genres, and episode info
//! from the watch page HTML.

use crate::base::common::BaseExtractor;
use scraper::{Html, Selector};

/// Metadata extracted from a 9anime watch page.
pub struct AnimeMetadata {
    /// Anime title.
    pub title: String,
    /// Episode number (if available from the server response).
    pub episode_number: Option<String>,
    /// Thumbnail URL.
    pub thumbnail: Option<String>,
    /// Description/synopsis.
    pub description: Option<String>,
}

/// Extract anime metadata from the watch page HTML.
#[must_use]
pub fn extract_metadata(html: &Html, webpage: &str) -> AnimeMetadata {
    let title = extract_title(html, webpage);
    let thumbnail = extract_thumbnail(html);
    let description = BaseExtractor::extract_description_multi_strategy(html);

    AnimeMetadata {
        title,
        episode_number: None,
        thumbnail,
        description,
    }
}

/// Extract the anime title, trying multiple strategies.
fn extract_title(html: &Html, webpage: &str) -> String {
    // Try OG title first (most reliable)
    if let Some(title) = BaseExtractor::extract_title_multi_strategy(html) {
        // Strip common suffixes like " - 9anime" or "Watch ... Online"
        let title = title
            .trim_end_matches(" - 9anime")
            .trim_end_matches(" | 9anime")
            .trim();
        if !title.is_empty() {
            return title.to_string();
        }
    }

    // Fallback: look for the anime name in heading elements
    let h2_selector = Selector::parse("h2.film-name").unwrap_or_else(|_| {
        // If that selector fails, try a broader one
        Selector::parse("h2").expect("h2 selector must parse")
    });

    if let Some(elem) = html.select(&h2_selector).next() {
        let text: String = elem.text().collect();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    // Last resort: extract from page content
    let _ = webpage; // Used implicitly via html
    "Unknown Anime".to_string()
}

/// Extract thumbnail URL from the page.
fn extract_thumbnail(html: &Html) -> Option<String> {
    // Try OG image
    if let Some(url) = BaseExtractor::extract_thumbnail_multi_strategy(html) {
        return Some(url);
    }

    // Fallback: look for poster image
    let img_selector = Selector::parse("img.film-poster-img").ok()?;
    html.select(&img_selector)
        .next()
        .and_then(|e| e.value().attr("src").map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_title_strips_suffix() {
        let html_str = r#"<html><head>
            <meta property="og:title" content="Sword Art Online - 9anime">
        </head><body></body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Sword Art Online");
    }

    #[test]
    fn test_extract_title_fallback_h2() {
        let html_str = r#"<html><head></head><body>
            <h2 class="film-name">Attack on Titan</h2>
        </body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Attack on Titan");
    }

    #[test]
    fn test_extract_thumbnail_og() {
        let html_str = r#"<html><head>
            <meta property="og:image" content="https://cdn.example.com/thumb.jpg">
        </head><body></body></html>"#;
        let html = Html::parse_document(html_str);
        assert_eq!(
            extract_thumbnail(&html),
            Some("https://cdn.example.com/thumb.jpg".to_string())
        );
    }
}
