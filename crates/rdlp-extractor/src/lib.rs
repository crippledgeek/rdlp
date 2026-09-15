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

    /// Find a search extractor by site name (case-insensitive), arbitrating
    /// name collisions the same way [`Self::find_extractor`] arbitrates URL
    /// collisions: a built-in wins its own site name unless a plugin's
    /// signed manifest declared `claims_override` (surfaced here via
    /// [`SearchExtractor::overrides_builtin`]); among competing plugins the
    /// highest [`SearchExtractor::search_priority`] wins.
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

        let builtin = candidates.iter().find(|e| !e.is_plugin());
        let has_overriding_plugin = candidates
            .iter()
            .any(|e| e.is_plugin() && e.overrides_builtin());

        // Built-in wins its own site name unless a plugin declared the
        // override in its signed manifest (red-flagged at first install).
        if let Some(builtin) = builtin
            && !has_overriding_plugin
        {
            return Some(Arc::clone(builtin));
        }

        // `max_by_key` returns the LAST maximum on ties; reverse the
        // registration order first so a tie instead resolves to whichever
        // plugin registered first, matching `find_extractor`'s tie policy.
        candidates
            .into_iter()
            .filter(|e| e.is_plugin())
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
mod registry_c4a_tests {
    use super::*;

    #[test]
    fn registers_all_c4a_extractors() {
        let reg = ExtractorRegistry::new();
        let names = reg.list_extractors();
        for expected in ["xvideos", "xnxx", "eporner"] {
            assert!(
                names.iter().any(|n| n.eq_ignore_ascii_case(expected)),
                "expected {expected} in list_extractors, got {names:?}"
            );
        }
    }

    #[test]
    fn registers_all_c4a_search_extractors() {
        let reg = ExtractorRegistry::new();
        let names = reg.list_search_extractors();
        for expected in ["xvideos", "xnxx", "eporner"] {
            assert!(
                names.iter().any(|n| n.eq_ignore_ascii_case(expected)),
                "expected {expected} in search extractors, got {names:?}"
            );
        }
    }

