//! # rdlp-extractor
//!
//! Extractor framework and site-specific extractors for rdlp.
//!
//! This crate provides the extractor registry, URL routing, and site-specific
//! extraction implementations.
//!
//! ## Architecture
//!
//! The extractor system uses a layered architecture:
//!
//! 1. **Base Utilities** (`base::common`) - Common extraction utilities
//! 2. **Network Bases** (`base::tnaflix_network`) - Site family patterns
//! 3. **Site Extractors** (`extractors::*`) - Individual site implementations
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use rdlp_extractor::{ExtractorRegistry, BaseExtractor};
//!
//! // Find an extractor for a URL
//! let registry = ExtractorRegistry::new();
//! let extractor = registry.find_extractor(url)?;
//!
//! // Use base utilities in custom extractors
//! let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
//! let title = BaseExtractor::extract_title_multi_strategy(&html);
//! ```

#![warn(missing_docs)]

/// Base extraction utilities and network-specific base extractors
pub mod base;
/// Site-specific extractor implementations
pub mod extractors;
/// HLS size detection and playlist parsing
pub mod hls;
/// Utility functions for extraction
pub mod utils;

// Re-export extractors
pub use extractors::{
    HQPornerExtractor, NineAnimeExtractor, PornHubExtractor, RedTubeExtractor, TNAFlixExtractor,
    TNAFlixSearchExtractor, XHamsterExtractor, XTitsExtractor,
};

// Re-export base utilities for convenient access
pub use base::common::BaseExtractor;
pub use base::tnaflix_network::TnaFlixNetworkBase;

use rdlp_core::{InfoExtractor, SearchExtractor};
use std::sync::Arc;

/// Trait for extractor registries to enable mocking in tests
pub trait ExtractorRegistryTrait: Send + Sync {
    /// Find a suitable extractor for the given URL
    fn find_extractor(&self, url: &str) -> Option<Arc<dyn InfoExtractor>>;

    /// Get all registered extractor names
    fn list_extractors(&self) -> Vec<&str>;

    /// Find a search extractor by site name (case-insensitive).
    fn find_search_extractor(&self, _name: &str) -> Option<Arc<dyn SearchExtractor>> {
        None
    }

    /// List all registered search extractor names.
    fn list_search_extractors(&self) -> Vec<&str> {
        Vec::new()
    }
}

/// Registry for managing extractors
pub struct ExtractorRegistry {
    extractors: Vec<Arc<dyn InfoExtractor>>,
    search_extractors: Vec<Arc<dyn SearchExtractor>>,
}

