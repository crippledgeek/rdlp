//! Production-configuration coverage for the HLS seed-URL security gate
//! (issue #660).
//!
//! # Why this file exists as an integration test and not a unit test
//!
//! `hls::expand::validate_resolved_url` delegates to
//! `base::common::manifest_url::validate_manifest_sourced_url`, which carries
//! an exemption gated `cfg(any(test, feature = "loopback-test-exemption"))`
//! that lets `http(s)` loopback origins through, so mockito-backed tests can
//! drive expansion against a local fixture server at all. That exemption makes
//! the loopback rejection class unassertable from inside
//! `src/hls/expand.rs`'s own `#[cfg(test)] mod tests`.
//!
//! `cfg(test)` is set by rustc only while compiling a crate AS a test harness.
//! An integration test under `tests/` links `rdlp-extractor` as an ordinary
//! dependency, so `cfg(test)` is off here regardless. The `feature =
//! "loopback-test-exemption"` half of the `cfg(any(...))`, however, is a real
//! Cargo feature on this crate, and **Cargo unifies features across the whole
//! build graph for a given profile** — so when this crate is built in the
//! same `cargo test --workspace` invocation as `rdlp-plugin` or `rdlp-api`
//! (whose `[dev-dependencies]` enable the feature so *their own* mockito
//! tests can drive HLS expansion without depending on `rdlp-extractor`'s
//! `#[cfg(test)]` code), the feature is unified in here too, and this
//! integration test's binary carries the loopback exemption exactly as if
//! `cfg(test)` had applied. This is by design, not a leak to fix: two
//! sibling crates need the exemption in their own test builds, and Cargo has
//! no per-crate feature isolation within one workspace build.
//!
//! Because of that unification, the loopback-specific cases below
//! (`loopback_seed_rejected_in_production_build`,
//! `https_loopback_seed_rejected_in_production_build`) are compiled only when
//! the feature is OFF (`#[cfg(not(feature = "loopback-test-exemption"))]`) —
//! `cargo test -p rdlp-extractor` (no `--features`, and no unifying sibling in
//! the same invocation) is what actually exercises them. The other three
//! cases here — link-local, RFC 1918, and non-HTTP scheme — carry NO
//! exemption in either build configuration (see
//! `validate_manifest_sourced_url`'s `cfg(any(test, feature = ...))` gate,
//! which only ever widens the loopback check), so they prove the real gate
//! fires in an integration build unconditionally, feature on or off.
//!
//! The actual production guarantee — that no binary a user runs ever carries
//! `loopback-test-exemption` — is `scripts/check-loopback-feature-not-in-release.sh`,
//! which derives every workspace binary crate from `cargo metadata` and checks
//! its non-dev dependency graph for the feature. That script, not this file,
//! is what a future refactor widening the exemption's scope would need to
//! defeat.
//!
//! Every assertion below matches on the `URI rejected:` prefix, which only
//! `validate_resolved_url` emits. A bare `HlsExpandError::Network(_)` match
//! would also be satisfied by the connect failure an ungated build produces,
//! so it would pass with the gate deleted and guarantee nothing.

// Integration-test helpers below aren't `#[cfg(test)]`/`#[test]` items, so
// `allow-unwrap-in-tests` does not cover them; opt the whole file out per the
// documented convention in `clippy.toml` (rust-clippy#13981, #9062, #9612).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use rdlp_extractor::hls::{HlsExpandError, expand_hls_url};
use rdlp_types::{DownloadProtocol, Format};

/// Bound the RED run against ungated code. Without the seed gate these URLs
/// are really dialled, and a private-range address can absorb a SYN until the
/// OS default connect timeout. With the timeout the ungated run fails on the
/// assertion — the intended RED signal — instead of hanging.
const UNREACHABLE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Port 1 is reserved (`tcpmux`) and never bound by this project's fixtures,
/// so a regression that reaches the network refuses immediately rather than
/// hanging or, worse, hitting a real service.
const UNREACHABLE_PORT: u16 = 1;

fn client() -> Arc<wreq::Client> {
    Arc::new(
        wreq::Client::builder()
            .connect_timeout(UNREACHABLE_CONNECT_TIMEOUT)
            .build()
            .expect("client builds"),
    )
}

async fn expand_seed_err(url: &str) -> HlsExpandError {
    let seed = Format::new("hls", url, "m3u8", DownloadProtocol::M3u8);
    expand_hls_url(&seed, client())
        .await
        .expect_err("seed URL must be refused before any fetch")
}

/// Assert the refusal came from the security gate rather than from a failed
/// connect. `URI rejected:` is emitted at exactly one place in the crate.
fn assert_rejected_by_gate(err: &HlsExpandError, url: &str) {
    match err {
        HlsExpandError::Network(msg) => assert!(
            msg.starts_with("URI rejected:"),
            "{url} must be refused by the seed gate, not by a failed fetch; got: {msg}"
        ),
        other => panic!("expected Network(URI rejected: ...) for {url}, got: {other:?}"),
    }
}

/// The loopback rejection class, asserted against the shipping gate.
///
/// Feature-gated OFF: with `loopback-test-exemption` enabled (as it is when
/// this crate is unified with `rdlp-plugin`/`rdlp-api` under
/// `cargo test --workspace`), loopback origins are let through by design —
/// this exact case is what that feature exists to allow. Run
/// `cargo test -p rdlp-extractor` alone to exercise this assertion.
#[cfg(not(feature = "loopback-test-exemption"))]
#[tokio::test]
async fn loopback_seed_rejected_in_production_build() {
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let url = format!("http://{host}:{UNREACHABLE_PORT}/master.m3u8");
        assert_rejected_by_gate(&expand_seed_err(&url).await, &url);
    }
}

/// The loopback exemption covers `http` and `https` alike, so both must be
/// refused once it is absent — see `loopback_seed_rejected_in_production_build`
/// for why this is feature-gated OFF the same way.
#[cfg(not(feature = "loopback-test-exemption"))]
#[tokio::test]
async fn https_loopback_seed_rejected_in_production_build() {
    let url = format!("https://127.0.0.1:{UNREACHABLE_PORT}/master.m3u8");
    assert_rejected_by_gate(&expand_seed_err(&url).await, &url);
}

/// Link-local, including the cloud-metadata address. Covered by a unit test
/// too; repeated here because link-local carries no exemption in either build
/// configuration (`loopback-test-exemption` only ever widens the *loopback*
/// check), so this proves the real gate fires in an integration build
/// regardless of the feature.
#[tokio::test]
async fn link_local_seed_rejected_in_production_build() {
    let url = "http://169.254.169.254/latest/meta-data/";
    assert_rejected_by_gate(&expand_seed_err(url).await, url);
}

/// RFC 1918 private space, all three blocks.
#[tokio::test]
async fn rfc1918_seed_rejected_in_production_build() {
    for host in ["10.0.0.1", "172.16.0.1", "192.168.1.1"] {
        let url = format!("http://{host}:{UNREACHABLE_PORT}/master.m3u8");
        assert_rejected_by_gate(&expand_seed_err(&url).await, &url);
    }
}

/// Non-HTTP schemes get no exemption in either build configuration.
#[tokio::test]
async fn non_http_scheme_seed_rejected_in_production_build() {
    for url in ["file:///etc/passwd", "file://localhost/etc/passwd"] {
        assert_rejected_by_gate(&expand_seed_err(url).await, url);
    }
}
