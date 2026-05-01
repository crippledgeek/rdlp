// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs,
)]

// Lints suppressed for test code — panicking on unexpected errors is intentional here.

use rdlp_plugin::host::cookie_jar::CookieJarCtx;

fn make_ctx(allowed: &[&str]) -> CookieJarCtx {
    CookieJarCtx::new_for_test(allowed.iter().map(|s| s.to_string()).collect())
}

#[test]
fn host_in_scope_exact_match() {
    let ctx = make_ctx(&["youtube.com"]);
    assert!(ctx.host_in_scope("youtube.com"));
}

#[test]
fn host_in_scope_subdomain_via_etld_collapse() {
    let ctx = make_ctx(&["youtube.com"]);
    assert!(ctx.host_in_scope("www.youtube.com"));
    assert!(ctx.host_in_scope("m.youtube.com"));
}

#[test]
fn host_in_scope_rejects_unrelated_etld() {
    let ctx = make_ctx(&["youtube.com"]);
    assert!(!ctx.host_in_scope("pornhub.com"));
    assert!(!ctx.host_in_scope("evil.com"));
}

#[test]
fn host_in_scope_rejects_etld_only() {
    let ctx = make_ctx(&["youtube.com"]);
    // .com is a TLD only — never an effective domain match.
    assert!(!ctx.host_in_scope("com"));
}

#[test]
fn host_in_scope_handles_subdomain_pattern() {
    // Plugin claims `*.youtube.com` — the host derives "youtube.com" as the
    // effective domain.
    let ctx = make_ctx(&["m.youtube.com"]);
    // Same etld+1 as the claim.
    assert!(ctx.host_in_scope("www.youtube.com"));
    // Different etld+1, must reject.
    assert!(!ctx.host_in_scope("vimeo.com"));
}

#[test]
fn extract_allowed_hosts_from_match_patterns() {
    // Helper that converts plugin manifest match patterns into a Vec<String>
    // of effective-domain hosts.
    let patterns = vec![
        "https://*.youtube.com/*".to_string(),
        "https://youtu.be/*".to_string(),
    ];
    let hosts = rdlp_plugin::host::cookie_jar::allowed_hosts_from_matches(&patterns);
    assert!(hosts.contains(&"youtube.com".to_string()));
    assert!(hosts.contains(&"youtu.be".to_string()));
}
