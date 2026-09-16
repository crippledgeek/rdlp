// Lint-tightening for LIBRARY code only — see `Cargo.toml` `[lints.clippy]`.
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

//! Plugin manifest (`plugin.toml`) schema, parser, and canonical-bytes encoder.
//!
//! This crate is the **leaf** of the plugin manifest dependency graph: pure
//! data types + serde + toml + sha2. It exists so plugin-author tooling
//! (`tools/sign-plugin`) can pull in just what it needs without dragging in
//! `rdlp-plugin`'s wasmtime + sigstore + sled deps (which roughly tripled
//! `tools/sign-plugin/Cargo.lock` before the split).
//!
//! `rdlp-plugin` re-exports everything here through its own `manifest` module
//! so existing call sites are unaffected.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

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

/// Maximum byte length of `display_name`. It is display-only — rendered in
/// `%(extractor)s` and log tags — so the bound
/// exists to keep those surfaces readable, not to encode any protocol limit;
/// 64 matches the identifier-length cap already applied to `name` and
/// `search_site` by [`validate_plugin_name`], giving plugin authors one
/// length rule to remember across both fields.
const DISPLAY_NAME_MAX_BYTES: usize = 64;

/// The Unicode bidi embedding/override (`U+202A..=U+202E`) and isolate
/// (`U+2066..=U+2069`) controls — the nine code points behind the
/// Trojan-Source visual-reordering attack (CVE-2021-42574), and exactly the
/// set `rustc`'s `text_direction_codepoint_in_literal` lint denies. They
/// are general-category `Cf`, not `Cc`, so [`char::is_control`] does not
/// see them; `display_name` is rendered into every log tag and
/// `%(extractor)s`, where `X\u{202E}Y` would display reordered. Refused at
/// the source. Inline rather than `rdlp_redact::text::is_bidi_control`
/// (the same set) because this leaf crate deliberately carries no rdlp
/// dependency — author tooling links it alone.
const BIDI_CONTROLS: [std::ops::RangeInclusive<char>; 2] =
    ['\u{202A}'..='\u{202E}', '\u{2066}'..='\u{2069}'];

