//! # rdlp-cookies
//!
//! Browser cookie extraction for rdlp.
//!
//! This crate provides cookie extraction from various browsers:
//! - Chrome/Chromium
//! - Firefox
//! - Safari (macOS)

use async_trait::async_trait;
use rdlp_core::{CookieJar, Result};

/// Simple cookie jar implementation (stub for now)
pub struct SimpleCookieJar {
    // Will be implemented in Phase 8
}

impl SimpleCookieJar {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for SimpleCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CookieJar for SimpleCookieJar {
    async fn get_cookies(&self, _url: &str) -> Result<Vec<String>> {
        // Stub implementation - returns empty cookies
        Ok(Vec::new())
    }

    async fn add_cookie(&self, _url: &str, _cookie: &str) -> Result<()> {
        // Stub implementation
        Ok(())
    }

    async fn load_from_browser(&self, _browser: &str) -> Result<usize> {
        // Stub implementation - returns 0 cookies loaded
        Ok(0)
    }
}
