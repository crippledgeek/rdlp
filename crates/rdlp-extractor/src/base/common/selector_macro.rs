//! Wrapper macros for `scraper::Selector` that concentrate the runtime panic
//! point on a single `expect(...)` call site, allowing `clippy::expect_used`
//! and `clippy::unwrap_used` to stay enabled crate-wide.
//!
//! ## Why a runtime macro instead of compile-time validation
//!
//! `scraper::Selector::parse` is not `const fn`, and the upstream `selectors`
//! (servo) crate cannot be rewritten in a single PR. Compile-time validation
//! via a proc-macro would require an extra crate in the build graph for a
//! cosmetic gain — the runtime panic at first use is identical to today's
//! behaviour: fast, deterministic, programmer-error-only.
//!
//! Mirrors what `lazy-regex` does at the API surface: even with
//! compile-time validation, the wrapper macro is what eliminates per-site
//! `expect_used` lint noise.
//!
//! ## Usage
//!
//! Item scope (analog of `lazy_regex!`):
//!
//! ```ignore
//! use std::sync::LazyLock;
//! use scraper::Selector;
//! use crate::static_selector;
//!
//! static OG_TITLE: LazyLock<Selector> =
//!     static_selector!(r#"meta[property="og:title"]"#);
//! ```
//!
//! Inside a function body (analog of `regex!`):
//!
//! ```ignore
//! use crate::selector;
//!
//! fn extract(html: &scraper::Html) -> Option<String> {
//!     let s: &scraper::Selector = selector!("p.title a");
//!     html.select(s).next().map(|e| e.text().collect())
//! }
//! ```

use scraper::Selector;

/// Compile a CSS selector at first use. Panics with the literal in the
/// message if the selector string is invalid — same contract as the
/// `Regex::new(LITERAL).expect(...)` pattern that `lazy-regex` replaces.
///
/// Public-but-doc-hidden because the macros invoke this fn; users should
/// always go through `static_selector!()` or `selector!()`.
#[doc(hidden)]
#[allow(clippy::expect_used)]
#[must_use]
pub fn _static_selector_parse(s: &'static str) -> Selector {
    // Hardcoded literal: failure here means a malformed selector in the
    // source, which is a programming error caught at first use.
    Selector::parse(s).expect(s)
}

/// Item-scope wrapper: declare a static `LazyLock<Selector>` from a literal.
///
/// ```ignore
/// static FOO: LazyLock<Selector> = static_selector!("css-selector");
/// ```
///
/// Equivalent to:
///
/// ```ignore
/// static FOO: LazyLock<Selector> =
///     LazyLock::new(|| Selector::parse("css-selector").expect("..."));
/// ```
///
/// but with the `expect()` confined to `_static_selector_parse` so
/// `clippy::expect_used` doesn't fire at every call site.
#[macro_export]
macro_rules! static_selector {
    ($pat:literal) => {
        ::std::sync::LazyLock::new(|| {
            $crate::base::common::selector_macro::_static_selector_parse($pat)
        })
    };
}

/// Function-body-scope wrapper: returns `&'static Selector`.
///
/// Each call site gets its own hidden `static LazyLock<Selector>`, so the
/// CSS selector is parsed exactly once per call site for the program's
/// lifetime — same caching behaviour as a module-level `static`.
///
/// ```ignore
/// fn extract(html: &scraper::Html) -> Option<String> {
///     let s = selector!("p.title a");
///     html.select(s).next().map(|e| e.text().collect())
/// }
/// ```
#[macro_export]
macro_rules! selector {
    ($pat:literal) => {{
        static __S: ::std::sync::LazyLock<::scraper::Selector> =
            $crate::static_selector!($pat);
        &*__S
    }};
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use scraper::Html;
    use std::sync::LazyLock;

    static OG_TITLE: LazyLock<scraper::Selector> =
        crate::static_selector!(r#"meta[property="og:title"]"#);

    #[test]
    fn static_selector_macro_works() {
        let html = Html::parse_document(
            r#"<html><head><meta property="og:title" content="hi"></head></html>"#,
        );
        let el = html.select(&OG_TITLE).next().unwrap();
        assert_eq!(el.value().attr("content"), Some("hi"));
    }

    #[test]
    fn selector_macro_works() {
        let html = Html::parse_document(r#"<html><body><p class="t">x</p></body></html>"#);
        let s: &scraper::Selector = crate::selector!("p.t");
        let el = html.select(s).next().unwrap();
        assert_eq!(el.text().collect::<String>(), "x");
    }
}
