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
                use sha2::{Digest, Sha256};
                let hash = Sha256::digest(pubkey.as_bytes());
                format!("ed25519:{}", hex::encode(&hash[..8]))
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

fn validate(m: &Manifest) -> Result<(), PluginError> {
    if m.name.is_empty() {
        return invalid("empty name");
    }
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
        p.starts_with("https://*/")
            || p.starts_with("http://*/")
            || p.starts_with("*://*/")
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
