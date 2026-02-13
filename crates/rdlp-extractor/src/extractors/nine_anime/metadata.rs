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
fn extract_title(html: &Html, _webpage: &str) -> String {
    // Strategy 1: h2 element — 9anime puts the clean title here
    for selector_str in &["h2.film-name", "h2"] {
        if let Ok(sel) = Selector::parse(selector_str) {
            if let Some(elem) = html.select(&sel).next() {
                let text: String = elem.text().collect();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }

    // Strategy 2: OG/Twitter/title tag with suffix cleaning
    if let Some(title) = BaseExtractor::extract_title_multi_strategy(html) {
        let cleaned = clean_9anime_title(&title);
        if !cleaned.is_empty() {
            return cleaned;
        }
    }

    // Strategy 3: breadcrumb — "Watching Sword Art Online"
    if let Ok(sel) = Selector::parse("ol li:last-child") {
        if let Some(elem) = html.select(&sel).next() {
            let text: String = elem.text().collect();
            let trimmed = text.trim().trim_start_matches("Watching ").trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    "Unknown Anime".to_string()
}

/// Strip 9anime boilerplate from a page title.
fn clean_9anime_title(title: &str) -> String {
    let t = title
        .trim_end_matches(" - 9anime")
        .trim_end_matches(" | 9anime")
        .trim();

    // "Watch Sword Art Online online free on 9anime"
    let t = t.strip_prefix("Watch ").unwrap_or(t);
    let t = t
        .strip_suffix(" online free on 9anime")
        .or_else(|| t.strip_suffix(" on 9anime"))
        .unwrap_or(t);

    t.trim().to_string()
}

/// Extract thumbnail URL from the page.
fn extract_thumbnail(html: &Html) -> Option<String> {
    // Try OG image first, then fall back to poster image
    BaseExtractor::extract_thumbnail_multi_strategy(html).or_else(|| {
        let img_selector = Selector::parse("img.film-poster-img").ok()?;
        html.select(&img_selector)
            .next()
            .and_then(|e| e.value().attr("src").map(String::from))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_title_h2_preferred() {
        // h2 should be preferred over og:title when present
        let html_str = r#"<html><head>
            <title>Watch Sword Art Online online free on 9anime</title>
        </head><body>
            <h2>Sword Art Online</h2>
        </body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Sword Art Online");
    }

    #[test]
    fn test_extract_title_h2_film_name() {
        let html_str = r#"<html><head></head><body>
            <h2 class="film-name">Attack on Titan</h2>
        </body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Attack on Titan");
    }

    #[test]
    fn test_extract_title_fallback_og_cleaned() {
        // No h2 → falls back to og:title with cleaning
        let html_str = r#"<html><head>
            <title>Watch Sword Art Online online free on 9anime</title>
        </head><body></body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Sword Art Online");
    }

    #[test]
    fn test_extract_title_suffix_dash() {
        let html_str = r#"<html><head>
            <meta property="og:title" content="Sword Art Online - 9anime">
        </head><body></body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Sword Art Online");
    }

    #[test]
    fn test_extract_title_breadcrumb() {
        let html_str = r#"<html><head></head><body>
            <ol><li>Home</li><li>Watching Sword Art Online</li></ol>
        </body></html>"#;
        let html = Html::parse_document(html_str);
        let title = extract_title(&html, html_str);
        assert_eq!(title, "Sword Art Online");
    }

    #[test]
    fn test_clean_9anime_title() {
        assert_eq!(
            clean_9anime_title("Watch Sword Art Online online free on 9anime"),
            "Sword Art Online"
        );
        assert_eq!(
            clean_9anime_title("Sword Art Online - 9anime"),
            "Sword Art Online"
        );
        assert_eq!(
            clean_9anime_title("Sword Art Online | 9anime"),
            "Sword Art Online"
        );
        assert_eq!(clean_9anime_title("Watch One Piece on 9anime"), "One Piece");
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
