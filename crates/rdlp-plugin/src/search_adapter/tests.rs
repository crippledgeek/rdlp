use super::*;

#[test]
fn unsupported_is_the_search_only_variant() {
    assert!(matches!(
        search_error_to_plugin_error("example", WitSearchError::Unsupported),
        PluginError::SearchUnsupported { ref plugin } if plugin == "example"
    ));
}

#[test]
fn shared_cases_map_like_the_extract_mapper() {
    assert!(matches!(
        search_error_to_plugin_error("example", WitSearchError::RateLimited(Some(7))),
        PluginError::RateLimited {
            retry_after: Some(7),
            ..
        }
    ));
    assert!(matches!(
        search_error_to_plugin_error("example", WitSearchError::Network("dns".into())),
        PluginError::ExtractNetwork { ref detail, .. } if detail == "dns"
    ));
    assert!(matches!(
        search_error_to_plugin_error("example", WitSearchError::Parse("html".into())),
        PluginError::ExtractParse { ref detail, .. } if detail == "html"
    ));
    assert!(matches!(
        search_error_to_plugin_error("example", WitSearchError::Cancelled),
        PluginError::Cancelled { .. }
    ));
    match search_error_to_plugin_error("example", WitSearchError::Internal("bad".into())) {
        PluginError::Internal(msg) => assert_eq!(msg, "plugin example: bad"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn only_internal_among_the_search_errors_is_a_strike() {
    use crate::adapter::counts_as_strike;
    let cases = [
        (WitSearchError::Unsupported, false),
        (WitSearchError::RateLimited(None), false),
        (WitSearchError::Network(String::new()), false),
        (WitSearchError::Parse(String::new()), false),
        (WitSearchError::Cancelled, false),
        (WitSearchError::Internal(String::new()), true),
    ];
    for (err, strike) in cases {
        let mapped = search_error_to_plugin_error("example", err);
        assert_eq!(counts_as_strike(&mapped), strike, "{mapped:?}");
    }
}

/// Drift guard for the hand-written lift: the record's field lines in
/// `wit/types.wit` must be exactly these, in this order — the component
/// canonical ABI lifts records positionally, so a reordered or retyped
/// field would lift garbage without a type error.
#[test]
fn lift_mirrors_the_wit_record_field_for_field() {
    const TYPES_WIT: &str = include_str!("../../wit/types.wit");
    let expected = [
        "key: string,",
        "display-name: string,",
        "allowed-values: list<string>,",
        "default: option<string>,",
    ];
    let (_, after) = TYPES_WIT
        .split_once("record search-filter-descriptor {")
        .expect("types.wit declares search-filter-descriptor");
    let (body, _) = after.split_once('}').expect("record body is closed");
    let fields: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        fields, expected,
        "search-filter-descriptor drifted from the Rust lift"
    );
}

#[test]
fn descriptor_labels_are_the_values() {
    let d = descriptor_from_wit(
        WitSearchFilterDescriptor {
            key: "ordering".into(),
            display_name: "Ordering".into(),
            allowed_values: vec!["newest".into(), "views".into()],
            default: Some("newest".into()),
        },
        &test_origin(),
    )
    .expect("within every bound");
    assert_eq!(d.key, "ordering");
    assert_eq!(d.display_name, "Ordering");
    assert_eq!(d.default.as_deref(), Some("newest"));
    assert_eq!(
        d.allowed_values,
        SearchFilterValue::list([("newest", "newest"), ("views", "views")])
    );
}

// ── PluginSearchExtractor ─────────────────────────────────────────────

use crate::test_support::unit::{
    FIXTURE_MANIFEST, TEST_LOG_TARGET, captured_entry_containing, captured_logs, fixture_extractor,
    fixture_extractor_with_manifest, test_origin,
};

fn host_query(filters: &[(&str, &str)]) -> SearchQuery {
    SearchQuery {
        query: "kittens".into(),
        filters: filters
            .iter()
            .map(|(k, v)| SearchFilter {
                key: (*k).into(),
                value: (*v).into(),
            })
            .collect(),
        max_results: None,
        page: None,
    }
}

#[test]
fn query_to_wit_sets_the_page_and_keeps_filter_tuples_in_order() {
    let q = host_query(&[("sort", "top"), ("period", "week")]);
    let wit = search_query_to_wit(&q, 3);
    assert_eq!(wit.query, "kittens");
    assert_eq!(wit.page, Some(3));
    assert_eq!(
        wit.filters,
        vec![
            ("sort".to_string(), "top".to_string()),
            ("period".to_string(), "week".to_string()),
        ]
    );
}

#[test]
fn page_from_wit_maps_every_result_field_and_the_page_flags() {
    let page = search_page_from_wit(WitSearchPage {
        results: vec![WitSearchResult {
            url: "https://example.com/video/1".into(),
            title: "One".into(),
            thumbnail: Some("https://example.com/1.jpg".into()),
            duration: Some(90),
            uploader: Some("someone".into()),
        }],
        page: 1,
        has_more: true,
        total_estimate: Some(42),
    });
    assert!(page.has_more);
    assert_eq!(page.total_estimate, Some(42));
    let [r] = page.results.as_slice() else {
        panic!("one result expected: {:?}", page.results)
    };
    assert_eq!(r.video_url, "https://example.com/video/1");
    assert_eq!(r.title, "One");
    assert_eq!(
        r.thumbnail_url.as_deref(),
        Some("https://example.com/1.jpg")
    );
    assert_eq!(r.duration, Some(90.0));
    assert_eq!(r.uploader.as_deref(), Some("someone"));
    // The WIT record carries none of these; they must stay unset.
    assert_eq!(r.uploader_url, None);
    assert!(r.actors.is_empty());
    assert_eq!(r.view_count, None);
    assert_eq!(r.upload_date, None);
}

#[test]
fn page_from_wit_keeps_absent_optionals_absent() {
    let page = search_page_from_wit(WitSearchPage {
        results: vec![WitSearchResult {
            url: "https://example.com/video/2".into(),
            title: "Two".into(),
            thumbnail: None,
            duration: None,
            uploader: None,
        }],
        page: 2,
        has_more: false,
        total_estimate: None,
    });
    assert!(!page.has_more);
    assert_eq!(page.total_estimate, None);
    let [r] = page.results.as_slice() else {
        panic!("one result expected: {:?}", page.results)
    };
    assert_eq!(r.thumbnail_url, None);
    assert_eq!(r.duration, None);
    assert_eq!(r.uploader, None);
}

fn sort_descriptor() -> SearchFilterDescriptor {
    SearchFilterDescriptor::new(
        "sort",
        "Sort",
        SearchFilterValue::list([("top", "top")]),
        None,
    )
}

/// The fixture is a 0.5.0 component with NO `search-filters` export, so
/// a validator that reached the wasm would see zero descriptors and
/// report `sort` as an unknown key. `InvalidValue` proves the cached
/// descriptors were consulted instead.
#[test]
fn validation_reads_the_cached_descriptors_not_the_component() {
    let ext =
        PluginSearchExtractor::with_filters(Arc::new(fixture_extractor()), vec![sort_descriptor()]);
    let err = ext
        .validate_search_filters(&host_query(&[("sort", "new")]).filters)
        .expect_err("out-of-set value");
    match err {
        RdlpError::Extraction { message, url } => {
            assert_eq!(
                message,
                "Invalid value 'new' for filter 'sort'. Allowed: top"
            );
            assert_eq!(url, None);
        }
        other => panic!("got {other:?}"),
    }
    assert_eq!(ext.inner.test_trap_count(), 0);
}

#[test]
fn validation_rejects_an_unknown_key_naming_the_site() {
    let ext =
        PluginSearchExtractor::with_filters(Arc::new(fixture_extractor()), vec![sort_descriptor()]);
    let err = ext
        .validate_search_filters(&host_query(&[("foo", "x")]).filters)
        .expect_err("unknown key");
    match err {
        RdlpError::Extraction { message, .. } => {
            assert_eq!(message, "Unknown filter 'foo' for example. Available: sort");
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn validation_accepts_an_allowed_value_and_no_filters() {
    let ext =
        PluginSearchExtractor::with_filters(Arc::new(fixture_extractor()), vec![sort_descriptor()]);
    ext.validate_search_filters(&host_query(&[("sort", "top")]).filters)
        .expect("allowed value");
    ext.validate_search_filters(&[]).expect("no filters");
}

/// Before `search`/`search_page` prime the cache, the sync validator
/// has no descriptors: every filter is unknown, none is not.
#[test]
fn an_unprimed_cache_treats_every_filter_as_unknown() {
    let ext = PluginSearchExtractor::new(Arc::new(fixture_extractor()));
    ext.validate_search_filters(&[]).expect("no filters");
    let err = ext
        .validate_search_filters(&host_query(&[("sort", "top")]).filters)
        .expect_err("nothing is known yet");
    match err {
        RdlpError::Extraction { message, .. } => {
            assert_eq!(
                message,
                "Unknown filter 'sort' for example. Available: (none)"
            );
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn identity_and_arbitration_come_from_the_manifest() {
    let plain = PluginSearchExtractor::new(Arc::new(fixture_extractor()));
    assert_eq!(SearchExtractor::name(&plain), "example");
    assert!(plain.is_plugin());
    assert_eq!(plain.search_priority(), 150);
    assert!(!plain.overrides_builtin());
    assert_eq!(plain.first_page_index(), 1);

    let ext = PluginSearchExtractor::new(Arc::new(fixture_extractor_with_manifest(
        &manifest_claiming("pornhub", "search_claims_override = [\"pornhub\"]"),
    )));
    assert_eq!(SearchExtractor::name(&ext), "pornhub");
    assert!(ext.overrides_builtin());
}

// ── search override binding (security M3) ─────────────────────────────

/// The fixture manifest serving `site` for search, with `claim` lines
/// appended verbatim.
fn manifest_claiming(site: &str, claim: &str) -> String {
    FIXTURE_MANIFEST.replace(
        "capabilities = []",
        &format!("capabilities = []\nsupports_search = true\nsearch_site = \"{site}\"\n{claim}"),
    )
}

/// The shadowing attempt: a plugin naming a built-in's site with only a
/// URL-routing `claims_override` (for a host it does match) claims
/// nothing about search — the override must be the explicit,
/// site-bound `search_claims_override`.
#[test]
fn a_url_claims_override_alone_does_not_override_builtin_search() {
    let ext = PluginSearchExtractor::new(Arc::new(fixture_extractor_with_manifest(
        &manifest_claiming("pornhub", "claims_override = [\"example.com\"]"),
    )));
    assert_eq!(SearchExtractor::name(&ext), "pornhub");
    assert!(!ext.overrides_builtin());
}

/// End to end through the real registry: against the built-in
/// `pornhub`, the URL-only claimant LOSES and the site-bound claimant
/// WINS — the registry consults exactly `overrides_builtin`.
#[test]
fn registry_arbitration_honours_only_the_site_bound_claim() {
    use rdlp_extractor::ExtractorRegistry;

    let mut reg = ExtractorRegistry::new();
    reg.register_search(Arc::new(PluginSearchExtractor::new(Arc::new(
        fixture_extractor_with_manifest(&manifest_claiming(
            "pornhub",
            "claims_override = [\"example.com\"]",
        )),
    ))));
    let found = reg
        .find_search_extractor("pornhub")
        .expect("the built-in exists");
    assert!(
        !found.is_plugin(),
        "a URL-only claimant must lose to the built-in"
    );

    let mut reg = ExtractorRegistry::new();
    reg.register_search(Arc::new(PluginSearchExtractor::new(Arc::new(
        fixture_extractor_with_manifest(&manifest_claiming(
            "pornhub",
            "search_claims_override = [\"pornhub\"]",
        )),
    ))));
    let found = reg.find_search_extractor("pornhub").expect("a match");
    assert!(
        found.is_plugin(),
        "the site-bound claimant must shadow the built-in"
    );
}

/// A plugin the runner refuses (here: disabled by the 3-strike rule)
/// answers no filters — and the failure is NOT cached, so the cell
/// stays empty for a retry once the plugin can answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_filters_fetch_reports_no_filters_and_is_not_cached() {
    let inner = Arc::new(fixture_extractor());
    for _ in 0..crate::adapter::TRAP_DISABLE_THRESHOLD {
        inner.test_record_trap();
    }
    assert!(inner.test_is_disabled());
    let ext = PluginSearchExtractor::new(inner);
    assert!(ext.supported_filters().await.is_empty());
    assert!(ext.filters.get().is_none(), "a failure must not be cached");
}

/// `SearchUnsupported`'s `Display` IS the operator message, so the
/// generic mapper needs no special arm; this pins the wording the
/// operator sees for a plugin that exports `search` only to satisfy
/// the world.
#[test]
fn search_unsupported_reaches_the_operator_by_name() {
    let err = search_call_error(PluginError::SearchUnsupported {
        plugin: "example".into(),
    });
    match err {
        RdlpError::Extraction { message, url } => {
            assert_eq!(message, "plugin 'example' does not support search");
            assert_eq!(url, None);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn descriptor_without_default_or_values_is_preserved() {
    let d = descriptor_from_wit(
        WitSearchFilterDescriptor {
            key: "k".into(),
            display_name: "K".into(),
            allowed_values: vec![],
            default: None,
        },
        &test_origin(),
    )
    .expect("within every bound");
    assert!(d.allowed_values.is_empty());
    assert_eq!(d.default, None);
}

// ── descriptor bounds (security L3) ───────────────────────────────────

fn descriptor(key: &str, values: usize) -> WitSearchFilterDescriptor {
    WitSearchFilterDescriptor {
        key: key.into(),
        display_name: key.to_uppercase(),
        allowed_values: (0..values).map(|i| format!("v{i}")).collect(),
        default: None,
    }
}

/// Exactly `MAX_SEARCH_FILTER_DESCRIPTORS` descriptors all survive; one
/// more is cut back to the bound, with the first ones kept, and the cut
/// is reported on the plugin's log target.
#[test]
fn descriptor_count_is_capped_at_the_bound_inclusive() {
    let logs = captured_logs();
    let at = descriptors_from_wit(
        (0..MAX_SEARCH_FILTER_DESCRIPTORS)
            .map(|i| descriptor(&format!("k{i}"), 1))
            .collect(),
        &test_origin(),
    );
    assert_eq!(at.len(), MAX_SEARCH_FILTER_DESCRIPTORS);

    let over = descriptors_from_wit(
        (0..=MAX_SEARCH_FILTER_DESCRIPTORS)
            .map(|i| descriptor(&format!("k{i}"), 1))
            .collect(),
        &test_origin(),
    );
    assert_eq!(over.len(), MAX_SEARCH_FILTER_DESCRIPTORS);
    assert_eq!(
        over.first().map(|d| d.key.as_str()),
        Some("k0"),
        "the first descriptors are the ones kept"
    );
    let (target, msg) = captured_entry_containing(
        &logs,
        &format!("declared {} descriptors", MAX_SEARCH_FILTER_DESCRIPTORS + 1),
    );
    assert_eq!(target, TEST_LOG_TARGET);
    assert!(
        msg.contains(&MAX_SEARCH_FILTER_DESCRIPTORS.to_string()),
        "{msg}"
    );
}

/// Exactly `MAX_SEARCH_FILTER_VALUES` allowed values survive; one more is
/// cut back to the bound (first ones kept) with a warning naming the key.
#[test]
fn allowed_values_are_capped_at_the_bound_inclusive() {
    let logs = captured_logs();
    let at = descriptor_from_wit(descriptor("at", MAX_SEARCH_FILTER_VALUES), &test_origin())
        .expect("a full list is within the bound");
    assert_eq!(at.allowed_values.len(), MAX_SEARCH_FILTER_VALUES);

    let over = descriptor_from_wit(
        descriptor("over", MAX_SEARCH_FILTER_VALUES + 1),
        &test_origin(),
    )
    .expect("an over-long list is cut, not refused");
    assert_eq!(over.allowed_values.len(), MAX_SEARCH_FILTER_VALUES);
    assert_eq!(
        over.allowed_values.first().map(|v| v.value.as_str()),
        Some("v0")
    );
    let (target, msg) = captured_entry_containing(
        &logs,
        &format!(
            "filter 'over' declared {} allowed values",
            MAX_SEARCH_FILTER_VALUES + 1
        ),
    );
    assert_eq!(target, TEST_LOG_TARGET);
    assert!(
        msg.contains(&format!("kept {MAX_SEARCH_FILTER_VALUES}")),
        "{msg}"
    );
}

/// A key of exactly `MAX_SEARCH_FILTER_STRING_BYTES` is accepted; one
/// byte more drops the whole descriptor. Same bound, applied per field:
/// the display name and the default are checked the same way.
#[test]
fn descriptor_strings_are_bounded_inclusive_per_field() {
    let logs = captured_logs();
    let at = "k".repeat(MAX_SEARCH_FILTER_STRING_BYTES);
    let over = "k".repeat(MAX_SEARCH_FILTER_STRING_BYTES + 1);
    assert!(descriptor_from_wit(descriptor(&at, 1), &test_origin()).is_some());
    assert!(descriptor_from_wit(descriptor(&over, 1), &test_origin()).is_none());

    let mut long_name = descriptor("k", 1);
    long_name.display_name = over.clone();
    assert!(descriptor_from_wit(long_name, &test_origin()).is_none());

    let mut long_default = descriptor("k", 1);
    long_default.default = Some(over);
    assert!(descriptor_from_wit(long_default, &test_origin()).is_none());

    let (target, _msg) = captured_entry_containing(
        &logs,
        "key, display name, or default is over the byte-length bound",
    );
    assert_eq!(target, TEST_LOG_TARGET);
}

/// A control character (here an ESC, the terminal-escape vector) in any
/// descriptor string is refused the same way an over-long one is: in the
/// key, display name or default it drops the descriptor; in an allowed
/// value it drops that value.
#[test]
fn descriptor_strings_with_control_characters_are_refused() {
    let logs = captured_logs();
    let esc = "sort\x1b[31m";
    for field in ["key", "display_name", "default"] {
        let mut d = descriptor("k", 1);
        match field {
            "key" => d.key = esc.into(),
            "display_name" => d.display_name = esc.into(),
            _ => d.default = Some(esc.into()),
        }
        assert!(
            descriptor_from_wit(d, &test_origin()).is_none(),
            "a control character in {field} must drop the descriptor"
        );
    }
    let mut d = descriptor("k", 2);
    d.allowed_values.push(esc.into());
    let converted = descriptor_from_wit(d, &test_origin()).expect("descriptor survives");
    assert_eq!(
        converted
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect::<Vec<_>>(),
        ["v0", "v1"]
    );
    let (target, msg) = captured_entry_containing(&logs, "control character");
    assert_eq!(target, TEST_LOG_TARGET);
    assert!(
        !msg.contains('\x1b'),
        "the offending text must not be echoed: {msg:?}"
    );
}

/// Each dropped allowed value is counted under its actual reason: two
/// over-long values positioned AFTER the 256 kept ones are reported as
/// over-long, not as "past the count bound".
#[test]
fn dropped_allowed_values_are_counted_by_their_actual_reason() {
    let logs = captured_logs();
    let mut d = descriptor("reasons", MAX_SEARCH_FILTER_VALUES);
    d.allowed_values
        .push("x".repeat(MAX_SEARCH_FILTER_STRING_BYTES + 1));
    d.allowed_values
        .push("y".repeat(MAX_SEARCH_FILTER_STRING_BYTES + 1));
    d.allowed_values.push("fine".into());
    let converted = descriptor_from_wit(d, &test_origin()).expect("descriptor survives");
    assert_eq!(converted.allowed_values.len(), MAX_SEARCH_FILTER_VALUES);
    let (_, msg) = captured_entry_containing(&logs, "filter 'reasons'");
    assert!(
        msg.contains("2 over 256 bytes") && msg.contains("1 past the 256-value bound"),
        "{msg}"
    );
}

/// An over-long allowed value drops only itself, not the descriptor.
#[test]
fn an_over_long_allowed_value_is_dropped_from_its_descriptor() {
    let mut d = descriptor("k", 2);
    d.allowed_values
        .push("x".repeat(MAX_SEARCH_FILTER_STRING_BYTES + 1));
    let converted = descriptor_from_wit(d, &test_origin()).expect("descriptor survives");
    assert_eq!(
        converted
            .allowed_values
            .iter()
            .map(|v| v.value.as_str())
            .collect::<Vec<_>>(),
        ["v0", "v1"]
    );
}

// ── the by-name export call against minimal components (L1 / m7) ─────────

/// A minimal component exporting only `search-filters`, instantiated on a
/// bare linker (no host world, no capabilities) into a store carrying the
/// unit-test plugin name — the smallest thing `call_search_filters` can be
/// pointed at.
async fn instantiate(wat: &str) -> (Store<PluginStoreData>, wasmtime::component::Instance) {
    use crate::engine::{Engine, EngineConfig};
    let engine = Engine::new(EngineConfig::default()).expect("engine");
    let component =
        wasmtime::component::Component::new(engine.raw(), wat::parse_str(wat).expect("wat"))
            .expect("component");
    let mut store = crate::instance::build_store(
        &engine,
        "test",
        tokio_util::sync::CancellationToken::new(),
        u64::MAX,
    );
    let instance = wasmtime::component::Linker::<PluginStoreData>::new(engine.raw())
        .instantiate_async(&mut store, &component)
        .await
        .expect("instantiate");
    (store, instance)
}

/// `search-filters` returning one descriptor, laid out by hand in core
/// memory in canonical-ABI order: `key`, `display-name`, `allowed-values`
/// (two strings), `default = some("newest")`. The core function returns
/// the address of the `(ptr, len)` return area for the list. The record
/// type is exported under a name first because the component model only
/// lets an exported function's signature reference named (exported) record
/// types — a purely local record is "not valid to be used as export".
const SEARCH_FILTERS_ONE_DESCRIPTOR_WAT: &str = r#"(component
  (core module $m
    (memory (export "mem") 1)
    (data (i32.const 100) "ordering")
    (data (i32.const 108) "Ordering")
    (data (i32.const 116) "newest")
    (data (i32.const 122) "views")
    (data (i32.const 128) "\74\00\00\00\06\00\00\00\7a\00\00\00\05\00\00\00")
    (data (i32.const 144) "\64\00\00\00\08\00\00\00\6c\00\00\00\08\00\00\00\80\00\00\00\02\00\00\00\01\00\00\00\74\00\00\00\06\00\00\00")
    (data (i32.const 180) "\90\00\00\00\01\00\00\00")
    (func (export "search-filters") (result i32) (i32.const 180))
  )
  (core instance $i (instantiate $m))
  (type $desc (record
    (field "key" string)
    (field "display-name" string)
    (field "allowed-values" (list string))
    (field "default" (option string))))
  (export $desc_x "search-filter-descriptor" (type $desc))
  (func (export "search-filters") (result (list $desc_x))
    (canon lift (core func $i "search-filters") (memory $i "mem") string-encoding=utf8))
)"#;

/// `search-filters` exported with the wrong type (`u32`, not
/// `list<search-filter-descriptor>`).
const SEARCH_FILTERS_WRONG_TYPE_WAT: &str = r#"(component
  (core module $m
    (func (export "search-filters") (result i32) (i32.const 7))
  )
  (core instance $i (instantiate $m))
  (func (export "search-filters") (result u32)
    (canon lift (core func $i "search-filters")))
)"#;

/// The positive half of the drift guard: a component whose export carries
/// exactly the record `wit/types.wit` declares is accepted by the typed
/// by-name call and lifts field-for-field — so the hand-written
/// `WitSearchFilterDescriptor` is proven against the canonical ABI, not
/// only against the record's source text. Calling a second time proves the
/// first call's `post_return` ran: wasmtime refuses a call while one is
/// outstanding.
#[tokio::test]
async fn a_correctly_typed_export_lifts_and_post_returns() {
    let (mut store, instance) = instantiate(SEARCH_FILTERS_ONE_DESCRIPTOR_WAT).await;
    let first = call_search_filters(&mut store, &instance)
        .await
        .expect("typed call succeeds");
    let [d] = first.as_slice() else {
        panic!("one descriptor expected: {first:?}")
    };
    assert_eq!(d.key, "ordering");
    assert_eq!(d.display_name, "Ordering");
    assert_eq!(d.default.as_deref(), Some("newest"));
    assert_eq!(
        d.allowed_values,
        SearchFilterValue::list([("newest", "newest"), ("views", "views")])
    );

    let second = call_search_filters(&mut store, &instance)
        .await
        .expect("a second call is only possible once post_return ran");
    assert_eq!(second, first);
}

/// A `search-filters` export with the wrong signature is a `Trapped` (the
/// typed lookup refuses it before any wasm runs) and therefore a strike —
/// not an absent export, which would silently mean "no filters".
#[tokio::test]
async fn a_mis_typed_export_is_a_trap_and_a_strike() {
    use crate::adapter::counts_as_strike;
    let (mut store, instance) = instantiate(SEARCH_FILTERS_WRONG_TYPE_WAT).await;
    let err = call_search_filters(&mut store, &instance)
        .await
        .expect_err("wrong signature");
    match &err {
        PluginError::Trapped { plugin, reason } => {
            assert_eq!(plugin, "test");
            assert!(
                reason.starts_with("signature of search-filters:"),
                "{reason}"
            );
        }
        other => panic!("expected Trapped, got {other:?}"),
    }
    assert!(
        counts_as_strike(&err),
        "a mis-typed export must count against the plugin"
    );
}

/// A component with no `search-filters` export at all answers no filters,
/// with no error — the pre-0.5.1 path, pinned at this level too.
#[tokio::test]
async fn an_absent_export_answers_no_filters() {
    let (mut store, instance) = instantiate("(component)").await;
    let filters = call_search_filters(&mut store, &instance)
        .await
        .expect("absent export is Ok");
    assert!(filters.is_empty());
}
