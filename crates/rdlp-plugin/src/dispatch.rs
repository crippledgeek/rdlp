//! Chrome-style match-pattern parser and dispatcher.
//!
//! Patterns follow the form `<scheme>://<host>/<path>` where:
//! - `<scheme>` is `http`, `https`, `file`, or `*` (matches http or https)
//! - `<host>` is a literal hostname, `*.example.com` (subdomain wildcard;
//!   the bare host `example.com` also matches), or `*` (full TLD wildcard;
//!   requires the `claim-all-urls` capability — gated at manifest validation)
//! - `<path>` is a literal or glob with `*` standing for "any chars"

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
