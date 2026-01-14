use async_trait::async_trait;
use regex::Regex;
use std::sync::Arc;

use crate::{Config, InfoDict, Result};

/// Core trait for all site extractors
///
/// Extractors are responsible for parsing website URLs and extracting video metadata.
/// Each extractor typically handles one or more related sites (e.g., YouTube, YouTube Music).
#[async_trait]
pub trait InfoExtractor: Send + Sync {
    /// Human-readable name of the extractor (e.g., "YouTube", "Vimeo")
    fn name(&self) -> &str;

    /// Regex pattern for matching valid URLs this extractor can handle
    ///
    /// This pattern should uniquely identify URLs that this extractor supports.
    /// The registry uses this for routing URLs to the appropriate extractor.
    fn valid_url(&self) -> &Regex;

    /// Extract video information from a URL
    ///
    /// This is the main method that extractors must implement. It should:
    /// 1. Fetch the webpage content
    /// 2. Parse metadata (title, description, uploader, etc.)
    /// 3. Extract available formats (video/audio streams)
    /// 4. Handle any site-specific logic (authentication, decryption, etc.)
    ///
    /// # Arguments
    /// * `url` - The video URL to extract from
    /// * `ctx` - Shared extraction context containing HTTP client, JS engine, etc.
    ///
    /// # Returns
    /// An `InfoDict` containing all extracted metadata and formats
    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict>;

    /// Extract playlist information (optional, returns single video by default)
    ///
    /// Override this method to support playlist/channel/search extraction.
    /// The default implementation treats all URLs as single videos.
    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        Ok(vec![self.extract(url, ctx).await?])
    }

    /// Check if this extractor can handle the given URL
    ///
    /// The default implementation checks against the valid_url() regex.
    /// Override this for more complex URL validation logic.
    fn suitable(&self, url: &str) -> bool {
        self.valid_url().is_match(url)
    }

    /// Priority for this extractor (higher = preferred when multiple match)
    ///
    /// When multiple extractors match a URL, the one with the highest priority
    /// is chosen. The default priority is 0.
    ///
    /// Use higher priorities for more specific extractors and lower priorities
    /// for generic/fallback extractors.
    fn priority(&self) -> i32 {
        0
    }
}

/// Context passed to extractors containing shared resources
///
/// This provides access to shared services that extractors need, such as
/// HTTP clients, JavaScript engines, cookie jars, and configuration.
pub struct ExtractionContext {
    /// HTTP client for making requests
    pub http_client: Arc<reqwest::Client>,

    /// JavaScript engine for executing site JavaScript (e.g., signature decryption)
    pub js_engine: Arc<dyn JsEngine>,

    /// Cookie jar for authentication
    pub cookie_jar: Arc<dyn CookieJar>,

    /// Application configuration
    pub config: Arc<Config>,
}

impl ExtractionContext {
    /// Create a new extraction context
    pub fn new(
        http_client: Arc<reqwest::Client>,
        js_engine: Arc<dyn JsEngine>,
        cookie_jar: Arc<dyn CookieJar>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            http_client,
            js_engine,
            cookie_jar,
            config,
        }
    }
}

/// JavaScript execution engine trait
///
/// Provides an abstraction over JavaScript engines (boa, V8, etc.) for
/// executing site-specific JavaScript code.
#[async_trait]
pub trait JsEngine: Send + Sync {
    /// Evaluate JavaScript code and return result as JSON
    ///
    /// # Arguments
    /// * `code` - JavaScript code to execute
    ///
    /// # Returns
    /// The result of execution as a JSON value
    async fn eval(&self, code: &str) -> Result<serde_json::Value>;

    /// Evaluate JavaScript with context variables
    ///
    /// # Arguments
    /// * `code` - JavaScript code to execute
    /// * `context` - Context variables to make available to the JavaScript
    ///
    /// # Returns
    /// The result of execution as a JSON value
    async fn eval_with_context(
        &self,
        code: &str,
        context: &serde_json::Value,
    ) -> Result<serde_json::Value>;

    /// Call a JavaScript function by name
    ///
    /// # Arguments
    /// * `function_name` - Name of the function to call
    /// * `args` - Arguments to pass to the function
    ///
    /// # Returns
    /// The function's return value as JSON
    async fn call_function(
        &self,
        function_name: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value>;
}

/// Cookie storage and management trait
///
/// Provides access to cookies for authenticated requests.
#[async_trait]
pub trait CookieJar: Send + Sync {
    /// Get cookies for a given URL
    ///
    /// # Arguments
    /// * `url` - The URL to get cookies for
    ///
    /// # Returns
    /// A vector of cookie strings in the format "name=value"
    async fn get_cookies(&self, url: &str) -> Result<Vec<String>>;

    /// Add a cookie
    ///
    /// # Arguments
    /// * `url` - The URL this cookie applies to
    /// * `cookie` - Cookie string in the format "name=value; Domain=...; ..."
    async fn add_cookie(&self, url: &str, cookie: &str) -> Result<()>;

    /// Load cookies from a browser
    ///
    /// # Arguments
    /// * `browser` - Browser name ("chrome", "firefox", "safari", etc.)
    ///
    /// # Returns
    /// Number of cookies loaded
    async fn load_from_browser(&self, browser: &str) -> Result<usize>;
}
