use super::*;
use crate::test_support::extraction_ctx;
use crate::test_support::unit::fixture_extractor;
use std::sync::atomic::AtomicBool;

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
