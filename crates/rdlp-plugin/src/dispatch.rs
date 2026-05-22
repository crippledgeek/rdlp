//! Chrome-style match-pattern parser and dispatcher.
//!
//! Patterns follow the form `<scheme>://<host>/<path>` where:
//! - `<scheme>` is `http`, `https`, `file`, or `*` (matches http or https)
//! - `<host>` is a literal hostname, `*.example.com` (subdomain wildcard;
//!   the bare host `example.com` also matches), or `*` (full TLD wildcard;
//!   requires the `claim-all-urls` capability — gated at manifest validation)
//! - `<path>` is a literal or glob with `*` standing for "any chars"

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::indexing_slicing,
    clippy::match_same_arms,
    clippy::manual_let_else
)]

use crate::PluginError;
use url::Url;

/// Scheme component of a [`MatchPattern`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemeMatch {
    /// Matches only `http://` URLs.
    Http,
    /// Matches only `https://` URLs.
    Https,
    /// `*` — matches http or https.
    Either,
    /// Matches only `file://` URLs.
    File,
}

/// Host component of a [`MatchPattern`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMatch {
    /// Exact host match (e.g. `youtube.com`).
    Exact(String),
    /// `*.example.com` — matches `example.com` and any subdomain.
    SubdomainWildcard(String),
    /// `*` — matches any host. Requires `claim-all-urls` capability at the
    /// manifest level (enforced in [`crate::manifest`]).
    Any,
}

/// A parsed Chrome-style match pattern.
///
/// # Syntax
///
/// ```text
/// <scheme>://<host>/<path>
/// ```
///
/// - `<scheme>`: `http`, `https`, `file`, or `*` (both http and https)
/// - `<host>`: exact hostname, `*.example.com`, or `*` (any)
/// - `<path>`: literal path or glob using `*` as a wildcard
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchPattern {
    /// Scheme matcher.
    pub scheme: SchemeMatch,
    /// Host matcher.
    pub host: HostMatch,
    /// Path glob, including the leading `/`.
    pub path_glob: String,
}

impl MatchPattern {
    /// Parse a Chrome-style match pattern string.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Internal`] when the pattern is structurally
    /// invalid (unsupported scheme, missing path component, etc.).
    pub fn parse(s: &str) -> Result<Self, PluginError> {
        let (scheme_str, rest) = s
            .split_once("://")
            .ok_or_else(|| invalid(s, "no scheme separator"))?;

        let scheme = match scheme_str {
            "http" => SchemeMatch::Http,
            "https" => SchemeMatch::Https,
            "*" => SchemeMatch::Either,
            "file" => SchemeMatch::File,
            other => return Err(invalid(s, &format!("unsupported scheme '{other}'"))),
        };

        let (host_str, path) = rest
            .split_once('/')
            .ok_or_else(|| invalid(s, "no path component"))?;

        let host = if host_str == "*" {
            HostMatch::Any
        } else if let Some(suffix) = host_str.strip_prefix("*.") {
            HostMatch::SubdomainWildcard(suffix.to_string())
        } else if host_str.is_empty() {
            // `file:///path` — host_str is empty; file URLs have no host.
            HostMatch::Any
        } else {
            HostMatch::Exact(host_str.to_string())
        };

        let path_glob = format!("/{path}");
        Ok(Self {
            scheme,
            host,
            path_glob,
        })
    }

    /// Returns `true` if this pattern matches `url`.
    #[must_use]
    pub fn matches(&self, url: &Url) -> bool {
        let scheme_ok = match self.scheme {
            SchemeMatch::Http => url.scheme() == "http",
            SchemeMatch::Https => url.scheme() == "https",
            SchemeMatch::Either => matches!(url.scheme(), "http" | "https"),
            SchemeMatch::File => url.scheme() == "file",
        };
        if !scheme_ok {
            return false;
        }

        let host_ok = match (&self.host, url.host_str()) {
            (HostMatch::Any, _) => true,
            (HostMatch::Exact(h), Some(uh)) => h == uh,
            (HostMatch::SubdomainWildcard(suffix), Some(uh)) => {
                uh == suffix || uh.ends_with(&format!(".{suffix}"))
            }
            _ => false,
        };
        if !host_ok {
            return false;
        }

        glob_path_matches(&self.path_glob, url.path())
    }
}

