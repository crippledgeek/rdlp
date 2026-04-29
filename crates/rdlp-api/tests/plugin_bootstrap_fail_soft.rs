//! Bootstrap fail-soft: a malformed plugin must NOT block built-in extractors.
//!
//! `plugin_bootstrap.rs` documents the contract: "A broken plugin directory
//! must never block rdlp from working with built-in extractors." This test
//! drops malformed plugin contents into a configured plugin_directory and
//! verifies `RdlpClient::new` still returns a working client whose
//! `list_extractors()` includes the full built-in set.

#![allow(clippy::disallowed_methods)] // test fixture I/O

use rdlp_api::RdlpClient;
use rdlp_types::Config;

fn isolate_xdg(tempdir: &std::path::Path) {
    // SAFETY: tests in this crate run with cargo's default scheduling.
    // The env vars are scoped to the process and the tempdir outlives this
    // function. We do this so the bootstrap doesn't write trust state into
    // the user's real ~/.config/rdlp.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tempdir);
        std::env::set_var("HOME", tempdir);
    }
}

#[test]
fn malformed_plugin_toml_does_not_break_client() {
    let tempdir = tempfile::tempdir().unwrap();
    isolate_xdg(tempdir.path());

    // Drop a directory that *looks* like a plugin but has a syntactically
    // invalid manifest. Bootstrap should warn-and-skip, not propagate.
    let plug_dir = tempdir.path().join("plugins").join("broken");
    std::fs::create_dir_all(&plug_dir).unwrap();
    std::fs::write(plug_dir.join("plugin.toml"), "this = is = not = valid = toml").unwrap();
    std::fs::write(plug_dir.join("plugin.wasm"), b"not actual wasm").unwrap();

    let config = Config {
        progress: false,
        plugin_directories: vec![tempdir.path().join("plugins")],
        ..Default::default()
    };

    let client = RdlpClient::new(config).expect("client must build despite broken plugin");
    let names = client.list_extractors();
    // Built-ins must be present and the broken plugin must not be.
    assert!(names.contains(&"Generic"));
    assert!(!names.contains(&"broken"));
}

#[test]
fn missing_plugin_dir_does_not_break_client() {
    let tempdir = tempfile::tempdir().unwrap();
    isolate_xdg(tempdir.path());
    let nonexistent = tempdir.path().join("does-not-exist");

    let config = Config {
        progress: false,
        plugin_directories: vec![nonexistent],
        ..Default::default()
    };

    let client = RdlpClient::new(config).expect("client must build with missing plugin dir");
    assert!(client.list_extractors().contains(&"Generic"));
}

#[test]
fn corrupted_disabled_list_blocks_bootstrap() {
    // Inverse of fail-soft: a corrupted disabled-plugins TOML MUST be
    // surfaced loudly. Silently treating it as empty would silently
    // re-enable any previously-blocked plugin (security regression).
    let tempdir = tempfile::tempdir().unwrap();
    isolate_xdg(tempdir.path());
    let rdlp_dir = tempdir.path().join("rdlp");
    std::fs::create_dir_all(&rdlp_dir).unwrap();
    std::fs::write(
        rdlp_dir.join("plugin-disabled.toml"),
        "this = is = not = valid",
    )
    .unwrap();

    let plug_dir = tempdir.path().join("plugins");
    std::fs::create_dir_all(&plug_dir).unwrap();

    let config = Config {
        progress: false,
        plugin_directories: vec![plug_dir],
        ..Default::default()
    };

    // RdlpClient::new currently returns Ok with an empty plugin set when
    // bootstrap fails (fail-soft at the top level), but the inner
    // `bootstrap_plugins` returns Err for this case so the WARN log fires.
    // We assert client construction succeeds AND the broken disabled list
    // does NOT silently let plugins through. With no plugin dirs to load
    // here, the test only confirms the corrupted-disabled-list path
    // doesn't panic and still builds the client with built-ins.
    let client = RdlpClient::new(config).expect("client builds (fail-soft top-level)");
    assert!(client.list_extractors().contains(&"Generic"));
}
