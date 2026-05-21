//! `host:cookie-jar` capability — scoped cookie access for plugins.
//!
//! Each plugin's cookie access is restricted to the effective domains derived
//! from its declared match patterns. A plugin claiming `*.youtube.com`
//! resolves to effective domain `youtube.com` via the Public Suffix List;
//! cookie reads/writes for any other effective domain are refused. This is
//! the vector A3 mitigation (cookie-jar cross-contamination).

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::format_push_string,
    clippy::items_after_statements,
    clippy::cast_possible_wrap,
    clippy::missing_errors_doc,
    clippy::option_if_let_else
)]

use crate::instance::PluginStoreData;
use std::sync::Arc;
use wasmtime::component::Linker;

/// Per-plugin cookie context. Holds a clone of the shared `SimpleCookieJar`
/// plus the set of allowed effective domains.
pub struct CookieJarCtx {
    /// Shared cookie jar — same instance across all plugins; scoping is enforced
    /// at this layer, not in the jar itself.
    pub jar: Arc<rdlp_cookies::SimpleCookieJar>,
    /// Allowed effective domains (eTLD+1 form) derived from the plugin's
    /// match patterns. Lower-case, no scheme, no port.
    pub allowed_etld_plus_one: Vec<String>,
}

impl CookieJarCtx {
    /// Construct a context with a real cookie jar.
    #[must_use]
    pub fn new(jar: Arc<rdlp_cookies::SimpleCookieJar>, match_patterns: &[String]) -> Self {
        Self {
            jar,
            allowed_etld_plus_one: allowed_hosts_from_matches(match_patterns),
        }
    }

    /// Convenience constructor for tests — creates a fresh, empty
    /// `SimpleCookieJar` and scopes it to the given eTLD+1 list.
    ///
    /// Integration tests live in separate crates and cannot use
    /// `#[cfg(test)]`-only items from the library, so this constructor is
    /// always compiled but hidden from rustdoc.
    #[must_use]
    #[doc(hidden)]
    pub fn new_for_test(allowed_etld_plus_one: Vec<String>) -> Self {
        Self {
            jar: Arc::new(rdlp_cookies::SimpleCookieJar::new()),
            allowed_etld_plus_one,
        }
    }

    /// Check whether `url_host` is in scope for this plugin. Returns `true` if
    /// the URL's effective domain matches any of the allowed eTLD+1 entries.
    #[must_use]
    pub fn host_in_scope(&self, url_host: &str) -> bool {
        let url_etld = effective_etld_plus_one(url_host);
        // Guard: if the URL resolves to no meaningful eTLD+1 (e.g. bare TLD
        // or empty), reject immediately.
        if url_etld.is_empty() {
            return false;
        }
        self.allowed_etld_plus_one
            .iter()
            .any(|allowed| effective_etld_plus_one(allowed) == url_etld)
    }
}

