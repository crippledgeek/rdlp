//! Plugin manifest (`plugin.toml`) schema and validation.

use crate::error::PluginError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// All capabilities the host can grant. Maintained as a closed set; unknown
/// capabilities in a manifest cause load to fail.
pub const KNOWN_CAPABILITIES: &[&str] = &[
    "fetch",
    "cookie-jar",
    "js-eval",
    "html-select",
    "log",
    "store-kv",
    "claim-all-urls",
];

/// Maximum byte length of a `url_regex` source string before compilation is even attempted.
const URL_REGEX_MAX_BYTES: usize = 2048;

/// Plugin manifest as parsed from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Plugin name (kebab-case, no namespace).
    pub name: String,
    /// Plugin semver version.
    pub version: String,
    /// Target WIT contract version (e.g. "0.1.0").
    pub wit_version: String,
    /// Chrome-style match patterns (mandatory; at least one).
    pub matches: Vec<String>,
    /// Optional fine-grained regex for ID extraction.
    #[serde(default)]
    pub url_regex: Option<String>,
    /// Plugin priority within the band 100..=199.
    pub priority: u32,
    /// Hostnames this plugin shadows from built-ins (red-flagged in first-install prompt).
    #[serde(default)]
    pub claims_override: Vec<String>,
    /// Whether the plugin implements the `search` export.
    #[serde(default)]
    pub supports_search: bool,
    /// Host capabilities the plugin requests (subset of `KNOWN_CAPABILITIES`).
    pub capabilities: Vec<String>,
    /// Signature backing the manifest + plugin.wasm (Sigstore or Ed25519).
    pub signature: Signature,
}

/// Plugin signature variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Signature {
    /// Sigstore keyless signature bound to an OIDC identity.
    Sigstore {
        /// OIDC subject (e.g. "github:user/repo").
        identity: String,
        /// OIDC issuer URL.
        oidc_issuer: String,
        /// Base64-encoded Sigstore bundle (Fulcio cert + signature + Rekor entry).
        bundle: String,
    },
    /// Ed25519 raw signature with embedded pubkey.
    Ed25519 {
        /// Base64-encoded 32-byte Ed25519 public key.
        pubkey: String,
        /// Base64-encoded 64-byte Ed25519 signature over (canonical_bytes(manifest) || wasm_bytes).
        signature: String,
    },
}

impl Signature {
    /// Stable identity string used for trust-store keys and prompt display.
    #[must_use]
    pub fn identity_string(&self) -> String {
        match self {
            Signature::Sigstore { identity, .. } => format!("sigstore:{identity}"),
            Signature::Ed25519 { pubkey, .. } => {
                // Full 32-byte SHA-256 of the base64-encoded pubkey, hex-rendered.
                // An earlier MVP used only the first 8 bytes (64 bits) which
                // gave a 2^32 birthday-collision cost — a crafted pubkey
                // whose 8-byte SHA-256 prefix matched an already-trusted
                // entry would inherit its approved capabilities silently.
                use sha2::{Digest, Sha256};
                let hash = Sha256::digest(pubkey.as_bytes());
                format!("ed25519:{}", hex::encode(hash))
            }
        }
    }
}

/// Parse a manifest from a TOML string and validate semantic constraints.
pub fn parse_manifest_str(s: &str) -> Result<Manifest, PluginError> {
    let m: Manifest = toml::from_str(s)?;
    validate(&m)?;
    Ok(m)
}

/// Parse a manifest from a file path.
///
/// # Blocking I/O
///
/// This function reads from disk synchronously. It is intended to be called at
/// plugin-loader startup (before any concurrent work), or from within a
/// `spawn_blocking` closure in async callers.
#[allow(clippy::disallowed_methods)] // startup/load-time sync I/O — acceptable per clippy.toml policy
pub fn parse_manifest_file(path: &Path) -> Result<Manifest, PluginError> {
    let s = std::fs::read_to_string(path)?;
    parse_manifest_str(&s).map_err(|e| match e {
        PluginError::Toml(_) | PluginError::InvalidManifest { .. } => {
            PluginError::InvalidManifest {
                path: path.to_path_buf(),
                reason: e.to_string(),
            }
        }
        other => other,
    })
}

