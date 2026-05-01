//! Episode list parsing for 9anime.
//!
//! Handles fetching and parsing the episode list from the AJAX API,
//! including individual episode info lookup and full episode list parsing.

use crate::utils::decode_html_entities;
use anyhow::Context as _;
use lazy_regex::{Lazy, Regex, lazy_regex};
use log::debug;
use rdlp_core::{ExtractionContext, RdlpError, Result};

use super::api::BASE_URL;

/// Episode info resolved from the episode list.
#[derive(Debug, Clone)]
pub struct EpisodeInfo {
    /// Sequential episode number (e.g., 1, 2, 3).
    pub number: String,
    /// Episode title (e.g., "The World of Swords").
    pub title: Option<String>,
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

/// Pattern to match episode items: `<a ... data-id="26565" data-number="1" title="The World of Swords" ...>`
static EP_ITEM_DATA_ID: Lazy<Regex> =
    lazy_regex!(r#"\bdata-id="(\d+)""#);

/// Extract `data-number` from an episode item.
static EP_DATA_NUMBER: Lazy<Regex> =
    lazy_regex!(r#"\bdata-number="(\d+)""#);

/// Extract `title` from an episode item.
static EP_TITLE_ATTR: Lazy<Regex> =
    lazy_regex!(r#"\btitle="([^"]+)""#);

/// Pattern to match episode `<a>` blocks.
static EP_ITEM_BLOCK: Lazy<Regex> =
    lazy_regex!(r#"(?s)<a[^>]*\bclass="[^"]*ep-item[^"]*"[^>]*>.*?</a>"#);

/// Fetch episode info for a specific episode data-id.
///
/// Calls `/ajax/episode/list/{anime_id}` and finds the episode matching
/// the given `episode_data_id` to extract its number and title.
pub async fn fetch_episode_info(
    anime_id: &str,
    episode_data_id: &str,
    ctx: &ExtractionContext,
) -> Result<Option<EpisodeInfo>> {
    fetch_episode_info_impl(anime_id, episode_data_id, ctx)
        .await
        .map_err(|e| RdlpError::Extraction {
            message: format!("{e:#}"),
            url: None,
        })
}

async fn fetch_episode_info_impl(
    anime_id: &str,
    episode_data_id: &str,
    ctx: &ExtractionContext,
) -> anyhow::Result<Option<EpisodeInfo>> {
    let url = format!("{BASE_URL}/ajax/episode/list/{anime_id}");
    debug!(url:%; "Fetching 9anime episode list");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .with_context(|| format!("failed to fetch 9anime episode list for anime_id={anime_id}"))?;

    let json: serde_json::Value = response.json().await.with_context(|| {
        format!("failed to parse 9anime episode list JSON for anime_id={anime_id}")
    })?;

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
    fetch_all_episodes_impl(anime_id, ctx)
        .await
        .map_err(|e| RdlpError::Extraction {
            message: format!("{e:#}"),
            url: None,
        })
}

async fn fetch_all_episodes_impl(
    anime_id: &str,
    ctx: &ExtractionContext,
) -> anyhow::Result<Vec<EpisodeListEntry>> {
    let url = format!("{BASE_URL}/ajax/episode/list/{anime_id}");
    debug!(url:%; "Fetching 9anime full episode list");

    let response = ctx
        .http_client
        .get(&url)
        .header("Referer", BASE_URL)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .with_context(|| {
            format!("failed to fetch 9anime full episode list for anime_id={anime_id}")
        })?;

    let json: serde_json::Value = response.json().await.with_context(|| {
        format!("failed to parse 9anime episode list JSON for anime_id={anime_id}")
    })?;

    let html = json["html"].as_str().unwrap_or_default();
    let episodes = parse_all_episodes(html);

    debug!(count = episodes.len(); "Parsed full episode list");
    Ok(episodes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
