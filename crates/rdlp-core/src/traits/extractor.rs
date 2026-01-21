use async_trait::async_trait;
use regex::Regex;
use std::sync::Arc;

use crate::{Config, InfoDict, Result};

/// Core trait for all site extractors
///
/// Extractors are responsible for parsing website URLs and extracting video metadata.
/// Each extractor typically handles one or more related sites (e.g., YouTube, YouTube Music).
///
/// # Lifetime Semantics
///
/// Most trait methods use **lifetime elision** (no explicit lifetime annotations needed):
/// - `fn name(&self) -> &str` - Compiler infers: `fn name<'a>(&'a self) -> &'a str`
/// - The returned reference has the same lifetime as the `&self` parameter
/// - This allows zero-cost borrowing of extractor data without clones
///
/// Methods returning owned data (`String`, `InfoDict`) enable transfer across async boundaries,
/// which is required for `Send + Sync` trait bounds in async contexts.
#[async_trait]
pub trait InfoExtractor: Send + Sync {
    /// Human-readable name of the extractor (e.g., "YouTube", "Vimeo")
    ///
    /// **Lifetime:** Returns `&str` with lifetime tied to `&self` (elided: `<'a>`).
    /// The string is borrowed from the extractor struct, avoiding allocation.
    fn name(&self) -> &str;

    /// Regex pattern for matching valid URLs this extractor can handle
    ///
    /// This pattern should uniquely identify URLs that this extractor supports.
    /// The registry uses this for routing URLs to the appropriate extractor.
    ///
    /// **Lifetime:** Returns `&Regex` with lifetime tied to `&self` (elided: `<'a>`).
    /// For optimal performance, extractors should use static lazy regexes (`&'static Regex`),
    /// which avoids regex compilation overhead on every constructor call.
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
    ///
    /// # Async Ownership Pattern
    ///
    /// This method returns **owned** `InfoDict` (not `&InfoDict`) because:
    /// - Async functions must return `Send` types that can cross thread boundaries
    /// - Borrowed data (&str) cannot be held across `.await` points
    /// - InfoDict must outlive the HTML parsing scope and async operations
    ///
    /// **Common Pattern:**
    /// ```rust,ignore
    /// async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
    ///     // Extract data from HTML synchronously, allocating strings
    ///     let (title, description) = {
    ///         let html = fetch_html(url).await?;
    ///         // Extract to owned Strings before html is dropped
    ///         (extract_title(&html).to_string(), extract_desc(&html).to_string())
    ///     }; // html dropped here, but strings are owned and can be used below
    ///
    ///     // Build InfoDict with owned data
    ///     Ok(InfoDict::new(id, title, extractor_name, url.to_string()))
    /// }
    /// ```
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
///
/// # Memory Management with Arc
///
/// All fields use `Arc<T>` (Atomic Reference Counting) for shared ownership:
/// - **Why Arc?** Enables cheap cloning for async tasks and parallel operations
/// - **Arc vs Box:** Arc allows multiple owners; Box has single ownership
/// - **Arc vs Rc:** Arc is thread-safe (Send + Sync); Rc is single-threaded only
///
/// **Cloning Cost:** `Arc::clone()` only increments a reference counter (~5ns),
/// much cheaper than deep-copying the underlying data.
///
/// ## Arc vs Box Trade-offs
///
/// | Criterion | Arc<T> | Box<T> |
/// |-----------|--------|--------|
/// | **Ownership** | Multiple owners (shared) | Single owner (unique) |
/// | **Thread Safety** | Yes (Send + Sync) | Depends on T |
/// | **Clone Cost** | ~5ns (atomic increment) | Deep copy entire T |
/// | **Memory Overhead** | 16 bytes (refcount + weak count) | 0 bytes |
/// | **Deref Cost** | 1 pointer indirection | 1 pointer indirection |
/// | **Use Case** | Shared services, parallel tasks | Unique ownership, trait objects |
///
/// **When to use Arc:**
/// - Sharing data across async tasks (tokio::spawn)
/// - Sharing clients/connections (HTTP, database)
/// - Caching expensive-to-create objects
/// - Trait objects that need Clone (Arc<dyn Trait>)
///
/// **When to use Box:**
/// - Single ownership with no sharing
/// - Trait objects without cloning (Box<dyn Trait>)
/// - Breaking recursive type cycles
/// - Storing large values on heap to avoid stack overflow
///
/// **Example:**
/// ```rust,ignore
/// // Arc: Shared across tasks
/// let ctx_clone = ctx.clone(); // Cheap: only increments Arc refcounts
/// tokio::spawn(async move {
///     // ctx_clone can be moved into async task
///     ctx_clone.http_client.get(url).send().await
/// });
///
/// // Box: Single owner, no sharing needed
/// let strategy: Box<dyn FormatSelectionStrategy> = Box::new(BestQualityStrategy);
/// let format = strategy.select(&formats);
/// // strategy is dropped here, cannot be cloned
/// ```
pub struct ExtractionContext {
    /// HTTP client for making requests
    ///
    /// **Arc-wrapped** for sharing across multiple extraction tasks without cloning the
    /// underlying connection pool. Reqwest's Client already uses Arc internally.
    pub http_client: Arc<reqwest::Client>,

    /// JavaScript engine for executing site JavaScript (e.g., signature decryption)
    ///
    /// **Arc<dyn Trait>** enables runtime polymorphism with shared ownership.
    /// Different JS engines (boa, V8) can be swapped without changing extractor code.
    pub js_engine: Arc<dyn JsEngine>,

    /// Cookie jar for authentication
    ///
    /// **Arc<dyn Trait>** allows sharing cookie state across extraction tasks while
    /// maintaining thread-safety for concurrent access.
    pub cookie_jar: Arc<dyn CookieJar>,

    /// Application configuration
    ///
    /// **Arc<Config>** eliminates expensive Config clones. Phase 1 optimization:
    /// sharing Config via Arc saves ~200ns per orchestrator creation.
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
