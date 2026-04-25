pub mod eval;
pub mod extract;
pub mod fetch;
pub mod record;

use anyhow::{Context, Result};
use rdlp_http::{HttpClientConfig, HttpClientFactory};
use rdlp_types::BrowserEmulation;
use wreq::header::HeaderName;
use wreq::{Client, Method, RequestBuilder};

pub fn parse_emulation(s: &str) -> BrowserEmulation {
    match s.to_ascii_lowercase().as_str() {
        "chrome" | "chrome-latest" => BrowserEmulation::ChromeLatest,
        "firefox" | "firefox-latest" => BrowserEmulation::FirefoxLatest,
        "safari" | "safari-latest" => BrowserEmulation::SafariLatest,
        other => BrowserEmulation::Pinned(other.to_string()),
    }
}

pub fn build_client(emulation: BrowserEmulation) -> Client {
    let config = HttpClientConfig::default().with_emulation(emulation);
    HttpClientFactory::from_config(&config).build()
}

pub fn parse_method(s: &str) -> Method {
    Method::from_bytes(s.to_ascii_uppercase().as_bytes()).unwrap_or(Method::GET)
}

/// Apply `--header 'K: V'` flags and an optional body to a request.
pub fn apply_headers_and_body(
    mut req: RequestBuilder,
    headers: &[String],
    body: Option<&str>,
) -> Result<RequestBuilder> {
    for raw in headers {
        let (k, v) = raw
            .split_once(':')
            .with_context(|| format!("--header value missing ':' separator: {raw}"))?;
        let name = HeaderName::from_bytes(k.trim().as_bytes())
            .with_context(|| format!("invalid header name: {k}"))?;
        req = req.header(name, v.trim());
    }
    if let Some(b) = body {
        req = req.body(b.to_string());
    }
    Ok(req)
}
