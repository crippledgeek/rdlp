//! Production-configuration coverage for the HLS seed-URL security gate
//! (issue #660).
//!
//! # Why this file exists as an integration test and not a unit test
//!
//! `hls::expand::validate_resolved_url` carries a `#[cfg(test)]` exemption that
//! lets `http(s)` loopback origins through, so mockito-backed unit tests can
//! drive expansion against a local fixture server at all. That exemption makes
//! the loopback rejection class unassertable from inside
//! `src/hls/expand.rs`'s own `#[cfg(test)] mod tests`.
//!
//! `cfg(test)` is set by rustc only while compiling a crate AS a test harness.
//! An integration test under `tests/` links `rdlp-extractor` as an ordinary
//! dependency, so the library here is the **production** build: the loopback
//! exemption is not compiled in, and the real gate is reachable.
//!
//! That buys two things the unit tests cannot:
//!
//! 1. the loopback rejection class, asserted against the gate that actually
//!    ships; and
//! 2. a regression guard on the exemption's own scope — if a future refactor
//!    widens it out of `cfg(test)`, or drops the `cfg` attribute entirely,
//!    these tests go red. That widening is precisely the failure the gate
//!    exists to prevent, and no unit test can observe it, because in a unit
//!    test the exemption is supposed to be present.
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
/// This is the case the unit tests structurally cannot cover: in
/// `src/hls/expand.rs`'s own test module the `cfg(test)` exemption lets these
/// exact URLs through on purpose.
#[tokio::test]
async fn loopback_seed_rejected_in_production_build() {
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let url = format!("http://{host}:{UNREACHABLE_PORT}/master.m3u8");
        assert_rejected_by_gate(&expand_seed_err(&url).await, &url);
    }
}

/// The `cfg(test)` exemption covers `http` and `https` alike, so both must be
/// refused once it is absent.
#[tokio::test]
async fn https_loopback_seed_rejected_in_production_build() {
    let url = format!("https://127.0.0.1:{UNREACHABLE_PORT}/master.m3u8");
    assert_rejected_by_gate(&expand_seed_err(&url).await, &url);
}

/// Link-local, including the cloud-metadata address. Covered by a unit test
/// too; repeated here because the unit-test version runs against a build whose
/// gate has an exemption compiled into it, and this one does not.
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