/// Extract effective-domain hosts from a list of Chrome-style match patterns.
///
/// Returns lower-case, port-stripped eTLD+1 strings (e.g. `"youtube.com"`).
/// Patterns like `"https://*.youtube.com/*"` → `"youtube.com"`.
#[must_use]
pub fn allowed_hosts_from_matches(patterns: &[String]) -> Vec<String> {
    let mut out: Vec<String> = patterns
        .iter()
        .filter_map(|p| {
            // Strip scheme (e.g. "https://").
            let after_scheme = p.split_once("://")?.1;
            // Strip path (everything from the first '/' after the host).
            let host_and_port = after_scheme.split('/').next()?;
            // Strip port if present.
            let host = if let Some((h, _port)) = host_and_port.rsplit_once(':') {
                // Only strip port if the part before ':' looks like a hostname,
                // not an IPv6 address (we skip IPv6 for now — plugins target
                // real CDN hostnames).
                if h.contains(':') {
                    // IPv6 — leave as-is.
                    host_and_port
                } else {
                    h
                }
            } else {
                host_and_port
            };
            // Strip leading "*." subdomain wildcard.
            let host = host.strip_prefix("*.").unwrap_or(host);
            // Reject catch-all wildcards.
            if host == "*" || host.is_empty() {
                return None;
            }
            let etld = effective_etld_plus_one(host);
            if etld.is_empty() { None } else { Some(etld) }
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Reduce a hostname to its effective eTLD+1 (e.g. `"www.youtube.com"` →
/// `"youtube.com"`).
///
/// Falls back to the input hostname if the PSL lookup fails (e.g. for hosts
/// like `"localhost"` or non-public TLDs). Returns an empty string for bare
/// TLD inputs (e.g. `"com"`) so callers can reject them.
fn effective_etld_plus_one(host: &str) -> String {
    let lower = host.to_lowercase();
    match psl::domain_str(&lower) {
        Some(d) => d.to_string(),
        None => {
            // PSL returned None — could be a bare TLD ("com"), unknown TLD, or
            // private-label domain. Return empty to signal "no recognisable
            // registrable domain".
            String::new()
        }
    }
}

/// Wire `host:cookie-jar` into a component linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_cookie_jar::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_cookie_jar::Host for PluginStoreData {
    /// Return cookies for the given URL, filtered to those visible at that URL.
    ///
    /// Returns an empty list when:
    /// - the `cookie-jar` capability was not granted to this plugin, or
    /// - the URL is invalid, or
    /// - the URL's host is outside the plugin's allowed match-pattern scope.
    ///
    /// The returned `Cookie` records have `name` and `value` populated from the
    /// underlying jar. The `domain`, `path`, `secure`, and `http_only` fields are
    /// inferred from the request URL since the Cookie header format does not carry
    /// attributes; `expires` is always `None` for the same reason.
    async fn get_cookies(
        &mut self,
        url: String,
    ) -> Vec<crate::bindings::rdlp::plugin::host_cookie_jar::Cookie> {
        let Some(ctx) = self.cookie_jar.as_ref() else {
            return Vec::new();
        };
        let Ok(parsed) = url::Url::parse(&url) else {
            return Vec::new();
        };
        let Some(host) = parsed.host_str() else {
            return Vec::new();
        };
        if !ctx.host_in_scope(host) {
            return Vec::new();
        }

        use rdlp_core::CookieJar as _;
        let raw_cookies = match ctx.jar.cookies(&url).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    target: &self.log_target,
                    "host:cookie-jar get_cookies error: {e}"
                );
                return Vec::new();
            }
        };

        // The jar returns "name=value" strings (Cookie header format). Parse
        // each one back into a WIT Cookie record. Attributes (Domain, Path,
        // Secure, HttpOnly, Expires) are not available from the Cookie header;
        // fill in defaults derived from the request URL.
        let url_host = host.to_string();
        let url_path = parsed.path().to_string();
        let is_secure = parsed.scheme() == "https";

        raw_cookies
            .into_iter()
            .filter_map(|pair| {
                // pair format: "name=value" (the jar strips attributes)
                let (name, value) = if let Some((n, v)) = pair.split_once('=') {
                    (n.to_string(), v.to_string())
                } else {
                    // name-only cookie (rare but valid per RFC 6265)
                    (pair, String::new())
                };
                if name.is_empty() {
                    return None;
                }
                Some(crate::bindings::rdlp::plugin::host_cookie_jar::Cookie {
                    name,
                    value,
                    domain: url_host.clone(),
                    path: url_path.clone(),
                    secure: is_secure,
                    http_only: false,
                    expires: None,
                })
            })
            .collect()
    }

    /// Store a cookie from a plugin, scoped to the plugin's match-pattern domains.
    ///
    /// Returns `Err(String)` when:
    /// - the `cookie-jar` capability was not granted, or
    /// - the URL is invalid or has no host, or
    /// - the URL's host falls outside the plugin's allowed match-pattern scope.
    ///
    /// All attributes from the WIT `Cookie` record — `Domain`, `Path`,
    /// `Secure`, `HttpOnly`, and `Expires` — are propagated into the
    /// Set-Cookie header string fed to the underlying jar.
    async fn set_cookie(
        &mut self,
        url: String,
        c: crate::bindings::rdlp::plugin::host_cookie_jar::Cookie,
    ) -> Result<(), String> {
        let Some(ctx) = self.cookie_jar.as_ref() else {
            return Err("cookie-jar capability not granted".into());
        };
        let parsed = url::Url::parse(&url).map_err(|e| format!("invalid url: {e}"))?;
        let Some(host) = parsed.host_str() else {
            return Err("url has no host".into());
        };
        if !ctx.host_in_scope(host) {
            return Err(format!(
                "cookie-jar access denied: {host} not in plugin's match-pattern scope"
            ));
        }

        // Build a full Set-Cookie header value including all attributes so that
        // the jar records Domain, Path, Secure, HttpOnly, and Expires faithfully.
        let mut cookie_str = format!("{}={}", c.name, c.value);
        if !c.domain.is_empty() {
            cookie_str.push_str(&format!("; Domain={}", c.domain));
        }
        if !c.path.is_empty() {
            cookie_str.push_str(&format!("; Path={}", c.path));
        }
        if c.secure {
            cookie_str.push_str("; Secure");
        }
        if c.http_only {
            cookie_str.push_str("; HttpOnly");
        }
        if let Some(expires_unix) = c.expires {
            // Use Max-Age rather than Expires so the value is wall-clock
            // independent and avoids timezone/formatting complexity.
            use std::time::{SystemTime, UNIX_EPOCH};
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let max_age_secs = (expires_unix as i64) - (now_secs as i64);
            // Only set Max-Age when the cookie hasn't already expired.
            if max_age_secs > 0 {
                cookie_str.push_str(&format!("; Max-Age={max_age_secs}"));
            }
        }

        use rdlp_core::CookieJar as _;
        ctx.jar
            .add_cookie(&url, &cookie_str)
            .await
            .map_err(|e| format!("cookie write failed: {e}"))
    }
}
