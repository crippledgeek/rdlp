//! Slice-1 golden: build 3 synthetic yt-dlp-shape extractors via
//! `rdlp plugin build-from-ytdlp` and assert that each produces a valid
//! .wasm + plugin.toml.template.
//!
//! Extract dispatch (executing the plugin against canned HTML) requires a
//! host-side fixture-injection harness that mocks `host:fetch` — deferred
//! to Slice 2 per the plan.

use std::path::PathBuf;
use std::process::Command;

const GOLDENS: &[&str] = &["simple_html", "json_traversal", "m3u8_with_fallback"];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
#[ignore = "slow: builds 3 ~35MB wasm components via componentize-py (~30s each)"]
#[allow(clippy::disallowed_methods)] // test fixture — sync I/O is acceptable per clippy.toml
fn ytdlp_goldens_build_and_emit_artefacts() {
    let root = workspace_root();
    for name in GOLDENS {
        let py = root.join(format!("examples/plugins/ytdlp-golden/{name}.py"));
        assert!(py.exists(), "source missing: {py:?}");

        // build-from-ytdlp normalises Python snake_case filenames to
        // kebab-case plugin names (manifest validation). The output dir
        // therefore uses the normalised stem, not the raw filename.
        let normalised = name.to_ascii_lowercase().replace('_', "-");
        let plugin_dir = py.parent().unwrap().join(&normalised);
        // Clean any prior build output
        let _ = std::fs::remove_dir_all(&plugin_dir);

        let status = Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "-p",
                "rdlp-cli",
                "--",
                "plugin",
                "build-from-ytdlp",
            ])
            .arg(&py)
            .current_dir(&root)
            .status()
            .expect("invoke rdlp build-from-ytdlp");
        assert!(status.success(), "build-from-ytdlp failed for {name}");

        let wasm = plugin_dir.join("plugin.wasm");
        let toml = plugin_dir.join("plugin.toml.template");
        assert!(wasm.exists(), "wasm missing for {name}: {wasm:?}");
        assert!(toml.exists(), "manifest missing for {name}: {toml:?}");

        let metadata = std::fs::metadata(&wasm).unwrap();
        // Sanity: must be a real CPython component (~30+ MB).
        assert!(
            metadata.len() > 30_000_000,
            "{name} wasm too small ({} bytes) — bindgen likely failed silently",
            metadata.len()
        );

        let manifest = std::fs::read_to_string(&toml).unwrap();
        // Schema-correctness — no [wasm] table; has [signature] placeholder.
        assert!(
            !manifest.contains("[wasm]"),
            "{name} has invalid [wasm] table"
        );
        assert!(
            manifest.contains("[signature]"),
            "{name} missing signature block"
        );
        assert!(
            manifest.contains("type = \"ed25519\""),
            "{name} missing ed25519 type"
        );
        // Capability vocab unprefixed.
        assert!(
            manifest.contains("\"fetch\""),
            "{name} missing fetch capability"
        );
        assert!(
            !manifest.contains("\"host:fetch\""),
            "{name} uses prefixed capability vocab"
        );
    }
}
