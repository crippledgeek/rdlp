//! Test-support seams shared by this crate's unit tests, its `tests/`
//! integration binaries, and — for the signer — other crates' integration
//! tests (`rdlp-api`'s bootstrap tests need to fabricate a loadable signed
//! plugin the same way this crate's own tests do, and a second hand-rolled
//! copy of the signing mechanism is exactly what
//! `extract-before-you-duplicate` forbids).
//!
//! Compiled into the library (not `cfg(test)`) because an integration
//! test binary links the library as an external crate and cannot see its
//! `cfg(test)` items. Hidden from docs like the `test_*` accessors on
//! `PluginExtractor`, which follow the same precedent.

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use rdlp_core::ExtractionContext;

/// The default extraction context a plugin test hands to `extract` /
/// `search`: a stock HTTP client, the boa engine, an empty cookie jar and
/// default config. Formerly six identical copies (the adapter unit tests
/// and five integration binaries).
#[doc(hidden)]
#[must_use]
pub fn extraction_ctx() -> ExtractionContext {
    ExtractionContext::new(
        Arc::new(rdlp_http::HttpClientFactory::default().build()),
        Arc::new(rdlp_jsinterp::BoaJsEngine::new()),
        Arc::new(rdlp_cookies::SimpleCookieJar::new()),
        Arc::new(rdlp_types::Config::default()),
    )
}

/// Every manifest field this crate's (and `rdlp-api`'s) integration tests
/// vary across their signed-plugin fixtures. `dir` and `key` stay as
/// [`write_signed_plugin`]'s own parameters (they answer "where" and "who
/// signs", not "what manifest").
#[doc(hidden)]
pub struct SignedPluginSpec<'a> {
    /// Plugin name — always varies (one per plugin fixture).
    pub name: &'a str,
    /// Plugin semver version.
    pub version: &'a str,
    /// Target WIT contract version this manifest declares.
    pub wit_version: &'a str,
    /// Chrome-style match patterns.
    pub matches: &'a [&'a str],
    /// Plugin priority within the band 100..=199.
    pub priority: u32,
    /// Hostnames this plugin claims the right to shadow from a built-in.
    pub claims_override: &'a [&'a str],
    /// Host capabilities the plugin requests.
    pub capabilities: &'a [&'a str],
    /// Whether the manifest declares `supports_extract`. Defaults to
    /// `true` (matching every manifest written before the field existed)
    /// when a caller has no reason to vary it.
    pub supports_extract: bool,
    /// Whether the manifest declares `supports_search`.
    pub supports_search: bool,
    /// The compiled component bytes to sign and write.
    pub wasm: &'a [u8],
}

/// Write a signed `plugin.toml` + `plugin.wasm` into `dir` so
/// `Loader::discover` accepts it. The one signing mechanism every plugin
/// integration test across the workspace shares.
#[doc(hidden)]
pub fn write_signed_plugin(dir: &Path, key: &SigningKey, spec: &SignedPluginSpec<'_>) {
    // Test fixture — sync I/O is acceptable per clippy.toml's disallowed-methods
    // carve-out (c); every caller is a `#[test]`/`#[tokio::test]` setup step,
    // not a hot async path.
    #[allow(clippy::disallowed_methods)]
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create plugin dir: {e}"));
    #[allow(clippy::disallowed_methods)]
    std::fs::write(dir.join("plugin.wasm"), spec.wasm)
        .unwrap_or_else(|e| panic!("write plugin.wasm: {e}"));

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
supports_search = {supports_search}
supports_extract = {supports_extract}
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
        supports_search = spec.supports_search,
        supports_extract = spec.supports_extract,
    );

    let m = crate::manifest::parse_manifest_str(&toml_placeholder)
        .unwrap_or_else(|e| panic!("parse manifest: {e}"));
    let mut buf = crate::manifest::canonical_bytes(&m);
    buf.extend_from_slice(spec.wasm);
    let sig = key.sign(&buf);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let final_toml = toml_placeholder.replace("PLACEHOLDER", &sig_b64);
    #[allow(clippy::disallowed_methods)]
    std::fs::write(dir.join("plugin.toml"), final_toml)
        .unwrap_or_else(|e| panic!("write plugin.toml: {e}"));
}