impl ExtractorRegistry {
    /// Create a new registry with default extractors
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            extractors: Vec::with_capacity(8),
            search_extractors: Vec::with_capacity(4),
        };

        // Register TNAFlix network extractors
        registry.register(Arc::new(TNAFlixExtractor::tnaflix()));
        registry.register(Arc::new(TNAFlixExtractor::empflix()));
        registry.register(Arc::new(TNAFlixExtractor::moviefap()));

        // Register RedTube extractor
        registry.register(Arc::new(RedTubeExtractor::new()));

        // Register PornHub extractor (with playlist support)
        registry.register(Arc::new(PornHubExtractor::new()));

        // Register XTits extractor
        registry.register(Arc::new(XTitsExtractor::new()));

        // Register XHamster extractor
        registry.register(Arc::new(XHamsterExtractor::new()));

        // Register 9anime extractor
        registry.register(Arc::new(NineAnimeExtractor::new()));

        // Register HQPorner extractor
        registry.register(Arc::new(HQPornerExtractor::new()));

        // Register search extractors
        registry
            .search_extractors
            .push(Arc::new(XHamsterExtractor::new()));
        registry
            .search_extractors
            .push(Arc::new(RedTubeExtractor::new()));
        registry
            .search_extractors
            .push(Arc::new(TNAFlixSearchExtractor::new()));
        registry
            .search_extractors
            .push(Arc::new(PornHubExtractor::new()));

        registry
    }

    /// Register a new extractor
    ///
    /// # Arguments
    /// * `extractor` - Arc-wrapped extractor implementing InfoExtractor trait
    pub fn register(&mut self, extractor: Arc<dyn InfoExtractor>) {
        self.extractors.push(extractor);
    }

    /// Find a suitable extractor for the given URL
    ///
    /// Returns the extractor with the highest priority that reports the URL as suitable.
    /// Returns `None` if no extractor matches the URL.
    ///
    /// # Arguments
    /// * `url` - The URL to find an extractor for
    ///
    /// # Returns
    /// An `Arc<dyn InfoExtractor>` if a suitable extractor is found, `None` otherwise
    ///
    /// # Examples
    /// ```no_run
    /// use rdlp_extractor::ExtractorRegistry;
    ///
    /// let registry = ExtractorRegistry::new();
    /// let extractor = registry.find_extractor("https://www.tnaflix.com/video/123");
    /// assert!(extractor.is_some());
    /// ```
    #[must_use]
    pub fn find_extractor(&self, url: &str) -> Option<Arc<dyn InfoExtractor>> {
        self.extractors
            .iter()
            .filter(|e| e.suitable(url))
            .max_by_key(|e| e.priority())
            .cloned()
    }

    /// Get all registered extractor names
    ///
    /// # Returns
    /// A vector of extractor names (e.g., ["TNAFlix", "EMPFlix", "MovieFap"])
    #[must_use]
    pub fn list_extractors(&self) -> Vec<&str> {
        self.extractors.iter().map(|e| e.name()).collect()
    }

    /// Find a search extractor by site name (case-insensitive).
    ///
    /// # Arguments
    /// * `name` - Site name to look up (e.g., "xhamster", "XHamster")
    ///
    /// # Returns
    /// An `Arc<dyn SearchExtractor>` if found, `None` otherwise
    #[must_use]
    pub fn find_search_extractor(&self, name: &str) -> Option<Arc<dyn SearchExtractor>> {
        self.search_extractors
            .iter()
            .find(|e| e.name().eq_ignore_ascii_case(name))
            .cloned()
    }

    /// List all registered search extractor names.
    ///
    /// # Returns
    /// A vector of site names that support search
    #[must_use]
    pub fn list_search_extractors(&self) -> Vec<&str> {
        self.search_extractors.iter().map(|e| e.name()).collect()
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtractorRegistryTrait for ExtractorRegistry {
    fn find_extractor(&self, url: &str) -> Option<Arc<dyn InfoExtractor>> {
        self.find_extractor(url)
    }

    fn list_extractors(&self) -> Vec<&str> {
        self.list_extractors()
    }

    fn find_search_extractor(&self, name: &str) -> Option<Arc<dyn SearchExtractor>> {
        self.find_search_extractor(name)
    }

    fn list_search_extractors(&self) -> Vec<&str> {
        self.list_search_extractors()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = ExtractorRegistry::new();
        let extractors = registry.list_extractors();
        assert!(extractors.contains(&"TNAFlix"));
        assert!(extractors.contains(&"EMPFlix"));
        assert!(extractors.contains(&"MovieFap"));
        assert!(extractors.contains(&"RedTube"));
        assert!(extractors.contains(&"PornHub"));
        assert!(extractors.contains(&"XTits"));
        assert!(extractors.contains(&"XHamster"));
        assert!(extractors.contains(&"9anime"));
    }

    #[test]
    fn test_find_search_extractor_xhamster() {
        let registry = ExtractorRegistry::new();
        let extractor = registry.find_search_extractor("xhamster");
        assert!(extractor.is_some());
        assert_eq!(extractor.unwrap().name(), "XHamster");
    }

    #[test]
    fn test_find_search_extractor_case_insensitive() {
        let registry = ExtractorRegistry::new();
        assert!(registry.find_search_extractor("XHamster").is_some());
        assert!(registry.find_search_extractor("XHAMSTER").is_some());
    }

    #[test]
    fn test_find_search_extractor_tnaflix() {
        let registry = ExtractorRegistry::new();
        let extractor = registry.find_search_extractor("tnaflix");
        assert!(extractor.is_some());
        assert_eq!(extractor.unwrap().name(), "TNAFlix");
    }

    #[test]
    fn test_find_search_extractor_pornhub() {
        let registry = ExtractorRegistry::new();
        let extractor = registry.find_search_extractor("pornhub");
        assert!(extractor.is_some());
        assert_eq!(extractor.unwrap().name(), "PornHub");
    }

    #[test]
    fn test_find_search_extractor_unknown() {
        let registry = ExtractorRegistry::new();
        assert!(registry.find_search_extractor("nonexistent").is_none());
    }

    #[test]
    fn test_list_search_extractors() {
        let registry = ExtractorRegistry::new();
        let sites = registry.list_search_extractors();
        assert!(
            sites
                .iter()
                .any(|name| name.eq_ignore_ascii_case("xhamster"))
        );
    }

    #[test]
    fn test_find_extractor() {
        let registry = ExtractorRegistry::new();

        let tnaflix = registry.find_extractor("https://www.tnaflix.com/hd-videos/test/video123");
        assert!(tnaflix.is_some());
        assert_eq!(tnaflix.unwrap().name(), "TNAFlix");

        let empflix = registry.find_extractor("https://www.empflix.com/videos/test-123");
        assert!(empflix.is_some());
        assert_eq!(empflix.unwrap().name(), "EMPFlix");

        let redtube = registry.find_extractor("https://www.redtube.com/123456");
        assert!(redtube.is_some());
        assert_eq!(redtube.unwrap().name(), "RedTube");

        let xtits = registry.find_extractor("https://www.xtits.xxx/videos/183207/spicy-lesbians/");
        assert!(xtits.is_some());
        assert_eq!(xtits.unwrap().name(), "XTits");

        let xhamster = registry.find_extractor("https://xhamster.com/videos/test-video-1509445");
        assert!(xhamster.is_some());
        assert_eq!(xhamster.unwrap().name(), "XHamster");

        let nine_anime =
            registry.find_extractor("https://9animetv.to/watch/sword-art-online-2274?ep=26565");
        assert!(nine_anime.is_some());
        assert_eq!(nine_anime.unwrap().name(), "9anime");

        let unknown = registry.find_extractor("https://youtube.com/watch?v=test");
        assert!(unknown.is_none());
    }
}
