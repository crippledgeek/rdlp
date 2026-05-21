//! CLI `rdlp plugin` subcommand tests.
//!
//! Focus: name-validation gates, disable→enable round-trip, and the path-
//! traversal vector in `run_uninstall` (which used to do `dir.join(name)`
//! → `remove_dir_all` on raw user input).

// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs
)]

use rdlp_plugin::disabled_list::read_disabled_list;
use rdlp_plugin::manifest::validate_plugin_name;
use rdlp_types::Config;

/// Run `f` with XDG_CONFIG_HOME and HOME both pointing at a fresh tempdir,
/// then pass the tempdir path into the closure.
///
/// temp-env's internal mutex serialises against any other test that reads or
/// writes these vars — no per-test `OnceLock<Mutex<()>>` guard is needed.
fn with_isolated_xdg<F: FnOnce(&std::path::Path)>(f: F) {
    let tmp = tempfile::TempDir::new().expect("tempdir creation");
    let path_str = tmp.path().to_str().expect("tempdir path is utf-8");
    temp_env::with_vars(
        [
            ("XDG_CONFIG_HOME", Some(path_str)),
            ("HOME", Some(path_str)),
        ],
        || f(tmp.path()),
    );
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

#[test]
fn disable_then_enable_round_trips() {
    with_isolated_xdg(|tmpdir| {
        // Each test sets its own tempdir and observes only its own state — no
        // cross-test dependency on the env var value.
        let _ = tmpdir; // XDG vars already set by with_isolated_xdg

        rdlp_cli::plugin_cmd::run_disable("plugin-a").unwrap();
        rdlp_cli::plugin_cmd::run_disable("plugin-b").unwrap();
        let path = rdlp_cli::plugin_cmd::disabled_list_path().unwrap();
        let list = read_disabled_list(&path).expect("read disabled list");
        assert!(list.contains(&"plugin-a".to_string()));
        assert!(list.contains(&"plugin-b".to_string()));

        rdlp_cli::plugin_cmd::run_enable("plugin-a").unwrap();
        let list = read_disabled_list(&path).expect("read disabled list");
        assert!(!list.contains(&"plugin-a".to_string()));
        assert!(list.contains(&"plugin-b".to_string()));
    });
}

#[test]
fn run_uninstall_rejects_path_traversal_name() {
    with_isolated_xdg(|tmpdir| {
        let plugin_dir = tmpdir.join("plugins");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let config = Config {
            progress: false,
            plugin_directories: vec![plugin_dir],
            ..Default::default()
        };

        // Marker file we will assert is NOT touched by a malicious name.
        let canary = tmpdir.join("canary.txt");
        std::fs::write(&canary, "must survive").unwrap();

        let err = rdlp_cli::plugin_cmd::run_uninstall("../canary.txt", &config)
            .expect_err("uninstall must reject traversal name");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("invalid plugin name"),
            "expected name-validation error, got: {msg}"
        );
        assert!(canary.exists(), "canary file MUST NOT be deleted");
    });
}

#[test]
fn run_disable_rejects_invalid_name() {
    with_isolated_xdg(|_tmpdir| {
        for bad in ["..", "/abs", "Capital", "a b"] {
            let err = rdlp_cli::plugin_cmd::run_disable(bad)
                .expect_err(&format!("disable must reject {bad:?}"));
            let msg = format!("{err:#}");
            assert!(msg.contains("invalid plugin name"));
        }
    });
}
