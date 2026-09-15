// `clippy::pedantic` / `clippy::nursery` / `clippy::indexing_slicing` are
// intentionally NOT applied to this crate. The codebase has ~1400 stylistic
// hits under those families (mostly `doc_markdown`, `redundant_pub_crate`,
// `non_std_lazy_statics` — the last fires on every `lazy_regex::Lazy<Regex>`,
// which is `once_cell::sync::Lazy` re-exported and IS the documented idiom
// for compile-time-validated static regex). The strict surface that matters
// for security correctness — `unwrap_used`, `expect_used`,
// `missing_errors_doc`, `missing_panics_doc` — lives in `Cargo.toml`
// `[lints.clippy]` and applies to ALL targets, not just lib.

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
//! 1. **Base Utilities** (`base::common`) - Common extraction utilities,
//!    including `base::common::dash::expand_dash_representations` which eagerly
//!    expands DASH MPD manifests into per-Representation [`rdlp_types::Format`] entries so
//!    that `-f bv*+ba*` selection works against DASH the same as HTTP or HLS.
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

// `loopback-test-exemption` widens the HLS seed gate's loopback bypass
// (`base::common::manifest_url`) and exists only so sibling crates' tests
// can drive expansion against mockito. It is enabled from dev-dependencies,
// and Cargo unifies features per profile, so `--all-features` or a stray
// non-dev dependency could carry it into a release build. A release build
// runs without `debug_assertions`; a test build (any profile Cargo uses for
// `cargo test` without `--release`) has them on, so this fires exactly on
// the combination that would ship the bypass. `cargo test --release` is
// not used anywhere in this repository (see BUILDING.md).
#[cfg(all(feature = "loopback-test-exemption", not(any(test, debug_assertions))))]
compile_error!(
    "the `loopback-test-exemption` feature is test-only and must not be enabled in a release build"
);

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
    AbxxxExtractor, EMPFlixSearchExtractor, EPornerExtractor, GenericExtractor, HQPornerExtractor,
    KoreanPornMovieExtractor, MovieFapSearchExtractor, NineAnimeExtractor, PornHubExtractor,
    PornoneExtractor, PornoxoExtractor, RedTubeExtractor, SpankBangExtractor, TNAFlixExtractor,
    TNAFlixSearchExtractor, XHamsterExtractor, XNXXExtractor, XTitsExtractor, XVideosExtractor,
};

