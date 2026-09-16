use super::*;
use crate::test_harness::{EMPTY_COMPONENT_WAT, instantiate};
use crate::test_support::extraction_ctx;
use crate::test_support::unit::fixture_extractor;
use std::sync::atomic::AtomicBool;

// ── `call_export_by_name`: the mechanism every optional export shares ────

/// An export `probe` whose type is not what a caller asks for (`u32`,
/// asked for as a `string`). The one wrong-typed fixture for the one
/// typecheck path; each adapter's own tests keep only what "absent" means
/// to them.
const WRONG_TYPE_EXPORT_WAT: &str = r#"(component
  (core module $m
    (func (export "probe") (result i32) (i32.const 7))
  )
  (core instance $i (instantiate $m))
  (func (export "probe") (result u32)
    (canon lift (core func $i "probe")))
)"#;

/// A component that never declared the export is `Ok(None)`, logged once
/// at debug — the pre-export path for every optional export.
#[tokio::test]
async fn by_name_absent_export_is_ok_none() {
    let (mut store, inst) = instantiate(EMPTY_COMPONENT_WAT).await;
    let r = call_export_by_name::<(), (u32,)>(
        &mut store,
        &inst,
        ExportCall {
            name: "probe",
            params: (),
        },
    )
    .await
    .expect("absent is not an error");
    assert!(r.is_none());
}

/// An export present under the name but with another signature is a
/// `Trapped` (the typed lookup refuses it before any wasm runs) naming
/// the export, and therefore a strike — never mistaken for absence.
#[tokio::test]
async fn by_name_mis_typed_export_is_a_trap_and_a_strike() {
    let (mut store, inst) = instantiate(WRONG_TYPE_EXPORT_WAT).await;
    let err = call_export_by_name::<(), (String,)>(
        &mut store,
        &inst,
        ExportCall {
            name: "probe",
            params: (),
        },
    )
    .await
    .expect_err("wrong signature");
    match &err {
        PluginError::Trapped { plugin, reason } => {
            assert_eq!(plugin, "test");
            assert!(reason.starts_with("signature of probe:"), "{reason}");
        }
        other => panic!("expected Trapped, got {other:?}"),
    }
    assert!(
        counts_as_strike(&err),
        "a mis-typed export must count against the plugin"
    );
}

fn spec(timeout: Duration) -> CallSpec<'static> {
    CallSpec {
        subject_for_errors: "https://example.com/video/42",
        timeout,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_counts_an_internal_error_as_a_strike() {
    let ext = fixture_extractor();
    let err = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
            Box::pin(async { Err::<(), _>(PluginError::Internal("boom".into())) })
        })
        .await
        .expect_err("closure error propagates");
    assert!(matches!(err, PluginError::Internal(_)), "got {err:?}");
    assert_eq!(ext.test_trap_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_counts_a_trap_as_a_strike() {
    let ext = fixture_extractor();
    let err = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
            Box::pin(async {
                Err::<(), _>(PluginError::Trapped {
                    plugin: "example".into(),
                    reason: "unreachable".into(),
                })
            })
        })
        .await
        .expect_err("closure error propagates");
    assert!(matches!(err, PluginError::Trapped { .. }), "got {err:?}");
    assert_eq!(ext.test_trap_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_counts_a_linker_wire_error_as_a_strike() {
    let ext = fixture_extractor();
    let err = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
            Box::pin(async {
                Err::<(), _>(PluginError::LinkerWire {
                    plugin: "example".into(),
                    reason: "missing import".into(),
                })
            })
        })
        .await
        .expect_err("closure error propagates");
    assert!(matches!(err, PluginError::LinkerWire { .. }), "got {err:?}");
    assert_eq!(ext.test_trap_count(), 1);
}

/// Moved here from the former `tests/adapter_trap_disable.rs`, which
/// returned early whenever its gitignored wasip1 artefact was absent
/// and so proved nothing on a fresh checkout.
#[test]
fn third_trap_disables_plugin() {
    let ext = fixture_extractor();
    assert!(!ext.test_is_disabled());
    assert_eq!(ext.test_trap_count(), 0);
    ext.test_record_trap();
    assert!(!ext.test_is_disabled(), "1 strike does not disable");
    ext.test_record_trap();
    assert!(!ext.test_is_disabled(), "2 strikes does not disable");
    ext.test_record_trap();
    assert!(ext.test_is_disabled(), "3 strikes MUST disable the adapter");
    assert_eq!(ext.test_trap_count(), TRAP_DISABLE_THRESHOLD);
}