/// Lightweight glob: `*` matches any number of any characters (including `/`).
///
/// This is `O(g * p)` for non-pathological inputs (a single `*` star).
fn glob_path_matches(glob: &str, path: &str) -> bool {
    fn rec(g: &[u8], p: &[u8]) -> bool {
        match (g.first(), p.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(b'*'), _) => {
                // Match zero or more of any character.
                (0..=p.len()).any(|i| rec(&g[1..], &p[i..]))
            }
            (Some(_), None) => false,
            (Some(a), Some(b)) if a == b => rec(&g[1..], &p[1..]),
            _ => false,
        }
    }
    rec(glob.as_bytes(), path.as_bytes())
}

fn invalid(s: &str, why: &str) -> PluginError {
    PluginError::Internal(format!("invalid match pattern '{s}': {why}"))
}

// ── URL regex compilation ─────────────────────────────────────────────────────

use regex::RegexBuilder;
use std::time::{Duration, Instant};

/// Maximum compiled NFA size for a plugin's `url_regex`, in bytes. Guards
/// against counted-repetition expansion (e.g. `a{5}{5}{5}{5}{5}` blowing the
/// compiled automaton).
const REGEX_SIZE_LIMIT: usize = 64 * 1024;

/// Maximum lazy-DFA size in bytes — a separate cap from the NFA `size_limit`.
const REGEX_DFA_SIZE_LIMIT: usize = 64 * 1024;

/// Wall-clock cap on regex compilation. Catches pathological patterns that
/// pass the size limits but take an unreasonable amount of time to build.
const REGEX_COMPILE_TIMEOUT: Duration = Duration::from_secs(1);

/// Compile a plugin's `url_regex` with hardened limits.
///
/// Rust's regex crate uses an RE2-style linear-time engine and is structurally
/// immune to ReDoS at match time, but its compile-time complexity is unbounded
/// by default. The `size_limit` and `dfa_size_limit` knobs cap the compiled
/// output; the wall-clock check defends against pathological inputs that compile
/// slowly even within those caps.
///
/// On error, [`PluginError::RegexCompile`] carries `plugin` so the operator can
/// identify the offending plugin.
pub fn compile_url_regex(plugin: &str, source: &str) -> Result<regex::Regex, PluginError> {
    let start = Instant::now();
    let result = RegexBuilder::new(source)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_DFA_SIZE_LIMIT)
        .build();
    if start.elapsed() > REGEX_COMPILE_TIMEOUT {
        return Err(PluginError::RegexCompile {
            plugin: plugin.to_string(),
            reason: "compilation exceeded 1s wall-clock".to_string(),
        });
    }
    result.map_err(|e| PluginError::RegexCompile {
        plugin: plugin.to_string(),
        reason: e.to_string(),
    })
}

/// Returns `true` if `url` is claimed by the plugin's manifest.
///
/// The manifest's `matches` array (Chrome-style match patterns) is the
/// authoritative declaration of which URLs a plugin handles. This function
/// parses the URL once, then checks every pattern.
///
/// **Empty `matches`:** returns `true` for any `http`/`https` URL that
/// parses cleanly. This preserves backwards compatibility with the
/// adapter's permissive `^https?://` regex fallback for plugins that
/// were authored before manifest patterns were enforced — but any
/// plugin shipping today MUST declare `matches` to scope its claim.
///
/// **Malformed URL:** returns `false` (no panic, no claim).
///
/// This is the fix for the godresource case: manifest declares
/// `["https://new.godresource.com/*"]` but the plugin's `valid_url()`
/// regex (the legacy fallback) was the permissive `^https?://`. Result:
/// the plugin claimed the apex `https://godresource.com/...` URL even
/// though the manifest didn't, the dispatcher picked it because of its
/// priority over Generic, and the wasm rejected the URL internally
/// after the user already paid the dispatch cost. Honoring `matches`
/// at `suitable()` time means the dispatcher correctly falls through
/// to Generic for URLs the plugin never claimed.
#[must_use]
pub fn claims_url(manifest: &crate::manifest::Manifest, url: &str) -> bool {
    let parsed = match Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if manifest.matches.is_empty() {
        return matches!(parsed.scheme(), "http" | "https");
    }
    manifest
        .matches
        .iter()
        .any(|raw| MatchPattern::parse(raw).is_ok_and(|p| p.matches(&parsed)))
}

