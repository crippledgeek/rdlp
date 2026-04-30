//! Slice-1 integration test: `rdlp plugin build-from-ytdlp` produces a valid
//! .wasm + .plugin.toml.template from a minimal yt-dlp-style .py file.

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

use std::process::Command;
use tempfile::TempDir;

fn rdlp_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_rdlp"))
}

#[test]
#[ignore = "requires tools/ytdlp-compat/.venv populated; takes ~30s (componentize-py)"]
fn build_from_ytdlp_produces_wasm_and_manifest() {
    let tmp = TempDir::new().unwrap();
    let py = tmp.path().join("dummy.py");
    std::fs::write(
        &py,
        r#"
from rdlp_ytdlp_compat import InfoExtractor

class DummyIE(InfoExtractor):
    _VALID_URL = r'https?://example\.com/v/(?P<id>\d+)'

    def _real_extract(self, url):
        m = self._search_regex(self._VALID_URL, url, "id", group="id")
        return {"id": m, "title": f"Dummy {m}", "formats": []}
"#,
    )
    .unwrap();

    let output = Command::new(rdlp_bin())
        .args(["plugin", "build-from-ytdlp"])
        .arg(&py)
        .output()
        .expect("run rdlp");

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let wasm = tmp.path().join("dummy/plugin.wasm");
    let toml = tmp.path().join("dummy/plugin.toml.template");
    assert!(wasm.exists(), "wasm not produced: {wasm:?}");
    assert!(toml.exists(), "manifest template not produced: {toml:?}");

    let manifest = std::fs::read_to_string(&toml).unwrap();
    assert!(
        manifest.contains("name = \"dummy\""),
        "missing name in manifest"
    );
    assert!(
        manifest.contains("https://example.com/*"),
        "missing match pattern"
    );
    assert!(
        manifest.contains("[signature]"),
        "missing signature placeholder block"
    );
    assert!(
        !manifest.contains("[wasm]"),
        "manifest has invalid [wasm] table"
    );
}
