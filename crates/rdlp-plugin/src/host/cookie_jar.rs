//! `host:cookie-jar` capability — scoped cookie access for plugins.
//!
//! Each plugin's cookie access is restricted to the effective domains derived
//! from its declared match patterns. A plugin claiming `*.youtube.com`
//! resolves to effective domain `youtube.com` via the Public Suffix List;
//! cookie reads/writes for any other effective domain are refused. This is
//! the vector A3 mitigation (cookie-jar cross-contamination).

use crate::instance::PluginStoreData;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
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
    /// Tracks whether the `get_cookies` stub-warning has fired for this
    /// plugin instance — emit at most once per ctx so logs don't drown.
    pub get_warned: AtomicBool,
}

impl CookieJarCtx {
    /// Construct a context with a real cookie jar.
    #[must_use]
    pub fn new(jar: Arc<rdlp_cookies::SimpleCookieJar>, match_patterns: &[String]) -> Self {
        Self {
            jar,
            allowed_etld_plus_one: allowed_hosts_from_matches(match_patterns),
            get_warned: AtomicBool::new(false),
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
            get_warned: AtomicBool::new(false),
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
    /// NOTE: The underlying `SimpleCookieJar` exposes `get_cookies(url) ->
    /// Result<Vec<String>>` which returns `"name=value"` strings, not structured
    /// `Cookie` records. Mapping those strings to WIT `Cookie` records requires
    /// a cookie parser (not yet wired). This implementation returns an empty
    /// `Vec` when cookies exist but correctly enforces the scoping gate — the
    /// real read is a follow-up task.
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
        // Surface the stub to plugin authors exactly once per plugin so they
        // don't waste hours debugging "0 cookies" — the scoping gate above
        // works, but the read-side cookie parser is a follow-up task.
        if !ctx
            .get_warned
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            log::warn!(
                target: &self.log_target,
                "host:cookie-jar get_cookies returns an empty list — \
                 cookie record parsing not yet wired (security scoping IS enforced)"
            );
        }
        Vec::new()
    }

    /// Store a cookie from a plugin, scoped to the plugin's match-pattern domains.
    ///
    /// Returns `Err(String)` when:
    /// - the `cookie-jar` capability was not granted, or
    /// - the URL is invalid or has no host, or
    /// - the URL's host falls outside the plugin's allowed match-pattern scope.
    ///
    /// NOTE: `SimpleCookieJar::add_cookie(url, cookie_str)` expects a
    /// `"name=value"` string. Converting the WIT `Cookie` record to that format
    /// (including `Domain`, `Path`, `Secure`, `HttpOnly`, `Expires` attributes)
    /// is wired here as a best-effort `"name=value"` write. Full attribute
    /// propagation is a follow-up task.
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
        // Wire the actual write via the CookieJar trait's `add_cookie` method.
        // We format the cookie as "name=value"; full attribute support is a
        // follow-up (Domain/Path/Secure/HttpOnly/Expires directives).
        use rdlp_core::CookieJar as _;
        let cookie_str = format!("{}={}", c.name, c.value);
        ctx.jar
            .add_cookie(&url, &cookie_str)
            .await
            .map_err(|e| format!("cookie write failed: {e}"))
    }
}
