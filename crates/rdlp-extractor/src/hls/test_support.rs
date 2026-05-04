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
    async fn get_cookies(&self, _url: &str) -> Result<Vec<String>> {
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
    let client = wreq::Client::builder()
        .redirect(wreq::redirect::Policy::none())
        .build()
        .expect("client build must succeed in tests");
    let config = Config {
        verbose: false,
        ..Config::default()
    };
    ExtractionContext::new(
        Arc::new(client),
        Arc::new(NoOpJsEngine),
        Arc::new(NoOpCookieJar),
        Arc::new(config),
    )
}

/// Master playlist with two video variants. Used by probe-order regression tests.
pub const MASTER_TWO_VARIANTS: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=1280x720\n\
v720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=640000,RESOLUTION=640x360\n\
v360.m3u8\n";

/// Variant media playlist body — two segments, ENDLIST'd.
pub const VARIANT_MEDIA: &str = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXTINF:6.0,\n\
seg-1.ts\n\
#EXTINF:6.0,\n\
seg-2.ts\n\
#EXT-X-ENDLIST\n";
