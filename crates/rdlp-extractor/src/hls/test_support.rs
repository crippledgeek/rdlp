//! `#[cfg(test)]`-only helpers shared by HLS regression tests.
//!
//! Not part of the crate's public API. Imported as `crate::hls::test_support::*`
//! from unit tests within `rdlp-extractor`.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use rdlp_core::{CookieJar, ExtractionContext, JsEngine, Result};
use rdlp_types::{BrowserType, Config};

/// JS engine stub returning `null` for every call.
pub struct NoOpJsEngine;

#[async_trait]
impl JsEngine for NoOpJsEngine {
    async fn eval(&self, _code: &str) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn eval_with_context(
        &self,
        _code: &str,
        _context: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
    async fn call_function(
        &self,
        _name: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

/// Cookie jar stub.
pub struct NoOpCookieJar;

#[async_trait]
impl CookieJar for NoOpCookieJar {
    async fn cookies(&self, _url: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn add_cookie(&self, _url: &str, _cookie: &str) -> Result<()> {
        Ok(())
    }
    async fn load_from_browser(&self, _browser: BrowserType) -> Result<usize> {
        Ok(0)
    }
    async fn load_from_file(&self, _path: &Path) -> Result<usize> {
        Ok(0)
    }
}

/// Build an `ExtractionContext` for mockito-backed unit tests.
pub fn test_ctx() -> ExtractionContext {
    test_ctx_with(Config {
        verbose: false,
        ..Config::default()
    })
}

/// [`test_ctx`] with a caller-supplied `Config`, for tests that exercise a
/// config-driven code path (the playlist loop's range, concurrency, timeout
/// and failure-policy fields).
pub fn test_ctx_with(config: Config) -> ExtractionContext {
    let client = wreq::Client::builder()
        .redirect(wreq::redirect::Policy::none())
        .build()
        .expect("client build must succeed in tests");
    ExtractionContext::new(
        Arc::new(client),
        Arc::new(NoOpJsEngine),
        Arc::new(NoOpCookieJar),
        Arc::new(config),
    )
}

// The two playlist fixtures live in `test_fixtures` (also reachable from
// `rdlp-plugin` via the `loopback-test-exemption` feature) and are
// re-exported here so every existing `test_support::{MASTER_TWO_VARIANTS,
// VARIANT_MEDIA}` import in this crate keeps working unchanged.
pub use super::test_fixtures::{MASTER_TWO_VARIANTS, VARIANT_MEDIA};
