//! HTML parser for PornHub's `/video/search` results page.
//!
//! Used as the **primary** path by `PornHubExtractor::search_page` — the
//! Webmaster JSON API does not carry uploader information, so the HTML
//! parse is what populates `SearchResultPreview.uploader`.

use lazy_regex::regex;
use rdlp_core::Result;
use rdlp_types::SearchResultPreview;
use scraper::{ElementRef, Html};

const SITE_BASE: &str = "https://www.pornhub.com";

/// Parse a PornHub HTML search-results page body.
///
/// Returns a vector of `SearchResultPreview`. An empty vector is a valid
/// result (zero matches on the page); only outright DOM-parse failure
/// returns `Err`.
pub(crate) fn parse_html_search_results(body: &str) -> Result<Vec<SearchResultPreview>> {
    let doc = Html::parse_document(body);
    let card_sel = crate::selector!("li.pcVideoListItem");

    let mut results = Vec::new();
    for card in doc.select(card_sel) {
        if let Some(preview) = parse_card(&card) {
            results.push(preview);
        }
    }

    Ok(results)
}

fn parse_card(card: &ElementRef<'_>) -> Option<SearchResultPreview> {
    let title_sel = crate::selector!("span.title a");
    let title_el = card.select(title_sel).next()?;
    let title = title_el.text().collect::<String>().trim().to_string();
    if title.is_empty() {
        return None;
    }

    let href = title_el.value().attr("href")?;
    if !href.contains("view_video.php?viewkey=") {
        return None;
    }
    let video_url = if href.starts_with("http") {
        href.to_string()
    } else {
        format!("{SITE_BASE}{href}")
    };

    let (uploader, uploader_url) = parse_uploader(card);
    let duration = parse_duration(card);
    let view_count = parse_view_count(card);
    let thumbnail_url = parse_thumbnail(card);

    Some(SearchResultPreview {
        video_url,
        title,
        thumbnail_url,
        duration,
        uploader,
        uploader_url,
        actors: Vec::new(),
        view_count,
        upload_date: None,
    })
}

/// Returns `(display_name, absolute_url)` for the uploader anchor, if present.
///
/// The same anchor element provides both — the text node is the display name
/// and the `href` attribute is the channel/model/pornstar path.
fn parse_uploader(card: &ElementRef<'_>) -> (Option<String>, Option<String>) {
    // Primary: badged link (~5% of cards observed live).
    let strict = crate::selector!(r#"div.usernameWrap span.usernameBadgesWrapper a"#);
    if let Some(el) = card.select(strict).next() {
        let name = el.text().collect::<String>().trim().to_string();
        if !name.is_empty() {
            let url = el.value().attr("href").map(|href| {
                if href.starts_with("http") {
                    href.to_string()
                } else {
                    format!("{SITE_BASE}{href}")
                }
            });
            return (Some(name), url);
        }
    }
    // Fallback: any link in the username wrapper (~95% coverage live).
    let loose = crate::selector!("div.usernameWrap a");
    if let Some(el) = card.select(loose).next() {
        let name = el.text().collect::<String>().trim().to_string();
        if !name.is_empty() {
            let url = el.value().attr("href").map(|href| {
                if href.starts_with("http") {
                    href.to_string()
                } else {
                    format!("{SITE_BASE}{href}")
                }
            });
            return (Some(name), url);
        }
    }
    (None, None)
}

fn parse_duration(card: &ElementRef<'_>) -> Option<f64> {
    let sel = crate::selector!("var.duration");
    let text = card.select(sel).next()?.text().collect::<String>();
    let trimmed = text.trim();
    let mut secs = 0_u64;
    for part in trimmed.split(':') {
        let n: u64 = part.parse().ok()?;
        secs = secs * 60 + n;
    }
    Some(secs as f64)
}

fn parse_view_count(card: &ElementRef<'_>) -> Option<u64> {
    let sel = crate::selector!(".views var");
    let text = card.select(sel).next()?.text().collect::<String>();
    parse_view_count_text(&text)
}

/// Parse "844K", "844K views", "3.7M", "1.2B".
pub(crate) fn parse_view_count_text(s: &str) -> Option<u64> {
    let re = regex!(r"(?i)([0-9]+(?:\.[0-9]+)?)\s*([KMB]?)");
    let caps = re.captures(s)?;
    let n: f64 = caps.get(1)?.as_str().parse().ok()?;
    let mult: f64 = match caps.get(2)?.as_str().to_ascii_uppercase().as_str() {
        "K" => 1_000.0,
        "M" => 1_000_000.0,
        "B" => 1_000_000_000.0,
        _ => 1.0,
    };
    let total = (n * mult).round();
    if total < 0.0 {
        None
    } else {
        Some(total as u64)
    }
}

fn parse_thumbnail(card: &ElementRef<'_>) -> Option<String> {
    let sel = crate::selector!("img");
    let img = card.select(sel).next()?;
    img.value()
        .attr("src")
        .or_else(|| img.value().attr("data-mediumthumb"))
        .or_else(|| img.value().attr("data-image"))
        .map(ToString::to_string)
}

#[cfg(test)]
mod unit_tests {
    use super::parse_view_count_text;

    #[test]
    fn parse_views_k_suffix() {
        assert_eq!(parse_view_count_text("844K views"), Some(844_000));
    }

    #[test]
    fn parse_views_m_suffix() {
        assert_eq!(parse_view_count_text("3.7M"), Some(3_700_000));
    }

    #[test]
    fn parse_views_no_suffix() {
        assert_eq!(parse_view_count_text("12345"), Some(12_345));
    }

    #[test]
    fn parse_views_garbage_returns_none() {
        assert_eq!(parse_view_count_text("???"), None);
    }
}
