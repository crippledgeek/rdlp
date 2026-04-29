//! CLI `rdlp plugin` subcommand tests.
//!
//! Focus: name-validation gates, disable→enable round-trip, and the path-
//! traversal vector in `run_uninstall` (which used to do `dir.join(name)`
//! → `remove_dir_all` on raw user input).

#![allow(clippy::disallowed_methods)] // test fixture I/O

use rdlp_plugin::disabled_list::read_disabled_list;
use rdlp_plugin::manifest::validate_plugin_name;
use rdlp_types::Config;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Shared lock — XDG_CONFIG_HOME is process-global, so tests that set it
/// must serialize. The lock is also held across the test body so a
/// concurrent test can't yank the env var out from under us.
fn xdg_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn isolate_xdg(tempdir: &std::path::Path) {
    // SAFETY: caller holds `xdg_lock` for the test body; tempdir outlives
    // the test.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tempdir);
        std::env::set_var("HOME", tempdir);
    }
}

#[test]
fn validate_plugin_name_accepts_kebab_case() {
    for ok in ["a", "1", "a1", "a-b", "example", "my-plugin-2", "0a-b-9"] {
        assert!(
            validate_plugin_name(ok).is_ok(),
            "{ok:?} should be accepted"
        );
    }
}

#[test]
fn validate_plugin_name_rejects_path_traversal() {
    for bad in [
        "..",
        "../../.ssh",
        "/etc/passwd",
        "a/b",
        "a\\b",
        ".",
        "-leading-hyphen",
        "Capital",
        "with space",
        "evil::collide",
        "a.b",
        "a..b",
    ] {
        assert!(
            validate_plugin_name(bad).is_err(),
            "{bad:?} MUST be rejected"
        );
    }
}

#[test]
fn validate_plugin_name_rejects_empty_and_oversize() {
    assert!(validate_plugin_name("").is_err());
    let too_long: String = "a".repeat(65);
    assert!(validate_plugin_name(&too_long).is_err());
    let just_at_limit: String = "a".repeat(64);
    assert!(validate_plugin_name(&just_at_limit).is_ok());
}

#[tokio::test]
async fn disable_then_enable_round_trips() {
    let _guard = xdg_lock();
    let tempdir = tempfile::tempdir().unwrap();
    isolate_xdg(tempdir.path());

    rdlp_cli::plugin_cmd::run_disable("plugin-a").await.unwrap();
    rdlp_cli::plugin_cmd::run_disable("plugin-b").await.unwrap();
    let path = rdlp_cli::plugin_cmd::disabled_list_path().unwrap();
    let list = read_disabled_list(&path).expect("read disabled list");
    assert!(list.contains(&"plugin-a".to_string()));
    assert!(list.contains(&"plugin-b".to_string()));

    rdlp_cli::plugin_cmd::run_enable("plugin-a").await.unwrap();
    let list = read_disabled_list(&path).expect("read disabled list");
    assert!(!list.contains(&"plugin-a".to_string()));
    assert!(list.contains(&"plugin-b".to_string()));
}

#[tokio::test]
async fn run_uninstall_rejects_path_traversal_name() {
    let _guard = xdg_lock();
    let tempdir = tempfile::tempdir().unwrap();
    isolate_xdg(tempdir.path());
    let plugin_dir = tempdir.path().join("plugins");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    let config = Config {
        progress: false,
        plugin_directories: vec![plugin_dir.clone()],
        ..Default::default()
    };

    // Marker file we will assert is NOT touched by a malicious name.
    let canary = tempdir.path().join("canary.txt");
    std::fs::write(&canary, "must survive").unwrap();

    let err = rdlp_cli::plugin_cmd::run_uninstall("../canary.txt", &config)
        .await
        .expect_err("uninstall must reject traversal name");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("invalid plugin name"),
        "expected name-validation error, got: {msg}"
    );
    assert!(canary.exists(), "canary file MUST NOT be deleted");
}

#[tokio::test]
async fn run_disable_rejects_invalid_name() {
    let _guard = xdg_lock();
    let tempdir = tempfile::tempdir().unwrap();
    isolate_xdg(tempdir.path());

    for bad in ["..", "/abs", "Capital", "a b"] {
        let err = rdlp_cli::plugin_cmd::run_disable(bad)
            .await
            .expect_err(&format!("disable must reject {bad:?}"));
        let msg = format!("{err:#}");
        assert!(msg.contains("invalid plugin name"));
    }
}