/// Errors that can be produced while parsing or validating a manifest.
///
/// `rdlp-plugin::PluginError` provides a `From<ManifestError>` conversion so
/// the rich host-side error type can absorb these without callers having to
/// handle them separately.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// Manifest TOML failed to parse or violated a structural invariant.
    #[error("manifest at {path} is invalid: {reason}")]
    InvalidManifest {
        /// Filesystem path of the offending manifest, or the empty path when
        /// the source was an in-memory string.
        path: PathBuf,
        /// Human-readable description of what is wrong.
        reason: String,
    },

    /// Plugin name violates the `[a-z0-9][a-z0-9-]{0,63}` shape required for
    /// safe filesystem / namespace use.
    #[error("invalid plugin name '{name}': {reason}")]
    InvalidPluginName {
        /// The offending name as supplied.
        name: String,
        /// Why it was rejected.
        reason: String,
    },

    /// A `claims_override` entry does not match any host in the `matches` patterns.
    ///
    /// Each `claims_override` entry must be the host (or an ancestor domain) of
    /// at least one `matches` URL pattern. Declaring a `claims_override` host that
    /// has no corresponding match pattern is a manifest authoring error — the
    /// override would be silently ignored during dispatch.
    #[error(
        "claims_override entry '{host}' does not correspond to any host in the matches patterns"
    )]
    ClaimsOverrideOutsideMatches {
        /// The `claims_override` host that has no match in the `matches` list.
        host: String,
    },

    /// TOML deserialisation error from `parse_manifest_str`.
    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),

    /// I/O error while reading a manifest file.
    #[error("manifest io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Plugin manifest as parsed from `plugin.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Plugin name (kebab-case, no namespace).
    pub name: String,
    /// Human-readable name for display only — `%(extractor)s`, log tags,
    /// and `PluginExtractor::name()`. Never used for identity, URL/search
    /// routing, the trust store, or the archive token; those stay on
    /// `name` (which also travels as `InfoDict::extractor_key`). Because
    /// `%(extractor)s` renders it into one output-path component, it may
    /// not contain a path separator (`/`, `\`), nor a control or bidi
    /// control character (see `validate_display_name`). Defaults to `name`
    /// when unset (see [`Manifest::display_name`]).
    #[serde(default)]
    pub display_name: Option<String>,
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
    /// Whether the plugin implements `extract` for real (default `true`). A
    /// search-only plugin sets this `false` and MUST set `supports_search`.
    /// TOML-only — `plugin-info` in the WIT is a shipped record and is not
    /// extended (COMPATIBILITY.md).
    #[serde(default = "default_true")]
    pub supports_extract: bool,
    /// Site name this plugin's `search` serves (the `--search-site` value).
    /// Defaults to `name`; set it when the search plugin's own name differs
    /// from the site (an extract-only `xhamster` plugin and a search-only
    /// `xhamster-search` plugin both routing `--search-site xhamster`).
    #[serde(default)]
    pub search_site: Option<String>,
    /// Search sites this plugin claims the right to shadow from a built-in
    /// (the `--search-site` counterpart of `claims_override`, which binds
    /// URL routing to hosts). A search has no URL, so the claim is bound
    /// to the site name instead: every entry must equal
    /// [`Manifest::search_site_name`] — a plugin serves one site — and the
    /// registry lets a plugin contest a built-in's site only when it is
    /// listed here. TOML-only, signed (in the canonical bytes when
    /// non-empty), surfaced at first-install and re-confirmed on change.
    #[serde(default)]
    pub search_claims_override: Vec<String>,
    /// Host capabilities the plugin requests (subset of `KNOWN_CAPABILITIES`).
    pub capabilities: Vec<String>,
    /// Signature backing the manifest + plugin.wasm (Sigstore or Ed25519).
    pub signature: Signature,
}

impl Manifest {
    /// Site name this plugin's `search` export serves — the `search_site`
    /// override when set, else `name`. `--search-site` routing compares
    /// against this value, never `name`, so a search-only plugin can be
    /// named independently of the site it searches.
    #[must_use]
    pub fn search_site_name(&self) -> &str {
        self.search_site.as_deref().unwrap_or(&self.name)
    }

    /// Human-readable name for display surfaces — the `display_name`
    /// override when set, else `name`. Never the identity/routing key;
    /// see [`Self::search_site_name`] and `name` for that.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// Whether this manifest claims the right to shadow the built-in
    /// search for the site it serves: `search_claims_override` names
    /// [`Self::search_site_name`]. Validation already rejects any other
    /// entry, so a non-empty list implies `true`; the membership test keeps
    /// the rule readable at the one place the registry asks.
    #[must_use]
    pub fn overrides_builtin_search(&self) -> bool {
        let site = self.search_site_name();
        self.search_claims_override.iter().any(|s| s == site)
    }

    /// The search-site claim this manifest makes, as the trust store
    /// records it and the prompts display it.
    #[must_use]
    pub fn search_claims(&self) -> SearchClaims {
        SearchClaims {
            search_site: self.search_site.clone(),
            search_claims_override: self.search_claims_override.clone(),
        }
    }
}

/// The search-site claim a manifest makes.
///
/// Which site its `search` serves, and whether it claims to shadow the
/// built-in for that site. Recorded in the trust store beside the approved
/// capabilities and compared on every load, so a plugin that starts
/// claiming a built-in's site after it was trusted is re-confirmed the way
/// capability creep is. Both fields default so a trust store written before
/// they existed still parses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchClaims {
    /// `Manifest::search_site` as declared (`None` = the plugin's name).
    #[serde(default)]
    pub search_site: Option<String>,
    /// `Manifest::search_claims_override` as declared.
    #[serde(default)]
    pub search_claims_override: Vec<String>,
}

