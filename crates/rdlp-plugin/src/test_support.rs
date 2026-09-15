//! Test-support seams shared by this crate's unit tests and its
//! `tests/` integration binaries.
//!
//! Compiled into the library (not `cfg(test)`) because an integration
//! test binary links the library as an external crate and cannot see its
//! `cfg(test)` items. Hidden from docs like the `test_*` accessors on
//! `PluginExtractor`, which follow the same precedent.

use std::sync::Arc;

use rdlp_core::ExtractionContext;

/// The default extraction context a plugin test hands to `extract` /
/// `search`: a stock HTTP client, the boa engine, an empty cookie jar and
/// default config. Formerly six identical copies (the adapter unit tests
/// and five integration binaries).
#[doc(hidden)]
#[must_use]
pub fn extraction_ctx() -> ExtractionContext {
    ExtractionContext::new(
        Arc::new(rdlp_http::HttpClientFactory::default().build()),
        Arc::new(rdlp_jsinterp::BoaJsEngine::new()),
        Arc::new(rdlp_cookies::SimpleCookieJar::new()),
        Arc::new(rdlp_types::Config::default()),
    )
}
