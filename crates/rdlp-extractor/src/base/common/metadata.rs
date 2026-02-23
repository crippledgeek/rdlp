//! HTML metadata extraction utilities
//!
//! Multi-strategy extraction of titles, descriptions, thumbnails, and
//! other metadata from HTML pages using Open Graph, Twitter cards,
//! JSON-LD, and standard HTML elements.

use scraper::{Html, Selector};

use super::BaseExtractor;
use super::selectors::{
    H1_SELECTOR, MAX_DESCRIPTION_LENGTH, MAX_TITLE_LENGTH, META_DESCRIPTION_SELECTOR,
    OG_DESCRIPTION_SELECTOR, OG_IMAGE_SELECTOR, OG_TITLE_SELECTOR, TITLE_TAG_SELECTOR,
    TWITTER_IMAGE_SELECTOR, TWITTER_TITLE_SELECTOR,
};

impl BaseExtractor {
    // ========================================================================
    // Metadata Extraction
    // ========================================================================

    /// Extract content from a meta tag
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the meta tag
    ///
    /// # Returns
    /// The content attribute value if found and non-empty
    #[must_use]
    pub fn extract_meta_content(html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .and_then(|elem| elem.value().attr("content"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract href from a link element
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the link element
    ///
    /// # Returns
    /// The href attribute value if found and non-empty
    #[must_use]
    pub fn extract_link_href(html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .and_then(|elem| elem.value().attr("href"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract text content from an element
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    /// * `selector` - CSS selector for the element
    ///
    /// # Returns
    /// The text content if found and non-empty
    #[must_use]
    pub fn extract_element_text(html: &Html, selector: &Selector) -> Option<String> {
        html.select(selector)
            .next()
            .map(|elem| elem.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Extract content from a meta tag using a CSS selector string.
    ///
    /// Convenience wrapper around [`Self::extract_meta_content`] that parses
    /// the selector from a string. Use this for site-specific selectors that
    /// are not worth pre-compiling as statics.
    #[must_use]
    pub fn extract_meta_content_str(html: &Html, selector_str: &str) -> Option<String> {
        let selector = Selector::parse(selector_str).ok()?;
        Self::extract_meta_content(html, &selector)
    }

    /// Extract text content from an element using a CSS selector string.
    ///
    /// Convenience wrapper around [`Self::extract_element_text`] that parses
    /// the selector from a string. Use this for site-specific selectors that
    /// are not worth pre-compiling as statics.
    #[must_use]
    pub fn extract_element_text_str(html: &Html, selector_str: &str) -> Option<String> {
        let selector = Selector::parse(selector_str).ok()?;
        Self::extract_element_text(html, &selector)
    }

    /// Extract the first href from a list of CSS selector strings.
    ///
    /// Tries each selector in order, returning the first non-empty `href`
    /// attribute found. Relative hrefs starting with `/` are made absolute
    /// using the provided `base_url`.
    #[must_use]
    pub fn extract_first_href(html: &Html, selectors: &[&str], base_url: &str) -> Option<String> {
        for selector_str in selectors {
            let Ok(selector) = Selector::parse(selector_str) else {
                continue;
            };
            if let Some(element) = html.select(&selector).next() {
                if let Some(href) = element.value().attr("href") {
                    if !href.is_empty() {
                        if href.starts_with('/') {
                            return Some(format!("{base_url}{href}"));
                        }
                        return Some(href.to_string());
                    }
                }
            }
        }
        None
    }

    /// Extract title using multiple fallback strategies
    ///
    /// Tries in order:
    /// 1. Open Graph title (`og:title`)
    /// 2. Twitter title (`twitter:title`)
    /// 3. HTML title tag
    /// 4. H1 heading
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Title from the first successful strategy, `None` if all fail
    #[must_use]
    pub fn extract_title_multi_strategy(html: &Html) -> Option<String> {
        // Strategy 1: Open Graph
        if let Some(title) = Self::extract_meta_content(html, &OG_TITLE_SELECTOR) {
            return Some(Self::truncate_string(title, MAX_TITLE_LENGTH));
        }

        // Strategy 2: Twitter
        if let Some(title) = Self::extract_meta_content(html, &TWITTER_TITLE_SELECTOR) {
            return Some(Self::truncate_string(title, MAX_TITLE_LENGTH));
        }

        // Strategy 3: HTML title tag
        if let Some(title) = Self::extract_element_text(html, &TITLE_TAG_SELECTOR) {
            return Some(Self::truncate_string(title, MAX_TITLE_LENGTH));
        }

        // Strategy 4: H1
        Self::extract_element_text(html, &H1_SELECTOR)
            .map(|t| Self::truncate_string(t, MAX_TITLE_LENGTH))
    }

    /// Extract description using multiple fallback strategies
    ///
    /// Tries in order:
    /// 1. Open Graph description
    /// 2. Meta description
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Description from the first successful strategy, `None` if all fail
    #[must_use]
    pub fn extract_description_multi_strategy(html: &Html) -> Option<String> {
        // Strategy 1: Open Graph
        if let Some(desc) = Self::extract_meta_content(html, &OG_DESCRIPTION_SELECTOR) {
            return Some(Self::truncate_string(desc, MAX_DESCRIPTION_LENGTH));
        }

        // Strategy 2: Meta description
        Self::extract_meta_content(html, &META_DESCRIPTION_SELECTOR)
            .map(|d| Self::truncate_string(d, MAX_DESCRIPTION_LENGTH))
    }

    /// Extract thumbnail URL using multiple fallback strategies
    ///
    /// Tries in order:
    /// 1. Open Graph image
    /// 2. Twitter image
    ///
    /// # Arguments
    /// * `html` - Parsed HTML document
    ///
    /// # Returns
    /// Thumbnail URL from the first successful strategy, `None` if all fail
    #[must_use]
    pub fn extract_thumbnail_multi_strategy(html: &Html) -> Option<String> {
        // Strategy 1: Open Graph
        if let Some(thumb) = Self::extract_meta_content(html, &OG_IMAGE_SELECTOR) {
            return Some(thumb);
        }

        // Strategy 2: Twitter
        Self::extract_meta_content(html, &TWITTER_IMAGE_SELECTOR)
    }
}
