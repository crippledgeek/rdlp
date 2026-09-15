// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at module scope. `missing_docs` exempt
// because integration test helpers aren't public API.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

//! Shared signer for the crate's plugin integration tests.
//!
//! `write_signed_plugin` used to exist as six near-identical copies across
//! five test files (`loader.rs` had two — the WAT-stub signer plus the D1
//! compat tests' `write_signed_real_component_plugin` — alongside one each
//! in `svt_golden.rs`, `mpd_golden.rs`, `xxxymovies_golden.rs`, and
//! `python_plugin_smoke.rs`), each hardcoding a different subset of the
//! manifest fields and passing the rest as parameters. Per
//! `limit-function-arguments` / `extract-before-you-duplicate`: the
//! difference between call sites is a VALUE, never a reason to keep a
//! second copy of the mechanism — so every value any caller varied is a
//! field on [`SignedPluginSpec`], and there is exactly one signing function.

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rdlp_plugin::manifest::canonical_bytes;
use std::path::Path;

/// The one extraction-context builder, shared with the library's own unit
/// tests; re-exported so every integration binary imports it from here
/// alongside the signer.
pub use rdlp_plugin::test_support::extraction_ctx;

/// Every manifest field this crate's integration tests vary across the five
/// former copies of the signer. `dir` and `key` stay as the function's own
/// parameters (they answer "where" and "who signs", not "what manifest") —
/// see `write_signed_plugin`'s 3-parameter signature.
pub struct SignedPluginSpec<'a> {
    /// `name` — always varies (one per plugin fixture).
    pub name: &'a str,
    /// `version` — fixed at `"0.1.0"` in four of the five former copies,
    /// `"1.0.0"` in `loader.rs`'s; kept a field rather than a shared
    /// constant so a caller can pin either without a flag.
    pub version: &'a str,
    /// `wit_version` — fixed at `"0.5.0"` everywhere except the D1 compat
    /// tests in `loader.rs`, which are the reason this varies at all.
    pub wit_version: &'a str,
    /// `matches` — always varies (one match-pattern set per plugin).
    pub matches: &'a [&'a str],
    /// `priority` — `150` everywhere today; kept a field because
    /// `loader.rs`'s original signature already exposed it as one.
    pub priority: u32,
    /// `claims_override` — `&[]` everywhere today; same rationale as
    /// `priority`.
    pub claims_override: &'a [&'a str],
    /// `capabilities` — always varies (exercises capability-creep /
    /// capability-denial paths).
    pub capabilities: &'a [&'a str],
    /// `wasm` — either a `(component)` WAT stub (`loader.rs`'s
    /// non-D1 tests) or a real compiled component.
    pub wasm: &'a [u8],
}

/// Write a signed `plugin.toml` + `plugin.wasm` into `dir` so
/// `Loader::discover` accepts it. The one signing mechanism every
/// integration test in this crate shares.
pub fn write_signed_plugin(dir: &Path, key: &SigningKey, spec: &SignedPluginSpec) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("plugin.wasm"), spec.wasm).unwrap();

    let pubkey_b64 =
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    let cap_str = spec
        .capabilities
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let match_str = spec
        .matches
        .iter()
        .map(|m| format!("\"{m}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let claims_str = spec
        .claims_override
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml_placeholder = format!(
        r#"
name = "{name}"
version = "{version}"
wit_version = "{wit_version}"
matches = [{match_str}]
priority = {priority}
claims_override = [{claims_str}]
capabilities = [{cap_str}]

[signature]
type = "ed25519"
pubkey = "{pubkey_b64}"
signature = "PLACEHOLDER"
"#,
        name = spec.name,
        version = spec.version,
        wit_version = spec.wit_version,
        priority = spec.priority,
    );

    let m = rdlp_plugin::manifest::parse_manifest_str(&toml_placeholder).unwrap();
    let mut buf = canonical_bytes(&m);
    buf.extend_from_slice(spec.wasm);
    let sig = key.sign(&buf);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let final_toml = toml_placeholder.replace("PLACEHOLDER", &sig_b64);
    std::fs::write(dir.join("plugin.toml"), final_toml).unwrap();
}