    #[test]
    fn find_extractor_by_url_returns_correct_impl() {
        let reg = ExtractorRegistry::new();
        assert_eq!(
            reg.find_extractor("https://www.xvideos.com/video.ooumovia9b7/")
                .map(|e| e.name().to_string()),
            Some("XVideos".to_string())
        );
        assert_eq!(
            reg.find_extractor("https://www.xnxx.com/video-14cco143/slug")
                .map(|e| e.name().to_string()),
            Some("XNXX".to_string())
        );
        assert_eq!(
            reg.find_extractor("https://www.eporner.com/video-svXh0Ne27Ig/slug/")
                .map(|e| e.name().to_string()),
            Some("EPorner".to_string())
        );
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
        assert!(extractors.contains(&"HQPorner"));
        assert!(extractors.contains(&"SpankBang"));
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
    fn registry_routes_pornoxo_video_urls_to_the_dedicated_extractor() {
        let registry = ExtractorRegistry::new();
        let e = registry
            .find_extractor("https://www.pornoxo.com/videos/2928541/slug/")
            .expect("a PornoXO video URL must find an extractor");
        assert_eq!(e.name(), "PornoXO", "must not fall through to Generic");
    }

    #[test]
    fn registry_exposes_pornoxo_as_a_search_extractor() {
        let registry = ExtractorRegistry::new();
        assert!(
            registry.find_search_extractor("pornoxo").is_some(),
            "--search-site pornoxo must resolve"
        );
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

        let hqporner =
            registry.find_extractor("https://hqporner.com/hdporn/81203-full_body_massage.html");
        assert!(hqporner.is_some());
        assert_eq!(hqporner.unwrap().name(), "HQPorner");

        let spankbang = registry.find_extractor("https://spankbang.com/56b3d/video/the+slut+maker");
        assert!(spankbang.is_some());
        assert_eq!(spankbang.unwrap().name(), "SpankBang");

        // Generic fallback extractor matches all HTTP URLs, so a YouTube URL
        // now returns the Generic extractor instead of None.
        let generic = registry.find_extractor("https://youtube.com/watch?v=test");
        assert!(generic.is_some());
        assert_eq!(generic.unwrap().name(), "Generic");

        // Non-HTTP URLs still return None
        let ftp = registry.find_extractor("ftp://example.com/file");
        assert!(ftp.is_none());
    }

    #[test]
    fn test_find_search_extractor_hqporner() {
        let registry = ExtractorRegistry::new();
        let extractor = registry.find_search_extractor("hqporner");
        assert!(extractor.is_some());
        assert_eq!(extractor.unwrap().name(), "HQPorner");
    }

    /// #756/#(this task): `SearchExtractor::name` is documented as "should
    /// match the corresponding `InfoExtractor::name()`", and nothing checked
    /// it. Every search-capable site must be registered under the same name
    /// as an `InfoExtractor`, so `--search-site <name>` and
    /// `"extractor": "<name>"` in `--dump-json` agree.
    ///
    /// When the two sides disagree (nine_anime: `"9anime"` vs `"NineAnime"`,
    /// caught by this test's predecessor), resolve toward
    /// `InfoExtractor::name()`, never the other way. `InfoExtractor::name()`
    /// is the one written to disk — `record_in_archive` persists it into
    /// `--download-archive` files, `%(extractor)s` names output
    /// directories/files from it, and `--dump-json` emits it — so changing it
    /// invalidates every existing archive entry and renames users' folders.
    /// `SearchExtractor::name()` only feeds the case-insensitive
    /// `--search-site` lookup and has no on-disk footprint; it is the side
    /// that moves.
    ///
    /// Now exhaustive in both directions: the registry's registered set must
    /// equal `ExtractorName`'s declared set exactly (a registry addition with
    /// no matching variant, or a variant with nothing registered, both fail),
    /// in addition to the original search-vs-info direction.
    #[test]
    fn built_in_extractor_names_and_the_enum_are_the_same_set() {
        use rdlp_types::ExtractorName;
        use strum::IntoEnumIterator as _;

        let registry = ExtractorRegistry::new();
        let registered: std::collections::BTreeSet<&str> =
            registry.list_extractors().into_iter().collect();
        let declared: std::collections::BTreeSet<&str> =
            ExtractorName::iter().map(|n| n.as_str()).collect();
        assert_eq!(registered, declared, "registry vs ExtractorName drift");

        for name in registry.list_search_extractors() {
            assert!(
                registered.contains(name) && name.parse::<ExtractorName>().is_ok(),
                "search extractor {name:?} is not a registered ExtractorName"
            );
        }
    }
}

#[cfg(test)]
mod registry_search_arbitration_tests {
    use super::*;
    use async_trait::async_trait;
    use rdlp_types::{
        SearchFilterDescriptor, SearchPageResponse, SearchQuery, SearchResultPreview,
    };

    /// A minimal `SearchExtractor` double whose only job is to report a
    /// name/plugin-flag/priority/override combination, so arbitration can
    /// be pinned without a real site.
    struct FakeSearch {
        name: &'static str,
        plugin: bool,
        prio: i32,
        overrides: bool,
    }

    #[async_trait]
    impl SearchExtractor for FakeSearch {
        fn name(&self) -> &str {
            self.name
        }

        async fn supported_filters(&self) -> Vec<SearchFilterDescriptor> {
            Vec::new()
        }

        async fn search(
            &self,
            _query: &SearchQuery,
            _ctx: &rdlp_core::ExtractionContext,
        ) -> rdlp_core::Result<Vec<SearchResultPreview>> {
            Ok(Vec::new())
        }

        async fn search_page(
            &self,
            query: &SearchQuery,
            _ctx: &rdlp_core::ExtractionContext,
        ) -> rdlp_core::Result<SearchPageResponse> {
            Ok(SearchPageResponse {
                results: Vec::new(),
                page: query.page.unwrap_or(1),
                has_more: false,
                total_estimate: None,
            })
        }

        fn is_plugin(&self) -> bool {
            self.plugin
        }

        fn search_priority(&self) -> i32 {
            self.prio
        }

        fn overrides_builtin(&self) -> bool {
            self.overrides
        }
    }

    fn builtin(name: &'static str) -> FakeSearch {
        FakeSearch {
            name,
            plugin: false,
            prio: 0,
            overrides: false,
        }
    }