/// Serde default for `Manifest::supports_extract` — `true`, matching every
/// manifest written before the field existed (all of which implement
/// `extract`).
const fn default_true() -> bool {
    true
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
        /// Base64-encoded 64-byte Ed25519 signature over
        /// (`canonical_bytes(manifest)` || `wasm_bytes`).
        signature: String,
    },
}

impl Signature {
    /// Stable identity string used for trust-store keys and prompt display.
    #[must_use]
    pub fn identity_string(&self) -> String {
        match self {
            Self::Sigstore { identity, .. } => format!("sigstore:{identity}"),
            Self::Ed25519 { pubkey, .. } => {
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
///
/// # Errors
///
/// - [`ManifestError::Toml`] — `s` is not valid TOML or cannot be deserialized
///   into [`Manifest`].
/// - [`ManifestError::InvalidManifest`] — the parsed manifest fails a semantic
///   invariant (empty name, out-of-range priority, unknown capability, etc.).
/// - [`ManifestError::InvalidPluginName`] — the `name` field violates the
///   kebab-case naming rule.
/// - [`ManifestError::ClaimsOverrideOutsideMatches`] — a `claims_override` entry
///   has no corresponding host in the `matches` patterns.
pub fn parse_manifest_str(s: &str) -> Result<Manifest, ManifestError> {
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
///
/// # Errors
///
/// - [`ManifestError::Io`] — the file cannot be read (missing, permission denied, etc.).
/// - All error variants from [`parse_manifest_str`] when the file contents fail
///   validation; in that case the error is wrapped as
///   [`ManifestError::InvalidManifest`] with the file path attached.
// Startup/load-time sync I/O — acceptable per the clippy.toml policy.
#[allow(clippy::disallowed_methods)]
pub fn parse_manifest_file(path: &Path) -> Result<Manifest, ManifestError> {
    let s = std::fs::read_to_string(path)?;
    parse_manifest_str(&s).map_err(|e| match e {
        ManifestError::Toml(_) | ManifestError::InvalidManifest { .. } => {
            ManifestError::InvalidManifest {
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
///
/// # Errors
///
/// Returns [`ManifestError::InvalidPluginName`] when the name is empty, longer
/// than 64 characters, starts with a non-alphanumeric character, or contains
/// any character outside `[a-z0-9-]`.
pub fn validate_plugin_name(name: &str) -> Result<(), ManifestError> {
    fn err(name: &str, reason: &str) -> Result<(), ManifestError> {
        Err(ManifestError::InvalidPluginName {
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
    // Safety: name.is_empty() is checked above, so bytes is guaranteed non-empty.
    // We use get(0) here to satisfy clippy::indexing_slicing; the unwrap_or
    // branch is unreachable by the invariant above.
    let first = bytes.first().copied().unwrap_or(0);
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

fn validate(m: &Manifest) -> Result<(), ManifestError> {
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
        p.split_once("://")
            .map(|(_, rest)| rest)
            .is_some_and(|after_scheme| after_scheme == "*" || after_scheme.starts_with("*/"))
    });
    if has_tld_wildcard && !m.capabilities.iter().any(|c| c == "claim-all-urls") {
        return invalid("TLD-wildcard match pattern requires 'claim-all-urls' capability");
    }

    // Validate claims_override: every entry must be the host (or an ancestor
    // domain) of at least one URL in the matches list. This ensures that
    // declared overrides are always load-bearing — a claims_override entry
    // with no corresponding match pattern is a manifest authoring error.
    let match_hosts: Vec<&str> = m
        .matches
        .iter()
        .filter_map(|p| {
            let after_scheme = p.split_once("://")?.1;
            let host_and_port = after_scheme.split('/').next()?;
            let host = if let Some((h, _)) = host_and_port.rsplit_once(':') {
                if h.contains(':') { host_and_port } else { h }
            } else {
                host_and_port
            };
            let host = host.strip_prefix("*.").unwrap_or(host);
            if host.is_empty() || host == "*" {
                None
            } else {
                Some(host)
            }
        })
        .collect();

    for override_host in &m.claims_override {
        let covered = match_hosts.iter().any(|mh| {
            mh.eq_ignore_ascii_case(override_host.as_str())
                || mh
                    .to_lowercase()
                    .ends_with(&format!(".{}", override_host.to_lowercase()))
        });
        if !covered {
            return Err(ManifestError::ClaimsOverrideOutsideMatches {
                host: override_host.clone(),
            });
        }
    }

    validate_capability_composition(m)?;
    validate_search_site(m)?;
    validate_display_name(m)?;

    Ok(())
}

/// `display_name` is rendered directly into `%(extractor)s` and log tags
/// (the first-install prompt shows `name`), so it is held to
/// plain-display-text rules rather than the filesystem-safe shape
/// `validate_plugin_name` enforces on `name`/`search_site`: any non-empty,
/// ≤64-byte string free of control and bidi-control characters
/// ([`BIDI_CONTROLS`]) is fine — spaces and mixed case included. The one
/// path rule it keeps: `%(extractor)s` is ONE output-path component, and
/// although the template renderer already maps `/` and `\` to `_` when it
/// renders a field, refusing them here keeps the display name an author
/// wrote the one the user sees — defence in depth at the source. Namespace
/// keys (the archive token, `host-store-kv`) stay on `name`.
fn validate_display_name(m: &Manifest) -> Result<(), ManifestError> {
    let Some(d) = &m.display_name else {
        return Ok(());
    };
    if d.is_empty() {
        return invalid("empty display_name");
    }
    if d.len() > DISPLAY_NAME_MAX_BYTES {
        return invalid(&format!(
            "display_name longer than {DISPLAY_NAME_MAX_BYTES} bytes"
        ));
    }
    if d.chars().any(char::is_control) {
        return invalid("display_name contains a control character");
    }
    if d.chars()
        .any(|c| BIDI_CONTROLS.iter().any(|block| block.contains(&c)))
    {
        return invalid("display_name contains a bidi control character");
    }
    if d.contains(['/', '\\']) {
        return invalid("display_name contains a path separator");
    }
    Ok(())
}

/// A plugin must implement at least one of `extract` or `search` — one with
/// neither would be dispatched to and always fail, so it is rejected at load
/// time rather than at first use.
fn validate_capability_composition(m: &Manifest) -> Result<(), ManifestError> {
    if !m.supports_extract && !m.supports_search {
        return invalid(
            "supports_extract = false requires supports_search = true \
             (a plugin must provide at least one capability)",
        );
    }
    Ok(())
}

/// `search_site` is used as a `--search-site` routing key and in trust-store
/// display, so it is held to the same path-traversal-safe shape as `name`
/// (see `validate_plugin_name`). `search_claims_override` entries are held
/// to the same shape, must name the one site this plugin serves, and only
/// make sense on a plugin that searches at all.
fn validate_search_site(m: &Manifest) -> Result<(), ManifestError> {
    if let Some(site) = &m.search_site {
        validate_plugin_name(site)?;
    }
    if !m.search_claims_override.is_empty() && !m.supports_search {
        return invalid("search_claims_override requires supports_search = true");
    }
    for claim in &m.search_claims_override {
        validate_plugin_name(claim)?;
        if claim != m.search_site_name() {
            return invalid(&format!(
                "search_claims_override entry '{claim}' is not the site this plugin serves ('{}')",
                m.search_site_name()
            ));
        }
    }
    Ok(())
}

fn invalid(reason: &str) -> Result<(), ManifestError> {
    Err(ManifestError::InvalidManifest {
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
/// - fields introduced after 0.5.0 (`supports_extract`, `search_site`,
///   `search_claims_override`, `display_name`) appear only when non-default
///   (`false` / present / non-empty / present respectively), so every
///   pre-0.5.1 manifest keeps its exact bytes and signature
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
    // Conditional so every pre-0.5.1 manifest keeps its exact canonical bytes
    // and signature (see the forward-compatibility note above): all three
    // fields are new, so only a non-default value can appear here.
    if !m.supports_extract {
        top.insert("supports_extract", "false".to_string());
    }
    if let Some(site) = &m.search_site {
        top.insert("search_site", quote_str(site));
    }
    if let Some(d) = &m.display_name {
        top.insert("display_name", quote_str(d));
    }
    if !m.search_claims_override.is_empty() {
        top.insert(
            "search_claims_override",
            string_list(&m.search_claims_override),
        );
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

#[cfg(test)]
mod display_name_tests {
    use super::{ManifestError, parse_manifest_str};

    /// A minimal-but-complete manifest, with `{line}` spliced in as an
    /// extra top-level key before `[signature]` — used to add a
    /// `display_name` line without hand-duplicating the whole fixture per test.
    fn manifest_with(line: &str) -> String {
        format!(
            r#"
name = "x"
version = "1.0.0"
wit_version = "0.5.0"
matches = ["https://x.com/*"]
priority = 150
capabilities = ["log"]
{line}

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#
        )
    }

    fn assert_invalid_reason_contains(toml: &str, needle: &str) {
        match parse_manifest_str(toml) {
            Err(ManifestError::InvalidManifest { reason, .. }) => {
                assert!(
                    reason.contains(needle),
                    "reason {reason:?} lacks {needle:?}"
                );
            }
            other => panic!("expected InvalidManifest, got {other:?}"),
        }
    }

    #[test]
    fn display_name_defaults_to_name() {
        let m = parse_manifest_str(&manifest_with("")).unwrap();
        assert_eq!(m.display_name(), "x");
    }

    #[test]
    fn display_name_empty_rejected() {
        assert_invalid_reason_contains(
            &manifest_with(r#"display_name = """#),
            "empty display_name",
        );
    }

    #[test]
    fn display_name_64_bytes_accepted() {
        let name = "a".repeat(64);
        let toml = manifest_with(&format!("display_name = \"{name}\""));
        let m = parse_manifest_str(&toml).unwrap();
        assert_eq!(m.display_name(), name);
    }

    #[test]
    fn display_name_65_bytes_rejected() {
        let name = "a".repeat(65);
        let toml = manifest_with(&format!("display_name = \"{name}\""));
        assert_invalid_reason_contains(&toml, "64 bytes");
    }

    /// Same boundary restated in bytes, not chars: 32 × "ä" is 64 bytes at
    /// 32 chars — `String::len` (bytes) is what the cap validates against,
    /// not `.chars().count()`, and this is the case that would tell them apart.
    #[test]
    fn display_name_64_bytes_multibyte_accepted() {
        let name = "ä".repeat(32);
        assert_eq!(name.len(), 64, "fixture must be exactly 64 bytes");
        let toml = manifest_with(&format!("display_name = \"{name}\""));
        let m = parse_manifest_str(&toml).unwrap();
        assert_eq!(m.display_name(), name);
    }

    #[test]
    fn display_name_66_bytes_multibyte_rejected() {
        let name = "ä".repeat(33);
        assert_eq!(name.len(), 66, "fixture must be over the 64-byte cap");
        let toml = manifest_with(&format!("display_name = \"{name}\""));
        assert_invalid_reason_contains(&toml, "64 bytes");
    }

    #[test]
    fn display_name_control_char_rejected() {
        // TOML's own `\u0007` escape — a literal control byte is not valid TOML
        // at all, so the escape is how the fixture reaches `validate`.
        assert_invalid_reason_contains(
            &manifest_with("display_name = \"X\\u0007\""),
            "control character",
        );
    }

    /// `display_name` is `InfoDict::extractor`, which `%(extractor)s`
    /// renders into one output-path component; the renderer would map a
    /// separator to `_` itself, so this refusal is defence in depth at the
    /// source rather than the only thing between the name and a directory.
    #[test]
    fn display_name_with_a_path_separator_rejected() {
        // TOML-escaped: `\\` in the file is one backslash in the value.
        for name in ["Site/Sub", r"Site\\Sub", "/", r"\\"] {
            assert_invalid_reason_contains(
                &manifest_with(&format!("display_name = \"{name}\"")),
                "path separator",
            );
        }
    }

    /// A bidi override or isolate is `Cf`, not `Cc`, so the control-character
    /// check above does not see it — yet `X\u{202E}Y` renders visually
    /// reordered in every log tag and `%(extractor)s` (Trojan Source,
    /// CVE-2021-42574). All nine hostile code points are refused; the
    /// neighbours just outside each block, and ordinary non-ASCII text, are
    /// still accepted.
    #[test]
    fn display_name_with_a_bidi_control_rejected() {
        for c in ('\u{202A}'..='\u{202E}').chain('\u{2066}'..='\u{2069}') {
            assert_invalid_reason_contains(
                &manifest_with(&format!("display_name = \"X{c}Y\"")),
                "bidi",
            );
        }
        for c in ['\u{2029}', '\u{202F}', '\u{2065}', '\u{206A}', 'é', '日'] {
            let name = format!("X{c}Y");
            let m = parse_manifest_str(&manifest_with(&format!("display_name = \"{name}\"")))
                .unwrap_or_else(|e| panic!("{name:?} (U+{:04X}) must be accepted: {e}", c as u32));
            assert_eq!(m.display_name(), name);
        }
    }

    #[test]
    fn display_name_with_spaces_and_case_accepted() {
        let m = parse_manifest_str(&manifest_with(r#"display_name = "XHamster Pro""#)).unwrap();
        assert_eq!(m.display_name(), "XHamster Pro");
    }
}

#[cfg(test)]
mod validate_plugin_name_tests {
    use super::{ManifestError, validate_plugin_name};

    fn assert_rejected(name: &str) {
        match validate_plugin_name(name) {
            Err(ManifestError::InvalidPluginName { .. }) => {}
            other => panic!("expected InvalidPluginName for {name:?}, got {other:?}"),
        }
    }

    #[test]
    fn empty_name_rejected() {
        assert_rejected("");
    }

    #[test]
    fn over_64_chars_rejected() {
        assert_rejected(&"a".repeat(65));
    }

    #[test]
    fn exactly_64_chars_accepted() {
        validate_plugin_name(&"a".repeat(64)).unwrap();
    }

    #[test]
    fn uppercase_rejected() {
        assert_rejected("MyPlugin");
        assert_rejected("PLUGIN");
    }

    #[test]
    fn underscore_rejected() {
        assert_rejected("my_plugin");
    }

    #[test]
    fn dot_rejected() {
        assert_rejected("my.plugin");
    }

    #[test]
    fn slash_rejected_path_traversal() {
        // The motivating attack: validate_plugin_name is the gate against
        // `name = "../../.ssh"` resolving outside the plugin dir.
        assert_rejected("../evil");
        assert_rejected("foo/bar");
        assert_rejected("..");
    }

    #[test]
    fn colon_rejected_sled_namespace_collision() {
        // Sled trees are namespaced as `plugin::<name>`; a colon would let
        // `name = "evil::collide"` shadow another plugin's namespace.
        assert_rejected("evil::collide");
        assert_rejected("a:b");
    }

    #[test]
    fn leading_hyphen_rejected() {
        assert_rejected("-foo");
    }

    #[test]
    fn whitespace_rejected() {
        assert_rejected("foo bar");
        assert_rejected(" foo");
        assert_rejected("foo\n");
    }

    #[test]
    fn null_byte_rejected() {
        assert_rejected("foo\0bar");
    }

    #[test]
    fn valid_kebab_accepted() {
        validate_plugin_name("my-plugin-1").unwrap();
        validate_plugin_name("a").unwrap();
        validate_plugin_name("0").unwrap();
        validate_plugin_name("a-b-c").unwrap();
        validate_plugin_name("plugin123").unwrap();
    }

    #[test]
    fn double_hyphen_accepted_per_rule() {
        // Rule allows any combination of [a-z0-9-]; consecutive hyphens are
        // permitted (no collision with any escape sequence in the consumers).
        validate_plugin_name("my--plugin").unwrap();
    }
}