/// Dispatcher backed by a linear scan over registered patterns.
///
/// MVP-grade — good enough for hundreds of plugins. Future optimization:
/// hostname-trie indexing if profiling shows hotspots.
#[derive(Default)]
pub struct MatchTrie<T> {
    entries: Vec<(MatchPattern, T)>,
}

impl<T: Clone> MatchTrie<T> {
    /// Register `value` under `pattern`. A URL may match multiple patterns;
    /// [`MatchTrie::lookup`] returns all of them in insertion order.
    pub fn insert(&mut self, pattern: MatchPattern, value: T) {
        self.entries.push((pattern, value));
    }

    /// Return all values whose pattern matches `url`, in insertion order.
    #[must_use]
    pub fn lookup(&self, url: &Url) -> Vec<T> {
        self.entries
            .iter()
            .filter(|(p, _)| p.matches(url))
            .map(|(_, v)| v.clone())
            .collect()
    }
}

#[cfg(test)]
mod claims_url_tests {
    use super::claims_url;
    use crate::manifest::parse_manifest_str;

    fn manifest_with_matches(patterns: &[&str]) -> crate::manifest::Manifest {
        let matches_toml = patterns
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let toml = format!(
            r#"
name = "test"
version = "1.0.0"
wit_version = "0.3.0"
matches = [{matches_toml}]
priority = 150
capabilities = ["log"]

[signature]
type = "ed25519"
pubkey = "ZA"
signature = "ZA"
"#,
        );
        parse_manifest_str(&toml).expect("parse")
    }

    #[test]
    fn matches_exact_host_pattern() {
        let m = manifest_with_matches(&["https://new.godresource.com/*"]);
        assert!(claims_url(&m, "https://new.godresource.com/video/abc"));
    }

    #[test]
    fn rejects_apex_when_pattern_requires_subdomain() {
        // Regression: the godresource plugin's manifest declares
        // `["https://new.godresource.com/*"]`, but before the
        // claims_url fix the adapter's permissive `^https?://` fallback
        // claimed every URL. Apex URL `godresource.com/...` must be
        // rejected so the dispatcher falls through to Generic.
        let m = manifest_with_matches(&["https://new.godresource.com/*"]);
        assert!(!claims_url(&m, "https://godresource.com/video/abc"));
    }

    #[test]
    fn rejects_other_host() {
        let m = manifest_with_matches(&["https://new.godresource.com/*"]);
        assert!(!claims_url(&m, "https://example.com/video/abc"));
    }

    #[test]
    fn rejects_wrong_scheme() {
        let m = manifest_with_matches(&["https://new.godresource.com/*"]);
        assert!(!claims_url(&m, "http://new.godresource.com/video/abc"));
    }

    // Note: the empty-matches fallback in claims_url is defensive code
    // — parse_manifest_str rejects manifests with empty `matches`, so
    // the branch is unreachable through normal manifest construction.
    // Kept in the implementation in case future refactors loosen the
    // schema; not tested here because we cannot construct a Manifest
    // that triggers it.

    #[test]
    fn malformed_url_returns_false() {
        let m = manifest_with_matches(&["https://example.com/*"]);
        assert!(!claims_url(&m, "not a url"));
    }

    #[test]
    fn multiple_match_patterns_or_together() {
        let m = manifest_with_matches(&["https://a.example.com/*", "https://b.example.com/*"]);
        assert!(claims_url(&m, "https://a.example.com/x"));
        assert!(claims_url(&m, "https://b.example.com/x"));
        assert!(!claims_url(&m, "https://c.example.com/x"));
    }

    #[test]
    fn subdomain_wildcard_pattern() {
        let m = manifest_with_matches(&["https://*.example.com/*"]);
        assert!(claims_url(&m, "https://api.example.com/x"));
        assert!(claims_url(&m, "https://www.example.com/x"));
        // Match-pattern semantics: *.example.com also matches the apex.
        assert!(claims_url(&m, "https://example.com/x"));
        assert!(!claims_url(&m, "https://example.org/x"));
    }
}
