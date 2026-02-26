//! # rdlp-cookies
//!
//! Browser cookie extraction for rdlp.
//!
//! This crate provides cookie storage and extraction from various browsers:
//! - Chrome/Chromium
//! - Firefox
//! - Safari (macOS)

#![warn(missing_docs)]

mod chrome;
mod firefox;
mod netscape;
mod util;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{BrowserType, CookieJar, Result};
use reqwest::cookie::CookieStore;
use std::path::Path;
use std::sync::Arc;
use url::Url;

/// Cookie jar backed by `reqwest::cookie::Jar`.
///
/// Cookies added via `add_cookie()` are automatically sent by any
/// `reqwest::Client` that was built with this jar's `cookie_provider()`.
pub struct SimpleCookieJar {
    jar: Arc<reqwest::cookie::Jar>,
}

impl SimpleCookieJar {
    /// Create a new empty cookie jar.
    #[must_use]
    pub fn new() -> Self {
        Self {
            jar: Arc::new(reqwest::cookie::Jar::default()),
        }
    }

    /// Get the underlying `reqwest::cookie::Jar` for use with `cookie_provider()`.
    #[must_use]
    pub fn jar(&self) -> Arc<reqwest::cookie::Jar> {
        Arc::clone(&self.jar)
    }
}

impl Default for SimpleCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CookieJar for SimpleCookieJar {
    async fn get_cookies(&self, url: &str) -> Result<Vec<String>> {
        let Ok(parsed) = Url::parse(url) else {
            return Ok(Vec::new());
        };
        let Some(header_value) = self.jar.cookies(&parsed) else {
            return Ok(Vec::new());
        };
        let cookie_str = header_value.to_str().unwrap_or("");
        Ok(cookie_str
            .split("; ")
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    async fn add_cookie(&self, url: &str, cookie: &str) -> Result<()> {
        let parsed = match Url::parse(url) {
            Ok(u) => u,
            Err(e) => {
                debug!("Invalid URL for cookie: {e}");
                return Ok(());
            }
        };

        debug!(cookie, url; "Adding cookie");
        self.jar.add_cookie_str(cookie, &parsed);
        Ok(())
    }

    async fn load_from_browser(&self, browser: BrowserType) -> Result<usize> {
        let jar = Arc::clone(&self.jar);

        let count = tokio::task::spawn_blocking(move || match browser {
            BrowserType::Chrome => chrome::extract_cookies(&*jar),
            BrowserType::Firefox => firefox::extract_cookies(&*jar),
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))??;

        debug!("Loaded {count} cookies from browser: {browser}");
        Ok(count)
    }

    async fn load_from_file(&self, path: &Path) -> Result<usize> {
        let jar = Arc::clone(&self.jar);
        let path = path.to_path_buf();
        let count = tokio::task::spawn_blocking(move || netscape::load_cookie_file(&path, &*jar))
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))??;

        debug!(count; "Loaded cookies from file");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_get_cookie() {
        let jar = SimpleCookieJar::new();
        jar.add_cookie("https://example.com", "session=abc123")
            .await
            .unwrap();

        let cookies = jar.get_cookies("https://example.com").await.unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0], "session=abc123");
    }

    #[tokio::test]
    async fn test_multiple_cookies() {
        let jar = SimpleCookieJar::new();
        jar.add_cookie("https://example.com", "a=1").await.unwrap();
        jar.add_cookie("https://example.com", "b=2").await.unwrap();

        let cookies = jar.get_cookies("https://example.com").await.unwrap();
        assert_eq!(cookies.len(), 2);
        assert!(cookies.contains(&"a=1".to_string()));
        assert!(cookies.contains(&"b=2".to_string()));
    }

    #[tokio::test]
    async fn test_cookies_scoped_by_domain() {
        let jar = SimpleCookieJar::new();
        jar.add_cookie("https://example.com", "a=1").await.unwrap();
        jar.add_cookie("https://other.com", "b=2").await.unwrap();

        let cookies = jar.get_cookies("https://example.com").await.unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0], "a=1");
    }

    #[tokio::test]
    async fn test_empty_jar() {
        let jar = SimpleCookieJar::new();
        let cookies = jar.get_cookies("https://example.com").await.unwrap();
        assert!(cookies.is_empty());
    }

    #[tokio::test]
    async fn test_invalid_url() {
        let jar = SimpleCookieJar::new();
        // Should not panic, just return empty
        let cookies = jar.get_cookies("not-a-url").await.unwrap();
        assert!(cookies.is_empty());
    }

    #[tokio::test]
    async fn test_jar_accessor() {
        let jar = SimpleCookieJar::new();
        let inner = jar.jar();
        // Verify it's the same jar by adding via inner and reading via trait
        let url = Url::parse("https://example.com").unwrap();
        inner.add_cookie_str("test=value", &url);

        let cookies = jar.get_cookies("https://example.com").await.unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0], "test=value");
    }
}
