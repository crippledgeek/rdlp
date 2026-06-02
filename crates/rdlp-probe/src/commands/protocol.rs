//! `rdlp-probe protocol` — report the HTTP version a host negotiates with the
//! emulating client.
//!
//! ALPN protocol selection happens during the TLS handshake, before any HTTP
//! response, so the negotiated version is observable from a single request to
//! *any* path on the host (even a 403 or redirect). This is the Tier-A signal
//! for the connectivity spike (PRD 2026-06-02 item 5): if a media CDN
//! negotiates **HTTP/1.1**, rdlp's N-connection AIMD download model works as
//! designed; if it negotiates **HTTP/2**, the N parallel range requests
//! multiplex onto a single TCP connection and the per-connection tuning is
//! partly moot.

use anyhow::{Context, Result};
use clap::Parser;

use rdlp_types::BrowserEmulation;

use super::{build_client, parse_method};

#[derive(Parser, Debug)]
pub struct Args {
    /// URL to probe.
    pub url: String,

    /// Browser emulation profile: `chrome` (default), `firefox`, `safari`,
    /// or a pinned identifier like `chrome-137`.
    #[arg(long = "browser", default_value = "chrome")]
    pub browser: String,

    /// HTTP method (default: GET). The negotiated protocol is independent of
    /// method/path — ALPN happens at the TLS layer.
    #[arg(short = 'X', long = "method", default_value = "GET")]
    pub method: String,
}

pub async fn run(args: Args) -> Result<()> {
    // BrowserEmulation::FromStr is infallible — see fetch.rs.
    let emulation: BrowserEmulation = args.browser.parse().expect("infallible");
    let client = build_client(emulation);

    let resp = client
        .request(parse_method(&args.method), &args.url)
        .send()
        .await
        .context("request failed")?;

    let version = resp.version();
    let status = resp.status();

    // Report to stdout — the negotiated version is the primary output. The
    // response body is intentionally not read; only the handshake + headers
    // matter here.
    // Flat `key: value` lines, matching the sibling `fetch` command's style.
    println!("url: {}", args.url);
    println!("protocol: {version:?}");
    println!(
        "status: {} {}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    for key in ["server", "alt-svc", "content-type"] {
        if let Some(v) = resp.headers().get(key) {
            println!("{key}: {}", v.to_str().unwrap_or("<binary>"));
        }
    }

    Ok(())
}