    fn plugin(name: &'static str, prio: i32, overrides: bool) -> FakeSearch {
        FakeSearch {
            name,
            plugin: true,
            prio,
            overrides,
        }
    }

    fn registry_of(extractors: Vec<FakeSearch>) -> ExtractorRegistry {
        let mut reg = ExtractorRegistry {
            extractors: Vec::new(),
            search_extractors: Vec::new(),
        };
        for e in extractors {
            reg.register_search(Arc::new(e));
        }
        reg
    }

    #[test]
    fn builtin_wins_its_own_site_over_a_non_overriding_plugin() {
        let reg = registry_of(vec![builtin("pornhub"), plugin("pornhub", 190, false)]);
        let found = reg.find_search_extractor("pornhub").expect("a match");
        assert!(
            !found.is_plugin(),
            "built-in must win when no override is claimed"
        );
    }

    /// A plugin literally named "evil" declaring the built-in's own site
    /// name (the manifest-level equivalent of `search_site = "pornhub"`)
    /// must NOT shadow the built-in unless it also claims the override —
    /// this is the exact shadowing attack the override gate exists for.
    #[test]
    fn a_plugin_claiming_a_builtins_site_name_cannot_shadow_it_without_override() {
        let reg = registry_of(vec![builtin("pornhub"), plugin("pornhub", 999, false)]);
        let found = reg.find_search_extractor("pornhub").expect("a match");
        assert!(
            !found.is_plugin(),
            "an unprivileged shadowing attempt must lose to the built-in regardless of priority"
        );
    }

    #[test]
    fn overriding_plugin_shadows_the_builtin() {
        let reg = registry_of(vec![builtin("pornhub"), plugin("pornhub", 100, true)]);
        let found = reg.find_search_extractor("pornhub").expect("a match");
        assert!(
            found.is_plugin(),
            "an override-claiming plugin must shadow the built-in"
        );
    }

    #[test]
    fn two_plugins_highest_priority_wins() {
        let reg = registry_of(vec![plugin("site", 120, false), plugin("site", 150, false)]);
        let found = reg.find_search_extractor("site").expect("a match");
        assert_eq!(found.search_priority(), 150);
    }

    #[test]
    fn equal_priority_plugins_resolve_to_first_registered() {
        // Equal-priority candidates are indistinguishable through any
        // `SearchExtractor` field, so the tie-break is pinned by pointer
        // identity (`Arc::ptr_eq`) against the specific instance registered
        // first — a `max_by_key` without the `.rev()` reversal would return
        // the second instance instead, and this assertion catches that.
        let mut reg = ExtractorRegistry {
            extractors: Vec::new(),
            search_extractors: Vec::new(),
        };
        let first: Arc<dyn SearchExtractor> = Arc::new(plugin("site", 100, false));
        let second: Arc<dyn SearchExtractor> = Arc::new(plugin("site", 100, false));
        reg.register_search(Arc::clone(&first));
        reg.register_search(Arc::clone(&second));

        let found = reg.find_search_extractor("site").expect("a match");
        assert!(
            Arc::ptr_eq(&found, &first),
            "a priority tie must resolve to whichever plugin registered first"
        );
    }

    #[test]
    fn plugin_only_name_returns_the_plugin() {
        let reg = registry_of(vec![plugin("onlyplugin", 100, false)]);
        let found = reg.find_search_extractor("onlyplugin").expect("a match");
        assert!(found.is_plugin());
    }

    #[test]
    fn list_search_extractors_dedupes_a_shared_name() {
        let reg = registry_of(vec![builtin("pornhub"), plugin("pornhub", 190, false)]);
        let names = reg.list_search_extractors();
        assert_eq!(
            names
                .iter()
                .filter(|n| n.eq_ignore_ascii_case("pornhub"))
                .count(),
            1,
            "a name shared by two providers must be listed once: {names:?}"
        );
    }

    #[test]
    fn name_matching_is_case_insensitive() {
        let reg = registry_of(vec![builtin("PornHub")]);
        assert!(reg.find_search_extractor("pornhub").is_some());
        assert!(reg.find_search_extractor("PORNHUB").is_some());
    }
}
