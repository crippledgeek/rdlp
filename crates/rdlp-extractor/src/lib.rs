//! # rdlp-extractor
//!
//! Extractor framework and site-specific extractors for rdlp.
//!
//! This crate provides the extractor registry, URL routing, and site-specific
//! extraction implementations.

pub mod extractors;

pub use extractors::{TNAFlixExtractor};

use rdlp_core::InfoExtractor;
use std::sync::Arc;

/// Registry for managing extractors
pub struct ExtractorRegistry {
    extractors: Vec<Arc<dyn InfoExtractor>>,
}

impl ExtractorRegistry {
    /// Create a new registry with default extractors
    pub fn new() -> Self {
        let mut registry = Self {
            extractors: Vec::new(),
        };

        // Register TNAFlix network extractors
        registry.register(Arc::new(TNAFlixExtractor::tnaflix()));
        registry.register(Arc::new(TNAFlixExtractor::empflix()));
        registry.register(Arc::new(TNAFlixExtractor::moviefap()));

        registry
    }

    /// Register a new extractor
    pub fn register(&mut self, extractor: Arc<dyn InfoExtractor>) {
        self.extractors.push(extractor);
    }

    /// Find a suitable extractor for the given URL
    pub fn find_extractor(&self, url: &str) -> Option<Arc<dyn InfoExtractor>> {
        self.extractors
            .iter()
            .filter(|e| e.suitable(url))
            .max_by_key(|e| e.priority())
            .cloned()
    }

    /// Get all registered extractors
    pub fn list_extractors(&self) -> Vec<String> {
        self.extractors
            .iter()
            .map(|e| e.name().to_string())
            .collect()
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = ExtractorRegistry::new();
        let extractors = registry.list_extractors();
        assert!(extractors.contains(&"TNAFlix".to_string()));
        assert!(extractors.contains(&"EMPFlix".to_string()));
        assert!(extractors.contains(&"MovieFap".to_string()));
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

        let unknown = registry.find_extractor("https://youtube.com/watch?v=test");
        assert!(unknown.is_none());
    }
}