#[test]
fn additional_traps_after_disable_are_idempotent() {
    let ext = fixture_extractor();
    for _ in 0..TRAP_DISABLE_THRESHOLD + 2 {
        ext.test_record_trap();
    }
    // The counter keeps climbing; the disabled flag stays latched.
    assert!(ext.test_is_disabled());
    assert_eq!(ext.test_trap_count(), TRAP_DISABLE_THRESHOLD + 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_does_not_count_a_domain_error_as_a_strike() {
    let ext = fixture_extractor();
    let err = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |_store, _inst| {
            Box::pin(async {
                Err::<(), _>(PluginError::NotFound {
                    plugin: "example".into(),
                    detail: "gone".into(),
                })
            })
        })
        .await
        .expect_err("closure error propagates");
    assert!(matches!(err, PluginError::NotFound { .. }), "got {err:?}");
    assert_eq!(ext.test_trap_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_hands_the_closure_a_live_instance() {
    let ext = fixture_extractor();
    let title = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |store, inst| {
            Box::pin(async move {
                let info = call_plugin_extract(store, inst, "https://example.com/video/42").await?;
                Ok(info.title)
            })
        })
        .await
        .expect("extract via runner");
    assert_eq!(title, "Example Video 42");
    assert_eq!(ext.test_trap_count(), 0);
}

/// Task 6: `call_plugin_extract` tries `extract-with-metadata` first; on
/// the committed 0.5.0 fixture (which predates that export) it must fall
/// back to the typed `extract` path unchanged — same title, no strike.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_on_a_0_5_0_component_still_uses_extract() {
    let ext = fixture_extractor();
    let info = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |store, inst| {
            Box::pin(async move {
                call_plugin_extract(store, inst, "https://example.com/video/42").await
            })
        })
        .await
        .expect("falls back to extract when extract-with-metadata is absent");
    assert_eq!(info.title, "Example Video 42");
    assert_eq!(ext.test_trap_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_timeout_is_a_strike_and_cancels_the_call() {
    let ext = fixture_extractor();
    let seen_cancel = Arc::new(std::sync::Mutex::new(None));
    let sink = seen_cancel.clone();
    let err = ext
        .run_in_fresh_store(spec(Duration::from_millis(50)), move |store, _inst| {
            *sink.lock().expect("mutex") = Some(store.data().cancel.clone());
            Box::pin(std::future::pending::<Result<(), PluginError>>())
        })
        .await
        .expect_err("deadline elapses");
    assert!(matches!(err, PluginError::Timeout { .. }), "got {err:?}");
    assert_eq!(ext.test_trap_count(), 1);
    let token = seen_cancel
        .lock()
        .expect("mutex")
        .clone()
        .expect("closure ran");
    assert!(token.is_cancelled(), "timeout must trip the per-call token");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runner_refuses_a_disabled_plugin_without_running_the_closure() {
    let ext = fixture_extractor();
    for _ in 0..TRAP_DISABLE_THRESHOLD {
        ext.test_record_trap();
    }
    assert!(ext.test_is_disabled());
    let ran = Arc::new(AtomicBool::new(false));
    let flag = ran.clone();
    let err = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), move |_store, _inst| {
            flag.store(true, Ordering::Relaxed);
            Box::pin(async { Ok::<(), _>(()) })
        })
        .await
        .expect_err("disabled plugin refuses calls");
    assert!(matches!(err, PluginError::Disabled { .. }), "got {err:?}");
    assert!(!ran.load(Ordering::Relaxed), "closure must not run");
    // Refusal is not itself a strike.
    assert_eq!(ext.test_trap_count(), TRAP_DISABLE_THRESHOLD);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_goes_through_the_runner() {
    let ext = fixture_extractor();
    let info = ext
        .extract("https://example.com/video/42", &extraction_ctx())
        .await
        .expect("extract");
    assert_eq!(info.title, "Example Video 42");

    // A domain error from the plugin is not a strike.
    let err = ext
        .extract("https://example.com/video/abc", &extraction_ctx())
        .await
        .expect_err("non-numeric id is a parse error");
    assert!(err.to_string().contains("parse error"), "got {err}");
    assert_eq!(ext.test_trap_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_on_a_disabled_plugin_reports_disabled() {
    let ext = fixture_extractor();
    for _ in 0..TRAP_DISABLE_THRESHOLD {
        ext.test_record_trap();
    }
    let err = ext
        .extract("https://example.com/video/42", &extraction_ctx())
        .await
        .expect_err("disabled");
    assert!(err.to_string().contains("is disabled"), "got {err}");
}

#[test]
fn extract_error_mapping_keeps_the_plugin_name_on_internal() {
    use crate::bindings::rdlp::plugin::types::ExtractError as W;
    match extract_error_to_plugin_error("example", W::Internal("bad".into())) {
        PluginError::Internal(msg) => assert_eq!(msg, "plugin example: bad"),
        other => panic!("got {other:?}"),
    }
    assert!(matches!(
        extract_error_to_plugin_error("example", W::RateLimited(Some(7))),
        PluginError::RateLimited {
            retry_after: Some(7),
            ..
        }
    ));
    assert!(matches!(
        extract_error_to_plugin_error("example", W::Cancelled),
        PluginError::Cancelled { .. }
    ));
}

/// `call_plugin_extract`'s `Some(Err(_))` arm — a
/// plugin that answers `extract-with-metadata` with a domain error must
/// map through `extract_error_to_plugin_error` and short-circuit, never
/// silently fall through to the typed `extract` path. Exercises
/// `route_metadata_extraction` directly (no wasm component needed — see
/// its own doc comment for why) rather than `call_plugin_extract` end to
/// end, because reaching that arm through a real component would require
/// hand-encoding a WAT component satisfying the ENTIRE
/// `extractor-plugin-host` world (metadata + extract + search exports,
/// plus every nested `info-dict`/`search-page` type) merely to reach one
/// `extract-with-metadata` error case — `call_extract_with_metadata`'s own
/// tests already cover the wasm-boundary mechanics this function's input
/// comes from.
#[test]
fn some_err_from_metadata_short_circuits_without_falling_through() {
    use crate::bindings::rdlp::plugin::types::ExtractError as W;
    use crate::convert::ExtractionSite;
    use crate::metadata_adapter::MetadataCaps;
    use crate::test_support::unit::test_origin;

    let caps = MetadataCaps::default();
    let site = ExtractionSite {
        url: "https://example.com/video/42",
        origin: test_origin(),
        caps: &caps,
    };
    let routed = route_metadata_extraction(
        Some(Err(W::UnsupportedUrl("nope".into()))),
        "example",
        &site,
    )
    .expect("Some(Err(_)) must short-circuit, not fall through as None");
    let err = routed.expect_err("a domain error stays an error");
    assert!(
        matches!(&err, PluginError::UnsupportedUrl { plugin, detail }
            if plugin == "example" && detail == "nope"),
        "got {err:?}"
    );
    assert!(
        !counts_as_strike(&err),
        "a declined URL is a domain outcome, not a strike"
    );
}

/// The fixture manifest carrying `display_name`.
fn fixture_manifest_with_display_name(display_name: &str) -> String {
    crate::test_support::unit::fixture_manifest_with(&format!("display_name = \"{display_name}\""))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extractor_field_and_name_use_display_name() {
    use crate::test_support::unit::fixture_extractor_with_manifest;

    let toml = fixture_manifest_with_display_name("Example Site");
    let ext = fixture_extractor_with_manifest(&toml);
    assert_eq!(ext.name(), "Example Site");

    let info = ext
        .extract("https://example.com/video/42", &extraction_ctx())
        .await
        .expect("extract");
    assert_eq!(info.extractor, "Example Site");
    // The identity key travels beside the display name so the archive
    // token (`rdlp_api::orchestrator::archive::archive_token_for`) is the
    // manifest `name`, never the display surface.
    assert_eq!(info.extractor_key.as_deref(), Some("example"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn display_name_does_not_leak_into_store_identity_or_log_target() {
    use crate::test_support::unit::fixture_extractor_with_manifest;

    let toml = fixture_manifest_with_display_name("Example Site");
    let ext = fixture_extractor_with_manifest(&toml);

    // `InfoDict::extractor` is the display surface and must show the
    // manifest's `display_name`.
    let info = ext
        .extract("https://example.com/video/42", &extraction_ctx())
        .await
        .expect("extract");
    assert_eq!(info.extractor, "Example Site");

    // `PluginStoreData::plugin_name`/`log_target` are the identity surface
    // (strike-log lines, `host:log` target) and must stay keyed on
    // `manifest.name`, never on the divergent `display_name`.
    let (plugin_name, log_target) = ext
        .run_in_fresh_store(spec(EXTRACT_TIMEOUT), |store, _inst| {
            let observed = (
                store.data().plugin_name.clone(),
                store.data().log_target.clone(),
            );
            Box::pin(async move { Ok::<_, PluginError>(observed) })
        })
        .await
        .expect("run_in_fresh_store");
    assert_eq!(plugin_name, "example");
    assert_eq!(log_target, "plugin::example");
}

#[test]
fn search_site_routing_still_uses_name() {
    use crate::test_support::unit::fixture_extractor_with_manifest;

    let toml = fixture_manifest_with_display_name("Example Site");
    let ext = fixture_extractor_with_manifest(&toml);
    // `FIXTURE_MANIFEST` declares `name = "example"`; display_name is
    // display-only and must not shift routing/identity, which stays on
    // `search_site_name()` (== `name` here, since no `search_site` override).
    assert_eq!(ext.manifest.search_site_name(), "example");
    assert_eq!(ext.manifest.name, "example");
}