/// Validate a plugin name as a path-traversal-safe identifier.
///
/// Plugin names are used directly as filesystem path components (under
/// `~/.config/rdlp/plugins/<name>/`), as sled tree namespaces
/// (`plugin::<name>`), and as trust-store keys. Allowing arbitrary strings
/// would let `name = "../../.ssh"` resolve to a `remove_dir_all` outside
/// the plugin dir from the `rdlp plugin uninstall` command, and let
/// `name = "evil::collide"` shadow another plugin's sled namespace.
///
/// Rule: lowercase kebab-case, must start with `[a-z0-9]`, may contain
/// `[a-z0-9-]` thereafter, length 1..=64.
pub fn validate_plugin_name(name: &str) -> Result<(), PluginError> {
    fn err(name: &str, reason: &str) -> Result<(), PluginError> {
        Err(PluginError::InvalidPluginName {
            name: name.to_string(),
            reason: reason.to_string(),
        })
    }
    if name.is_empty() {
        return err(name, "empty");
    }
    if name.len() > 64 {
        return err(name, "longer than 64 characters");
    }
    let bytes = name.as_bytes();
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return err(name, "must start with a lowercase letter or digit");
    }
    for &b in bytes {
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return err(
                name,
                "only lowercase letters, digits, and hyphens are allowed",
            );
        }
    }
    Ok(())
}

fn validate(m: &Manifest) -> Result<(), PluginError> {
    if m.name.is_empty() {
        return invalid("empty name");
    }
    validate_plugin_name(&m.name)?;
    if !(100..=199).contains(&m.priority) {
        return invalid(&format!(
            "priority {} outside allowed range 100..=199",
            m.priority
        ));
    }
    if m.matches.is_empty() {
        return invalid("matches must declare at least one pattern");
    }
    for cap in &m.capabilities {
        if !KNOWN_CAPABILITIES.contains(&cap.as_str()) {
            return invalid(&format!("unknown capability '{cap}'"));
        }
    }
    if let Some(rx) = &m.url_regex
        && rx.len() > URL_REGEX_MAX_BYTES
    {
        return invalid(&format!(
            "url_regex source string too long ({} bytes; max {URL_REGEX_MAX_BYTES})",
            rx.len()
        ));
    }
    let has_tld_wildcard = m.matches.iter().any(|p| {
        // Detect any pattern whose host component is a bare `*` — i.e. there's
        // a `://*` followed by either '/' (path-bearing form) or end-of-string
        // (bare form). Both require the claim-all-urls capability.
        if let Some(after_scheme) = p.split_once("://").map(|(_, rest)| rest) {
            after_scheme == "*" || after_scheme.starts_with("*/")
        } else {
            false
        }
    });
    if has_tld_wildcard && !m.capabilities.iter().any(|c| c == "claim-all-urls") {
        return invalid("TLD-wildcard match pattern requires 'claim-all-urls' capability");
    }
    Ok(())
}

fn invalid(reason: &str) -> Result<(), PluginError> {
    Err(PluginError::InvalidManifest {
        path: std::path::PathBuf::new(),
        reason: reason.to_string(),
    })
}

/// Produce the canonical byte form of a manifest for signing.
///
/// Properties:
/// - keys sorted lexicographically at the top level
/// - list contents sorted lexicographically
/// - `signature` block excluded (the signature signs everything except itself)
/// - LF line endings, single space around `=`
/// - optional fields included only when present
///
/// Reference implementation in another language must produce identical bytes
/// for an equivalent manifest. Test fixtures live in
/// `crates/rdlp-plugin/tests/manifest_canonical.rs`.
///
/// **IMPORTANT — forward compatibility:** if you add a new field to `Manifest`,
/// decide explicitly whether to include it here. Omitting a field from the
/// canonical form is intentional in some cases (e.g. fields added for runtime
/// state that aren't part of the signed surface), but the omission must be
/// deliberate. Silently leaving a new field out is a signing-format bug that
/// breaks reproducibility in third-party tooling. Update this function or
/// document the omission inline when extending the manifest schema.
#[must_use]
pub fn canonical_bytes(m: &Manifest) -> Vec<u8> {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    let mut top: BTreeMap<&str, String> = BTreeMap::new();
    top.insert("capabilities", string_list(&m.capabilities));
    top.insert("claims_override", string_list(&m.claims_override));
    top.insert("matches", string_list(&m.matches));
    top.insert("name", quote_str(&m.name));
    top.insert("priority", m.priority.to_string());
    top.insert("supports_search", m.supports_search.to_string());
    top.insert("version", quote_str(&m.version));
    top.insert("wit_version", quote_str(&m.wit_version));
    if let Some(rx) = &m.url_regex {
        top.insert("url_regex", quote_str(rx));
    }

    let mut out = String::new();
    for (k, v) in &top {
        let _ = writeln!(out, "{k} = {v}");
    }
    out.into_bytes()
}

fn quote_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn string_list(v: &[String]) -> String {
    let mut s = String::from("[");
    let mut sorted = v.to_vec();
    sorted.sort();
    for (i, item) in sorted.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&quote_str(item));
    }
    s.push(']');
    s
}
