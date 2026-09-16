//! Registry tests: built-in registration, URL routing, and search-site
//! arbitration. Split out of `lib.rs` so the registry's production code
//! reads without three test modules beneath it (CODING_RULES.md, "Test
//! placement").

use super::*;

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
        assert!(extractors.contains(&"9anime"));
        assert!(extractors.contains(&"HQPorner"));
        assert!(extractors.contains(&"SpankBang"));
    }

    #[test]
    fn test_find_search_extractor_case_insensitive() {
        let registry = ExtractorRegistry::new();
        assert!(registry.find_search_extractor("PornHub").is_some());
        assert!(registry.find_search_extractor("PORNHUB").is_some());
    }

    /// xhamster ships as the rdlp-plugins `xhamster` plugin (removed here in
    /// slice C0-b, #771; landing as rdlp#762 slice C1-b), so the built-in
    /// registry must neither route its URLs to a
    /// dedicated extractor nor claim it as a search site — the plugin
    /// host would otherwise lose the arbitration to the built-in.
    #[test]
    fn xhamster_is_not_a_built_in() {
        let registry = ExtractorRegistry::new();
        assert!(
            registry
                .find_extractor("https://xhamster.com/videos/x-1")
                .is_none_or(|e| e.name() == "Generic"),
            "an xhamster URL must fall through to Generic"
        );
        assert!(registry.find_search_extractor("xhamster").is_none());
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
                .any(|name| name.eq_ignore_ascii_case("pornhub"))
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
        /// `None` = the trait default (`display_name()` echoes `name()`).
        display: Option<&'static str>,
        plugin: bool,
        prio: i32,
        overrides: bool,
    }

    #[async_trait]
    impl SearchExtractor for FakeSearch {
        fn name(&self) -> &str {
            self.name
        }

        fn display_name(&self) -> &str {
            self.display.unwrap_or(self.name)
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
            display: None,
            plugin: false,
            prio: 0,
            overrides: false,
        }
    }

    fn plugin(name: &'static str, prio: i32, overrides: bool) -> FakeSearch {
        FakeSearch {
            name,
            display: None,
            plugin: true,
            prio,
            overrides,
        }
    }

    /// An `ExtractorRegistry` with no built-in extractors registered, so
    /// tests can pin arbitration against exactly the `FakeSearch` doubles
    /// they construct instead of the ~30 real built-ins `new()` populates.
    fn empty() -> ExtractorRegistry {
        ExtractorRegistry {
            extractors: Vec::new(),
            search_extractors: Vec::new(),
        }
    }

    fn registry_of(extractors: Vec<FakeSearch>) -> ExtractorRegistry {
        let mut reg = empty();
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

    /// Regression for the round-1 CRITICAL finding: once ANY plugin among
    /// the candidates claims `overrides_builtin`, a naive final
    /// `max_by_key` over ALL plugins lets a higher-priority
    /// NON-overriding plugin hijack the site out from under the built-in.
    /// Only the override-claiming plugin may ever contest a built-in's
    /// site name; a bystander plugin's priority is irrelevant.
    #[test]
    fn a_non_overriding_plugin_cannot_hijack_via_priority_once_another_plugin_overrides() {
        let reg = registry_of(vec![
            builtin("pornhub"),
            plugin("pornhub", 100, true),
            plugin("pornhub", 200, false),
        ]);
        let found = reg.find_search_extractor("pornhub").expect("a match");
        assert_eq!(
            found.search_priority(),
            100,
            "only the override-claiming plugin may contest the built-in's site name"
        );
        assert!(found.overrides_builtin());
    }

    #[test]
    fn equal_priority_plugins_resolve_to_first_registered() {
        // Equal-priority candidates are indistinguishable through any
        // `SearchExtractor` field, so the tie-break is pinned by pointer
        // identity (`Arc::ptr_eq`) against the specific instance registered
        // first — a `max_by_key` without the `.rev()` reversal would return
        // the second instance instead, and this assertion catches that.
        let mut reg = empty();
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
        // Exact-Vec assertion (not just a count) so both the casing kept
        // (the first-registered provider's) and the surviving order (the
        // shared name first, in its original slot) are pinned.
        let reg = registry_of(vec![
            builtin("PornHub"),
            plugin("PORNHUB", 190, false),
            builtin("other"),
        ]);
        let names = reg.list_search_extractors();
        assert_eq!(names, vec!["PornHub", "other"]);
    }

    /// A plugin's `name()` is its `search_site` routing key (`xhamster`)
    /// while its `display_name()` is the manifest's human label
    /// (`XHamster`); the site list must carry BOTH, not derive the label
    /// from the key. In-tree extractors satisfy both roles with one
    /// string, which is why this only shows with a plugin-shaped double.
    #[test]
    fn list_search_sites_carries_display_name_separately_from_routing_key() {
        let reg = registry_of(vec![
            builtin("PornHub"),
            FakeSearch {
                display: Some("XHamster"),
                ..plugin("xhamster", 150, false)
            },
        ]);
        let sites = reg.list_search_sites();
        let names: Vec<(&str, &str)> = sites
            .iter()
            .map(|s| (s.name.as_str(), s.display_name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![("pornhub", "PornHub"), ("xhamster", "XHamster")]
        );
    }
}
