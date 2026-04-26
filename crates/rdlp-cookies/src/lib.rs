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
use log::{debug, warn};
use rdlp_core::{CookieJar, Result};
use rdlp_types::BrowserType;
use wreq::cookie::CookieStore;
use std::path::Path;
use std::sync::Arc;
use url::Url;

/// Cookie jar backed by `wreq::cookie::Jar`.
///
/// Cookies added via `add_cookie()` are automatically sent by any
/// `wreq::Client` that was built with this jar's `cookie_provider()`.
pub struct SimpleCookieJar {
    jar: Arc<wreq::cookie::Jar>,
}

impl SimpleCookieJar {
    /// Create a new empty cookie jar.
    #[must_use]
    pub fn new() -> Self {
        Self {
            jar: Arc::new(wreq::cookie::Jar::default()),
        }
    }

    /// Get the underlying `wreq::cookie::Jar` for use with `cookie_provider()`.
    #[must_use]
    pub fn jar(&self) -> Arc<wreq::cookie::Jar> {
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
        // Validate as URL first (early-return on parse failure matches
        // historical reqwest behaviour), then hand the string to wreq
        // whose IntoUri is implemented for &str.
        if Url::parse(url).is_err() {
            return Ok(Vec::new());
        }
        let Ok(uri) = url.parse::<wreq::Uri>() else {
            return Ok(Vec::new());
        };
        let cookies = self.jar.cookies(&uri);
        let headers: Vec<&wreq::header::HeaderValue> = match &cookies {
            wreq::cookie::Cookies::Compressed(hv) => vec![hv],
            wreq::cookie::Cookies::Uncompressed(v) => v.iter().collect(),
            wreq::cookie::Cookies::Empty => return Ok(Vec::new()),
            other => {
                // wreq::cookie::Cookies is #[non_exhaustive]; a future variant
                // would silently drop authenticated requests if we just
                // returned Vec::new(). Surface it so a wreq update is noticed.
                warn!(
                    "Unknown wreq::cookie::Cookies variant encountered ({other:?}); \
                     dropping cookies for this request"
                );
                return Ok(Vec::new());
            }
        };
        let mut out = Vec::new();
        for hv in headers {
            let cookie_str = match hv.to_str() {
                Ok(s) => s,
                Err(e) => {
                    warn!("Cookie header contains non-ASCII bytes: {e}");
                    continue;
                }
            };
            for s in cookie_str.split("; ") {
                if !s.is_empty() {
                    out.push(s.to_string());
                }
            }
        }
        Ok(out)
    }

    async fn add_cookie(&self, url: &str, cookie: &str) -> Result<()> {
        if Url::parse(url).is_err() {
            debug!("Invalid URL for cookie: {url}");
            return Ok(());
        }
        debug!(cookie, url; "Adding cookie");
        self.jar.add(cookie, url);
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
        inner.add("test=value", "https://example.com");

        let cookies = jar.get_cookies("https://example.com").await.unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0], "test=value");
    }
}
