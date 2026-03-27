//! 9anime AJAX API helpers.
//!
//! Handles the server discovery and source resolution endpoints:
//! - `/ajax/episode/servers?episodeId={id}` — lists SUB/DUB streaming servers
//! - `/ajax/episode/sources?id={data-id}` — resolves to an embed iframe URL

use anyhow::Context as _;
use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use regex::Regex;
use std::sync::LazyLock;

/// Base URL for the 9anime site.
pub(crate) const BASE_URL: &str = "https://9animetv.to";

/// Audio type for a server entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioType {
    /// Subtitled (original audio)
    Sub,
    /// English dubbed
    Dub,
}

impl std::fmt::Display for AudioType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sub => write!(f, "SUB"),
            Self::Dub => write!(f, "DUB"),
        }
    }
}

/// A streaming server option returned by the servers endpoint.
#[derive(Debug, Clone)]
pub struct ServerEntry {
    /// Server data-id used to resolve the source URL.
    pub data_id: String,
    /// Server name (e.g., "Vidcloud", "Vidstreaming", "DouVideo").
    pub server_name: String,
    /// Numeric server ID.
    pub server_id: u32,
    /// Audio type (SUB or DUB).
    pub audio_type: AudioType,
}

/// Source resolution result from the sources endpoint.
#[derive(Debug, Clone)]
pub struct SourceResult {
    /// The embed iframe URL (e.g., `https://rapid-cloud.co/embed-2/v2/e-1/{id}?z=`).
    pub embed_url: String,
    /// Server ID.
    pub server_id: u32,
}

/// Pattern to match server-item blocks containing a `data-id` attribute.
///
/// Captures the full block from the opening `<div` (or any tag) with `data-id`
/// through to the next `</div>`. Attribute extraction is done separately to
/// handle any attribute ordering. The `(?s)` flag enables `.` to match newlines.
static SERVER_ITEM_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]*\bdata-id="\d+"[^>]*>.*?</div>"#)
        .expect("Valid server item block pattern")
});

/// Extract `data-id` value from a tag's attributes.
static DATA_ID_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bdata-id="(\d+)""#).expect("Valid data-id pattern"));

/// Extract `data-server-id` value from a tag's attributes.
static SERVER_ID_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bdata-server-id="(\d+)""#).expect("Valid server-id pattern"));

/// Extract `data-type` value from a tag's attributes.
static DATA_TYPE_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bdata-type="([^"]+)""#).expect("Valid data-type pattern"));

/// Fetch the list of streaming servers for an episode.
///
/// Calls `/ajax/episode/servers?episodeId={episode_id}` and parses
/// the HTML response for SUB and DUB server entries.
pub async fn fetch_servers(episode_id: &str, ctx: &ExtractionContext) -> Result<Vec<ServerEntry>> {
    fetch_servers_impl(episode_id, ctx).await.map_err(|e| RdlpError::Extraction {
        message: format!("{e:#}"),
        url: None,
    })
}

async fn fetch_servers_impl(episode_id: &str, ctx: &ExtractionContext) -> anyhow::Result<Vec<ServerEntry>> {
    let url = format!("{BASE_URL}/ajax/episode/servers?episodeId={episode_id}");
    debug!(url:%; "Fetching 9anime servers");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .with_context(|| format!("failed to fetch 9anime servers for episode {episode_id}"))?;

    let json: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("failed to parse 9anime servers JSON for episode {episode_id}"))?;

    let html = json["html"].as_str().unwrap_or_default();

    debug!(html_len = html.len(); "Received servers HTML");

    let servers = parse_server_items(html);

    debug!(count = servers.len(); "Found 9anime servers");
    Ok(servers)
}

/// Extract the first non-empty text content from within HTML tags.
///
/// Scans for `>text<` pairs and returns the first non-whitespace match.
/// Handles nested elements like `<div ...><a ...>ServerName</a></div>`.
fn extract_inner_text(html: &str) -> Option<String> {
    static INNER_TEXT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#">([^<]+)<"#).expect("Valid inner text pattern"));

    INNER_TEXT.captures_iter(html).find_map(|c| {
        let text = c[1].trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    })
}

/// Parse all server items from the AJAX HTML response.
///
/// Real 9anime HTML has server-item divs with `data-id`, `data-server-id`,
/// and `data-type` attributes (in any order), with the server name inside a
/// child `<a>` element. We match the full `<div>...</div>` block and extract
/// attributes + inner text independently.
fn parse_server_items(html: &str) -> Vec<ServerEntry> {
    let mut servers = Vec::new();

    for block_match in SERVER_ITEM_BLOCK.find_iter(html) {
        let block = block_match.as_str();

        let Some(data_id_caps) = DATA_ID_ATTR.captures(block) else {
            continue;
        };
        let data_id = data_id_caps[1].to_string();

        let server_id: u32 = SERVER_ID_ATTR
            .captures(block)
            .and_then(|c| c[1].parse().ok())
            .unwrap_or(0);

        let audio_type = match DATA_TYPE_ATTR.captures(block) {
            Some(c) if c[1].eq_ignore_ascii_case("dub") => AudioType::Dub,
            _ => AudioType::Sub,
        };

        let Some(server_name) = extract_inner_text(block) else {
            continue;
        };

        // Avoid duplicates
        if !servers.iter().any(|s: &ServerEntry| s.data_id == data_id) {
            servers.push(ServerEntry {
                data_id,
                server_name,
                server_id,
                audio_type,
            });
        }
    }

    servers
}

/// Preferred server order for fallback.
const PREFERRED_SERVERS: &[&str] = &["Vidcloud", "Vidstreaming", "DouVideo"];

