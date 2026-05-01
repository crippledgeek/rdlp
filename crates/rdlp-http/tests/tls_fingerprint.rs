// Integration tests aren't covered by clippy's `allow-unwrap-in-tests`
// (rust-clippy#13981) — re-allow at file scope. `disallowed_methods` permitted
// for `std::fs` test fixtures per clippy.toml policy (c). `missing_docs`
// exempt because integration tests aren't public API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::disallowed_methods,
    missing_docs
)]

//! Live-network JA4 fingerprint assertion.
//!
//! Marked `#[ignore]` so it does not hit the network during a normal
//! `cargo test`. Run manually before each Phase-2-related PR:
//!
//! ```
//! cargo test --ignored -p rdlp-http tls_fingerprint -- --nocapture
//! ```
//!
//! When CI is added, a nightly job runs `cargo test --ignored -p rdlp-http`
//! to catch drift. Drift = wreq-util shipped a new profile AND tls.peet.ws
//! updated its JA4 catalog.
//!
//! See docs/superpowers/specs/2026-04-24-tls-impersonation-design.md §6.11.

use rdlp_http::{HttpClientConfig, HttpClientFactory};
use rdlp_types::BrowserEmulation;

#[tokio::test]
#[ignore = "live network; run with --ignored"]
async fn ja4_matches_chrome_emulation() {
    let config = HttpClientConfig::default().with_emulation(BrowserEmulation::ChromeLatest);
    let client = HttpClientFactory::from_config(&config).build();

    let resp: serde_json::Value = client
        .get("https://tls.peet.ws/api/all")
        .send()
        .await
        .expect("tls.peet.ws request failed")
        .json()
        .await
        .expect("response was not JSON");

    let ja4 = resp["tls"]["ja4"]
        .as_str()
        .expect("tls.peet.ws response missing tls.ja4 field");

    eprintln!("Chrome emulation JA4: {ja4}");

    assert!(
        ja4.starts_with("t13d"),
        "expected TLS 1.3 d13 class, got {ja4}"
    );

    // Negative assertion — make sure we are NOT rustls's default fingerprint.
    // rustls 0.23 default JA4 on stable TLS 1.3 is (as of 2026-04-24):
    //   t13d1517h2_8daaf6152771_02713d6af862
    // Any rdlp-as-rustls leak would show that exact string.
    assert_ne!(
        ja4, "t13d1517h2_8daaf6152771_02713d6af862",
        "client leaked rustls-default fingerprint — emulation profile is not applied"
    );
}

#[tokio::test]
#[ignore = "live network; run with --ignored"]
async fn ja4_matches_firefox_emulation() {
    let config = HttpClientConfig::default().with_emulation(BrowserEmulation::FirefoxLatest);
    let client = HttpClientFactory::from_config(&config).build();

    let resp: serde_json::Value = client
        .get("https://tls.peet.ws/api/all")
        .send()
        .await
        .expect("tls.peet.ws request failed")
        .json()
        .await
        .expect("response was not JSON");

    let ja4 = resp["tls"]["ja4"].as_str().unwrap();
    eprintln!("Firefox emulation JA4: {ja4}");
    assert!(
        ja4.starts_with("t13d"),
        "expected TLS 1.3 d13 class, got {ja4}"
    );
}
