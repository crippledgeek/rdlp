//! `rdlp-probe record` — fetch a URL and persist the (request, response) pair as JSON.
//!
//! Output shape is intentionally minimal and stable so it can be checked into
//! `crates/rdlp-extractor/tests/cassettes/<site>/` and replayed by parser tests
//! without network access.

use anyhow::{Context, Result};
use clap::Parser;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use rdlp_types::BrowserEmulation;

use super::{apply_headers_and_body, build_client, parse_method};

#[derive(Parser, Debug)]
pub struct Args {
    pub url: String,

    /// Output cassette path (created/overwritten).
    #[arg(short = 'o', long = "out")]
    pub out: PathBuf,

    #[arg(short = 'X', long = "method", default_value = "GET")]
    pub method: String,

    #[arg(short = 'd', long = "data")]
    pub data: Option<String>,

    #[arg(short = 'H', long = "header")]
    pub header: Vec<String>,

    #[arg(long = "browser", default_value = "chrome")]
    pub browser: String,

    /// Note recorded alongside the cassette (e.g. "video page, 2026-04-25").
    #[arg(long = "note")]
    pub note: Option<String>,
}

#[derive(Serialize)]
struct Cassette {
    url: String,
    method: String,
    request_headers: BTreeMap<String, String>,
    request_body: Option<String>,
    browser_emulation: String,
    recorded_at_unix: u64,
    note: Option<String>,
    status: u16,
    response_headers: BTreeMap<String, String>,
    response_body: String,
}

pub async fn run(args: Args) -> Result<()> {
    let emulation: BrowserEmulation = args.browser.parse().expect("infallible");
    let client = build_client(emulation);
    let req = client.request(parse_method(&args.method), &args.url);
    let req = apply_headers_and_body(req, &args.header, args.data.as_deref())?;

    let request_headers: BTreeMap<String, String> = args
        .header
        .iter()
        .filter_map(|h| h.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let resp = req.send().await.context("request failed")?;
    let status = resp.status();
    let response_headers: BTreeMap<String, String> = resp
        .headers()
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let response_body = resp.text().await.context("body read failed")?;

    let cassette = Cassette {
        url: args.url.clone(),
        method: args.method.to_uppercase(),
        request_headers,
        request_body: args.data,
        browser_emulation: args.browser,
        recorded_at_unix: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        note: args.note,
        status: status.as_u16(),
        response_headers,
        response_body,
    };

    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&cassette)?;
    tokio::fs::write(&args.out, json)
        .await
        .with_context(|| format!("write {}", args.out.display()))?;

    eprintln!(
        "recorded HTTP {} → {} ({} bytes)",
        status.as_u16(),
        args.out.display(),
        cassette.response_body.len()
    );
    Ok(())
}