/// Sort servers by preference (Vidcloud first, then Vidstreaming, then others).
pub fn sort_by_preference(servers: &mut [ServerEntry]) {
    servers.sort_by_key(|s| {
        PREFERRED_SERVERS
            .iter()
            .position(|p| {
                s.server_name
                    .as_bytes()
                    .windows(p.len())
                    .any(|w| w.eq_ignore_ascii_case(p.as_bytes()))
            })
            .unwrap_or(PREFERRED_SERVERS.len())
    });
}

/// Resolve a server data-id to an embed iframe URL.
///
/// Calls `/ajax/episode/sources?id={data_id}` and returns the embed URL.
pub async fn fetch_source(data_id: &str, ctx: &ExtractionContext) -> Result<SourceResult> {
    fetch_source_impl(data_id, ctx).await.map_err(|e| RdlpError::Extraction {
        message: format!("{e:#}"),
        url: None,
    })
}

async fn fetch_source_impl(data_id: &str, ctx: &ExtractionContext) -> anyhow::Result<SourceResult> {
    let url = format!("{BASE_URL}/ajax/episode/sources?id={data_id}");
    debug!(url:%; "Fetching 9anime source");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .with_context(|| format!("failed to fetch 9anime source for data_id={data_id}"))?;

    let json: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("failed to parse 9anime source JSON for data_id={data_id}"))?;

    let embed_url = json["link"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no 'link' field in 9anime source response for data_id={data_id}"))?
        .to_string();

    let server_id = json["server"].as_u64().unwrap_or(0) as u32;

    if embed_url.is_empty() {
        return Err(anyhow::anyhow!("empty embed URL in 9anime source response for data_id={data_id}"));
    }

    debug!(embed_url:%, server_id; "Resolved 9anime source");
    Ok(SourceResult {
        embed_url,
        server_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_server_items_real_html() {
        // Real 9anime HTML: div with data attrs, server name inside child <a>
        let html = r#"
            <div class="ps_-block servers-sub">
                <div class="ps__-list">
                    <div class="item server-item" data-type="sub" data-id="579510" data-server-id="4">
                        <a href="javascript:;" class="btn">Vidstreaming</a>
                    </div>
                    <div class="item server-item" data-type="sub" data-id="13548" data-server-id="1">
                        <a href="javascript:;" class="btn">Vidcloud</a>
                    </div>
                    <div class="item server-item" data-type="sub" data-id="1172700" data-server-id="6">
                        <a href="javascript:;" class="btn">DouVideo</a>
                    </div>
                </div>
            </div>
            <div class="ps_-block servers-dub">
                <div class="ps__-list">
                    <div class="item server-item" data-type="dub" data-id="617130" data-server-id="4">
                        <a href="javascript:;" class="btn">Vidstreaming</a>
                    </div>
                    <div class="item server-item" data-type="dub" data-id="149364" data-server-id="1">
                        <a href="javascript:;" class="btn">Vidcloud</a>
                    </div>
                </div>
            </div>
        "#;
        let servers = parse_server_items(html);
        assert_eq!(servers.len(), 5);

        // SUB servers
        assert_eq!(servers[0].data_id, "579510");
        assert_eq!(servers[0].server_name, "Vidstreaming");
        assert_eq!(servers[0].server_id, 4);
        assert_eq!(servers[0].audio_type, AudioType::Sub);

        assert_eq!(servers[1].data_id, "13548");
        assert_eq!(servers[1].server_name, "Vidcloud");
        assert_eq!(servers[1].audio_type, AudioType::Sub);

        assert_eq!(servers[2].data_id, "1172700");
        assert_eq!(servers[2].server_name, "DouVideo");
        assert_eq!(servers[2].audio_type, AudioType::Sub);

        // DUB servers
        assert_eq!(servers[3].data_id, "617130");
        assert_eq!(servers[3].audio_type, AudioType::Dub);

        assert_eq!(servers[4].data_id, "149364");
        assert_eq!(servers[4].audio_type, AudioType::Dub);
    }

    #[test]
    fn test_parse_server_items_direct_text() {
        // Also handles direct text content (no child element)
        let html = r#"
            <div class="item" data-id="13548" data-server-id="1" data-type="sub">Vidcloud</div>
            <div class="item" data-id="579510" data-server-id="4" data-type="dub">Vidstreaming</div>
        "#;
        let servers = parse_server_items(html);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].data_id, "13548");
        assert_eq!(servers[0].server_name, "Vidcloud");
        assert_eq!(servers[0].audio_type, AudioType::Sub);
        assert_eq!(servers[1].data_id, "579510");
        assert_eq!(servers[1].server_name, "Vidstreaming");
        assert_eq!(servers[1].audio_type, AudioType::Dub);
    }

    #[test]
    fn test_sort_by_preference() {
        let mut servers = vec![
            ServerEntry {
                data_id: "3".to_string(),
                server_name: "DouVideo".to_string(),
                server_id: 6,
                audio_type: AudioType::Sub,
            },
            ServerEntry {
                data_id: "1".to_string(),
                server_name: "Vidstreaming".to_string(),
                server_id: 4,
                audio_type: AudioType::Sub,
            },
            ServerEntry {
                data_id: "2".to_string(),
                server_name: "Vidcloud".to_string(),
                server_id: 1,
                audio_type: AudioType::Sub,
            },
        ];
        sort_by_preference(&mut servers);
        assert_eq!(servers[0].server_name, "Vidcloud");
        assert_eq!(servers[1].server_name, "Vidstreaming");
        assert_eq!(servers[2].server_name, "DouVideo");
    }

    #[test]
    fn test_audio_type_display() {
        assert_eq!(AudioType::Sub.to_string(), "SUB");
        assert_eq!(AudioType::Dub.to_string(), "DUB");
    }
}
