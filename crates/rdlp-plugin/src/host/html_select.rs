//! `host:html-select` capability — CSS-selector text + attribute extraction
//! via the `scraper` crate.
//!
//! Pure parsing, no I/O — never blocks, never errors. Invalid selectors
//! return an empty list rather than failing, matching browser-extension
//! semantics where authors are tolerant of malformed inputs.

use crate::instance::PluginStoreData;
use wasmtime::component::Linker;

/// Per-plugin context. Currently no state — included for symmetry with
/// other host capabilities and to allow future extension (e.g. per-plugin
/// selector caching).
#[derive(Debug, Default, Clone)]
pub struct HtmlSelectCtx;

impl HtmlSelectCtx {
    /// Return the concatenated text content of every element matched by
    /// `css_selector`. Invalid selectors yield an empty list.
    #[must_use]
    pub fn select_text(html: &str, css_selector: &str) -> Vec<String> {
        let Ok(selector) = scraper::Selector::parse(css_selector) else {
            return Vec::new();
        };
        scraper::Html::parse_document(html)
            .select(&selector)
            .map(|el| el.text().collect::<String>())
            .collect()
    }

    /// Return the value of `attr` on every element matched by `css_selector`.
    /// Elements without the attribute are silently skipped.
    #[must_use]
    pub fn select_attr_values(html: &str, css_selector: &str, attr: &str) -> Vec<String> {
        let Ok(selector) = scraper::Selector::parse(css_selector) else {
            return Vec::new();
        };
        scraper::Html::parse_document(html)
            .select(&selector)
            .filter_map(|el| el.value().attr(attr).map(str::to_string))
            .collect()
    }
}

/// Wire `host:html-select` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_html_select::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_html_select::Host for PluginStoreData {
    async fn select(&mut self, html: String, css_selector: String) -> Vec<String> {
        HtmlSelectCtx::select_text(&html, &css_selector)
    }

    async fn select_attr(
        &mut self,
        html: String,
        css_selector: String,
        attr: String,
    ) -> Vec<String> {
        HtmlSelectCtx::select_attr_values(&html, &css_selector, &attr)
    }
}