// Re-export base utilities for convenient access
pub use base::common::BaseExtractor;
pub use base::tnaflix_network::TnaFlixNetworkBase;
pub use base::wgcz_network::WgczNetworkBase;

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

        // Register KoreanPornMovie extractor
        registry.register(Arc::new(KoreanPornMovieExtractor::new()));

        // Register WGCZ network extractors (XVideos, XNXX)
        registry.register(Arc::new(XVideosExtractor::new()));
        registry.register(Arc::new(XNXXExtractor::new()));

        // Register EPorner extractor
        registry.register(Arc::new(EPornerExtractor::new()));

        // Register ABXXX extractor (KVS site with JSON XHR player config)
        registry.register(Arc::new(AbxxxExtractor::new()));
        registry.register_search(Arc::new(AbxxxExtractor::new()));

        // Register SpankBang extractor
        registry.register(Arc::new(SpankBangExtractor::new()));

        // Register PornoXO extractor
        registry.register(Arc::new(PornoxoExtractor::new()));

        // Register PornOne extractor
        registry.register(Arc::new(PornoneExtractor::new()));

        // Register Generic fallback extractor (MUST be last — lowest priority)
        registry.register(Arc::new(GenericExtractor::new()));

        // Register search extractors
        registry.register_search(Arc::new(XHamsterExtractor::new()));
        registry.register_search(Arc::new(RedTubeExtractor::new()));
        registry.register_search(Arc::new(TNAFlixSearchExtractor::new()));
        registry.register_search(Arc::new(PornHubExtractor::new()));
        registry.register_search(Arc::new(HQPornerExtractor::new()));
        registry.register_search(Arc::new(EMPFlixSearchExtractor::new()));
        registry.register_search(Arc::new(MovieFapSearchExtractor::new()));
        registry.register_search(Arc::new(XTitsExtractor::new()));
        registry.register_search(Arc::new(NineAnimeExtractor::new()));
        registry.register_search(Arc::new(KoreanPornMovieExtractor::new()));
        registry.register_search(Arc::new(XVideosExtractor::new()));
        registry.register_search(Arc::new(XNXXExtractor::new()));
        registry.register_search(Arc::new(EPornerExtractor::new()));
        registry.register_search(Arc::new(SpankBangExtractor::new()));
        registry.register_search(Arc::new(PornoxoExtractor::new()));
        registry.register_search(Arc::new(PornoneExtractor::new()));

        registry
    }

    /// Register a new extractor
    ///
    /// # Arguments
    /// * `extractor` - Arc-wrapped extractor implementing InfoExtractor trait
    pub fn register(&mut self, extractor: Arc<dyn InfoExtractor>) {
        self.extractors.push(extractor);
    }

    /// Register a new search extractor
    ///
    /// # Arguments
    /// * `extractor` - Arc-wrapped extractor implementing `SearchExtractor` trait
    pub fn register_search(&mut self, extractor: Arc<dyn SearchExtractor>) {
        self.search_extractors.push(extractor);
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
        // Two-pass selection so plugins shadowing built-in host space get
        // clamped: gather suitable candidates, note whether any built-in
        // is among them, then sort by URL-aware effective_priority.
        let suitable: Vec<&Arc<dyn InfoExtractor>> =
            self.extractors.iter().filter(|e| e.suitable(url)).collect();
        let builtin_competitor = suitable.iter().any(|e| !e.is_plugin());
        suitable
            .into_iter()
            .max_by_key(|e| e.effective_priority(url, builtin_competitor))
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

    /// Find a search extractor by site name (case-insensitive), applying
    /// this policy on a name collision: a built-in wins its own site name
    /// unless a plugin's signed manifest declared `claims_override`
    /// (surfaced here via [`SearchExtractor::overrides_builtin`]) — and
    /// when a built-in IS present, only override-claiming plugins are
    /// even eligible to compete for the name, so a bystander plugin's
    /// priority can never hijack a site it never claimed the right to
    /// shadow. Among the eligible plugins the highest
    /// [`SearchExtractor::search_priority`] wins; a tie resolves to
    /// whichever plugin registered first. This tie policy is a
    /// **deliberate divergence** from [`Self::find_extractor`], whose
    /// plain `max_by_key` over URL candidates resolves ties to whichever
    /// extractor registered LAST — the two are not "the same policy".
    ///
    /// # Arguments
    /// * `name` - Site name to look up (e.g., "xhamster", "XHamster")
    ///
    /// # Returns
    /// An `Arc<dyn SearchExtractor>` if found, `None` otherwise
    #[must_use]
    pub fn find_search_extractor(&self, name: &str) -> Option<Arc<dyn SearchExtractor>> {
        let candidates: Vec<&Arc<dyn SearchExtractor>> = self
            .search_extractors
            .iter()
            .filter(|e| e.name().eq_ignore_ascii_case(name))
            .collect();

        let builtin = candidates.iter().copied().find(|e| !e.is_plugin());

        // When a built-in exists, only plugins that claim `overrides_builtin`
        // are eligible to contest its site name at all — a non-overriding
        // plugin's priority must never matter against a built-in it did not
        // claim the right to shadow. Without a built-in, every plugin is
        // eligible and competes on priority alone.
        let eligible_plugins: Vec<&Arc<dyn SearchExtractor>> = candidates
            .iter()
            .copied()
            .filter(|e| e.is_plugin() && (builtin.is_none() || e.overrides_builtin()))
            .collect();

        if let Some(builtin) = builtin
            && eligible_plugins.is_empty()
        {
            return Some(Arc::clone(builtin));
        }

        // `max_by_key` returns the LAST maximum on ties; reverse the
        // registration order first so a tie instead resolves to whichever
        // eligible plugin registered first.
        eligible_plugins
            .into_iter()
            .rev()
            .max_by_key(|e| e.search_priority())
            .cloned()
    }

    /// List all registered search extractor names, deduplicated
    /// case-insensitively so a name shared by a built-in and a plugin is
    /// listed once (keeping the casing and position of whichever
    /// registered first).
    ///
    /// # Returns
    /// A vector of site names that support search
    #[must_use]
    pub fn list_search_extractors(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::with_capacity(self.search_extractors.len());
        for extractor in &self.search_extractors {
            let name = extractor.name();
            if !names.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
                names.push(name);
            }
        }
        names
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
mod registry_tests;
