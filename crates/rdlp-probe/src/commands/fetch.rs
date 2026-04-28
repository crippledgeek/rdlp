//! `rdlp-probe fetch` — request a URL through the production HTTP stack.

use anyhow::{Context, Result};
use clap::Parser;

use rdlp_types::BrowserEmulation;

use super::{apply_headers_and_body, build_client, parse_method};

#[derive(Parser, Debug)]
pub struct Args {
    /// URL to fetch.
    pub url: String,

    /// HTTP method (default: GET).
    #[arg(short = 'X', long = "method", default_value = "GET")]
    pub method: String,

    /// Request body (sent as-is). Use `--header 'Content-Type: ...'` to set the type.
    #[arg(short = 'd', long = "data")]
    pub data: Option<String>,

    /// Repeatable header `'Name: value'`.
    #[arg(short = 'H', long = "header")]
    pub header: Vec<String>,

    /// Browser emulation profile: `chrome` (default), `firefox`, `safari`,
    /// or a pinned identifier like `chrome-137`.
    #[arg(long = "browser", default_value = "chrome")]
    pub browser: String,

    /// Print every response header instead of the curated subset.
    #[arg(long = "headers")]
    pub all_headers: bool,
}

pub async fn run(args: Args) -> Result<()> {
    // BrowserEmulation::FromStr is infallible — unknown identifiers become
    // `Pinned(_)` and are validated lazily at resolve() time.
    let emulation: BrowserEmulation = args.browser.parse().expect("infallible");
    let client = build_client(emulation);
    let req = client.request(parse_method(&args.method), &args.url);
    let req = apply_headers_and_body(req, &args.header, args.data.as_deref())?;

    let resp = req.send().await.context("request failed")?;
    let status = resp.status();
    eprintln!(
        "HTTP {} {}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );

    if args.all_headers {
        for (name, value) in resp.headers() {
            eprintln!("{name}: {}", value.to_str().unwrap_or("<binary>"));
        }
    } else {
        for key in [
            "content-type",
            "content-length",
            "cf-ray",
            "server",
            "cf-cache-status",
            "cf-mitigated",
            "set-cookie",
            "location",
        ] {
            if let Some(v) = resp.headers().get(key) {
                eprintln!("{key}: {}", v.to_str().unwrap_or("<binary>"));
            }
        }
    }

    let body = resp.text().await.context("body read failed")?;
    print!("{body}");

    if status.is_success() {
        Ok(())
    } else {
        anyhow::bail!("non-2xx status: {}", status.as_u16());
    }
}
