//! 9anime AJAX API helpers.
//!
//! Handles the server discovery and source resolution endpoints:
//! - `/ajax/episode/servers?episodeId={id}` — lists SUB/DUB streaming servers
//! - `/ajax/episode/sources?id={data-id}` — resolves to an embed iframe URL

use crate::utils::decode_html_entities;
use log::debug;
use once_cell::sync::Lazy;
use rdlp_core::{ExtractionContext, RdlpError, Result};
use regex::Regex;

/// Base URL for the 9anime site.
const BASE_URL: &str = "https://9animetv.to";

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
static SERVER_ITEM_BLOCK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<div[^>]*\bdata-id="\d+"[^>]*>.*?</div>"#)
        .expect("Valid server item block pattern")
});

/// Extract `data-id` value from a tag's attributes.
static DATA_ID_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdata-id="(\d+)""#).expect("Valid data-id pattern"));

/// Extract `data-server-id` value from a tag's attributes.
static SERVER_ID_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdata-server-id="(\d+)""#).expect("Valid server-id pattern"));

/// Extract `data-type` value from a tag's attributes.
static DATA_TYPE_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdata-type="([^"]+)""#).expect("Valid data-type pattern"));

/// Fetch the list of streaming servers for an episode.
///
/// Calls `/ajax/episode/servers?episodeId={episode_id}` and parses
/// the HTML response for SUB and DUB server entries.
pub async fn fetch_servers(episode_id: &str, ctx: &ExtractionContext) -> Result<Vec<ServerEntry>> {
    let url = format!("{BASE_URL}/ajax/episode/servers?episodeId={episode_id}");
    debug!(url:%; "Fetching 9anime servers");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch servers: {e}")))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| RdlpError::Extraction(format!("Failed to parse servers JSON: {e}")))?;

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
    static INNER_TEXT: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#">([^<]+)<"#).expect("Valid inner text pattern"));

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
    let url = format!("{BASE_URL}/ajax/episode/sources?id={data_id}");
    debug!(url:%; "Fetching 9anime source");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch source: {e}")))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| RdlpError::Extraction(format!("Failed to parse source JSON: {e}")))?;

    let embed_url = json["link"]
        .as_str()
        .ok_or_else(|| RdlpError::Extraction("No 'link' field in source response".to_string()))?
        .to_string();

    let server_id = json["server"].as_u64().unwrap_or(0) as u32;

    if embed_url.is_empty() {
        return Err(RdlpError::Extraction(
            "Empty embed URL in source response".to_string(),
        ));
    }

    debug!(embed_url:%, server_id; "Resolved 9anime source");
    Ok(SourceResult {
        embed_url,
        server_id,
    })
}

/// Episode info resolved from the episode list.
#[derive(Debug, Clone)]
pub struct EpisodeInfo {
    /// Sequential episode number (e.g., 1, 2, 3).
    pub number: String,
    /// Episode title (e.g., "The World of Swords").
    pub title: Option<String>,
}

/// Pattern to match episode items: `<a ... data-id="26565" data-number="1" title="The World of Swords" ...>`
static EP_ITEM_DATA_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdata-id="(\d+)""#).expect("Valid ep data-id pattern"));

/// Extract `data-number` from an episode item.
static EP_DATA_NUMBER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdata-number="(\d+)""#).expect("Valid ep data-number pattern"));

/// Extract `title` from an episode item.
static EP_TITLE_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\btitle="([^"]+)""#).expect("Valid ep title pattern"));

/// Pattern to match episode `<a>` blocks.
static EP_ITEM_BLOCK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<a[^>]*\bclass="[^"]*ep-item[^"]*"[^>]*>.*?</a>"#)
        .expect("Valid ep-item block pattern")
});

/// Fetch episode info for a specific episode data-id.
///
/// Calls `/ajax/episode/list/{anime_id}` and finds the episode matching
/// the given `episode_data_id` to extract its number and title.
pub async fn fetch_episode_info(
    anime_id: &str,
    episode_data_id: &str,
    ctx: &ExtractionContext,
) -> Result<Option<EpisodeInfo>> {
    let url = format!("{BASE_URL}/ajax/episode/list/{anime_id}");
    debug!(url:%; "Fetching 9anime episode list");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch episode list: {e}")))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| RdlpError::Extraction(format!("Failed to parse episode list: {e}")))?;

    let html = json["html"].as_str().unwrap_or_default();

    Ok(parse_episode_info(html, episode_data_id))
}

/// Parse episode info from the episode list HTML.
fn parse_episode_info(html: &str, episode_data_id: &str) -> Option<EpisodeInfo> {
    for block_match in EP_ITEM_BLOCK.find_iter(html) {
        let block = block_match.as_str();

        // Check if this block's data-id matches
        let id_caps = EP_ITEM_DATA_ID.captures(block)?;
        if &id_caps[1] != episode_data_id {
            continue;
        }

        let number = EP_DATA_NUMBER.captures(block).map(|c| c[1].to_string())?;

        let title = EP_TITLE_ATTR
            .captures(block)
            .map(|c| decode_html_entities(&c[1]))
            .filter(|t| !t.is_empty());

        return Some(EpisodeInfo { number, title });
    }

    None
}

/// A single episode entry from the full episode list.
///
/// Combines the episode's `data-id` (needed for server resolution) with
/// its metadata (number, title).
#[derive(Debug, Clone)]
pub struct EpisodeListEntry {
    /// Episode data-id used for server/source resolution.
    pub data_id: String,
    /// Episode metadata (number and title).
    pub info: EpisodeInfo,
}

/// Parse all episodes from the episode list HTML.
///
/// Returns every `ep-item` block as an `EpisodeListEntry` with data-id,
/// episode number, and title.
pub fn parse_all_episodes(html: &str) -> Vec<EpisodeListEntry> {
    let mut entries = Vec::new();

    for block_match in EP_ITEM_BLOCK.find_iter(html) {
        let block = block_match.as_str();

        let Some(id_caps) = EP_ITEM_DATA_ID.captures(block) else {
            continue;
        };
        let Some(num_caps) = EP_DATA_NUMBER.captures(block) else {
            continue;
        };

        let title = EP_TITLE_ATTR
            .captures(block)
            .map(|c| decode_html_entities(&c[1]))
            .filter(|t| !t.is_empty());

        entries.push(EpisodeListEntry {
            data_id: id_caps[1].to_string(),
            info: EpisodeInfo {
                number: num_caps[1].to_string(),
                title,
            },
        });
    }

    entries
}

/// Fetch the full episode list for an anime.
///
/// Calls `/ajax/episode/list/{anime_id}` and parses all episode entries.
pub async fn fetch_all_episodes(
    anime_id: &str,
    ctx: &ExtractionContext,
) -> Result<Vec<EpisodeListEntry>> {
    let url = format!("{BASE_URL}/ajax/episode/list/{anime_id}");
    debug!(url:%; "Fetching 9anime full episode list");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| RdlpError::Network(format!("Failed to fetch episode list: {e}")))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| RdlpError::Extraction(format!("Failed to parse episode list: {e}")))?;

    let html = json["html"].as_str().unwrap_or_default();
    let episodes = parse_all_episodes(html);

    debug!(count = episodes.len(); "Parsed full episode list");
    Ok(episodes)
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

    #[test]
    fn test_parse_episode_info() {
        let html = r#"
            <a href="/watch/sword-art-online-2274?ep=26565"
               title="The World of Swords"
               class="item ep-item"
               data-number="1"
               data-id="26565">
              <div class="order">1</div>
            </a>
            <a href="/watch/sword-art-online-2274?ep=26566"
               title="Beater"
               class="item ep-item"
               data-number="2"
               data-id="26566">
              <div class="order">2</div>
            </a>
        "#;

        let info = parse_episode_info(html, "26565").unwrap();
        assert_eq!(info.number, "1");
        assert_eq!(info.title.as_deref(), Some("The World of Swords"));

        let info2 = parse_episode_info(html, "26566").unwrap();
        assert_eq!(info2.number, "2");
        assert_eq!(info2.title.as_deref(), Some("Beater"));

        assert!(parse_episode_info(html, "99999").is_none());
    }

    #[test]
    fn test_parse_all_episodes() {
        let html = r#"
            <a href="/watch/sword-art-online-2274?ep=26565"
               title="The World of Swords"
               class="item ep-item"
               data-number="1"
               data-id="26565">
              <div class="order">1</div>
            </a>
            <a href="/watch/sword-art-online-2274?ep=26566"
               title="Beater"
               class="item ep-item"
               data-number="2"
               data-id="26566">
              <div class="order">2</div>
            </a>
            <a href="/watch/sword-art-online-2274?ep=26567"
               title="The Red-Nosed Reindeer"
               class="item ep-item"
               data-number="3"
               data-id="26567">
              <div class="order">3</div>
            </a>
        "#;

        let episodes = parse_all_episodes(html);
        assert_eq!(episodes.len(), 3);

        assert_eq!(episodes[0].data_id, "26565");
        assert_eq!(episodes[0].info.number, "1");
        assert_eq!(
            episodes[0].info.title.as_deref(),
            Some("The World of Swords")
        );

        assert_eq!(episodes[1].data_id, "26566");
        assert_eq!(episodes[1].info.number, "2");
        assert_eq!(episodes[1].info.title.as_deref(), Some("Beater"));

        assert_eq!(episodes[2].data_id, "26567");
        assert_eq!(episodes[2].info.number, "3");
        assert_eq!(
            episodes[2].info.title.as_deref(),
            Some("The Red-Nosed Reindeer")
        );
    }

    #[test]
    fn test_parse_all_episodes_empty() {
        assert!(parse_all_episodes("").is_empty());
        assert!(parse_all_episodes("<div>no episodes</div>").is_empty());
    }

    #[test]
    fn test_parse_episode_title_html_entities() {
        let html = r#"
            <a href="/watch/sailor-moon-1067?ep=40198"
               title="Usagi&#39;s Disaster: Beware of the Clock of Confusion"
               class="item ep-item"
               data-number="15"
               data-id="40198">
              <div class="order">15</div>
            </a>
        "#;

        let info = parse_episode_info(html, "40198").unwrap();
        assert_eq!(info.number, "15");
        assert_eq!(
            info.title.as_deref(),
            Some("Usagi's Disaster: Beware of the Clock of Confusion")
        );

        let episodes = parse_all_episodes(html);
        assert_eq!(episodes.len(), 1);
        assert_eq!(
            episodes[0].info.title.as_deref(),
            Some("Usagi's Disaster: Beware of the Clock of Confusion")
        );
    }
}
