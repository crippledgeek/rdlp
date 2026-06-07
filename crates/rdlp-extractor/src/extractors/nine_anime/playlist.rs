//! Season/playlist extraction for 9anime.
//!
//! Downloads all episodes of an anime when the URL has no `?ep=` parameter.
//! One episode is fully resolved during extraction (for audio type detection
//! and subtitle info). If the first episode fails (CDN error, Cloudflare),
//! up to 3 episodes are tried in order. Remaining episodes are built as
//! lightweight `InfoDict` stubs with `webpage_url` set, enabling lazy
//! resolution at download time.
//!
//! # Performance
//!
//! - Playlist prompt appears in ~5 seconds (vs ~68s with eager extraction)
//! - Each episode resolves formats just before download (~3-5s per episode)

use crate::base::common::MAX_PLAYLIST_SIZE;
use log::{debug, info};
use rdlp_core::{ExtractionContext, RdlpError, Result};
use rdlp_types::InfoDict;
use scraper::Html;

use super::{build_subtitle_map, episodes, metadata, patterns, resolve_episode_formats};
use crate::base::common::BaseExtractor;

/// Extract all episodes of an anime as a playlist.
///
/// One episode is fully resolved (formats + subtitles) for audio type detection.
/// If the first episode's CDN resolution fails, up to 3 episodes are tried.
/// Remaining episodes are returned as metadata-only stubs with `webpage_url`
/// set so the orchestrator can lazily resolve them at download time.
///
/// # Flow
///
/// 1. Parse anime_id and slug from URL
/// 2. Fetch watch page for shared metadata (title, thumbnail, description)
/// 3. Fetch full episode list via AJAX
/// 4. Fully resolve **one episode** via `resolve_episode_formats()` (tries up to 3)
/// 5. Build remaining episodes as lightweight `InfoDict` with proper `webpage_url`
/// 6. Return `Vec<InfoDict>` with playlist fields set
///
/// # Errors
///
/// Returns `Err` when:
/// - Anime ID cannot be extracted from URL (`RdlpError::Extraction`)
/// - Webpage fetch fails (`RdlpError::Network`)
/// - Episode list fetch fails (`RdlpError::Extraction`)
/// - No episodes found for the anime (`RdlpError::Extraction`)
/// - Playlist size exceeds `MAX_PLAYLIST_SIZE` (`RdlpError::Extraction`)
/// - All episode resolution attempts fail (`RdlpError::Extraction`)
pub async fn extract_season(url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
    let anime_id = patterns::extract_anime_id(url).ok_or_else(|| RdlpError::Extraction {
        message: format!(
            "Could not extract anime ID from URL: {}",
            rdlp_redact::RedactedUrl::new(&url)
        ),
        url: Some(url.to_string().into()),
    })?;

    let slug = patterns::extract_slug(url).unwrap_or_default();

    info!(anime_id:%; "Extracting 9anime season (all episodes)");

    // Fetch watch page for shared metadata
    let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
    let anime_metadata = {
        let html = Html::parse_document(&webpage);
        metadata::extract_metadata(&html, &webpage)
    };

    let anime_title = anime_metadata.title.clone();
    debug!(title:% = anime_title; "Anime metadata resolved");

    // Fetch full episode list
    let episodes = episodes::fetch_all_episodes(&anime_id, ctx).await?;

    if episodes.is_empty() {
        return Err(RdlpError::Extraction {
            message: format!("No episodes found for anime ID {anime_id}"),
            url: Some(url.to_string().into()),
        });
    }

    let total = episodes.len();
    debug!(total; "Found episodes in anime");

    // Security: limit playlist size
    if total > MAX_PLAYLIST_SIZE {
        return Err(RdlpError::Extraction {
            message: format!("Playlist too large: {total} episodes (max: {MAX_PLAYLIST_SIZE})"),
            url: Some(url.to_string().into()),
        });
    }

    // Resolve one episode fully for audio type detection + subtitles.
    // Try episodes in order until one succeeds — CDN failures on the first
    // episode shouldn't prevent SUB/DUB and subtitle selection.
    const MAX_PROBE_EPISODES: usize = 3;
    let probe_limit = MAX_PROBE_EPISODES.min(total);
    let mut probe_result = None;
    let mut probe_index = 0;

    for (i, ep) in episodes.iter().enumerate().take(probe_limit) {
        let label = format!(
            "Episode {} ({})",
            ep.info.number,
            ep.info.title.as_deref().unwrap_or("untitled")
        );
        debug!(episode:% = label; "Resolving episode for type detection");

        match resolve_episode_formats(&ep.data_id, ctx).await {
            Ok((formats, hls_flags, subtitle_tracks)) if !formats.is_empty() => {
                debug!(
                    episode:% = label,
                    formats = formats.len();
                    "Episode resolved successfully for type detection"
                );
                probe_result = Some((formats, hls_flags, subtitle_tracks));
                probe_index = i;
                break;
            }
            Ok(_) => {
                debug!(episode:% = label; "No formats resolved, trying next episode");
            }
            Err(e) => {
                debug!(episode:% = label; "Failed to resolve episode: {e}");
            }
        }
    }

    let mut results: Vec<InfoDict> = Vec::with_capacity(total);

    // Build InfoDict for each episode
    for (index, episode) in episodes.iter().enumerate() {
        let position = index + 1;

        let title = match &episode.info.title {
            Some(ep_title) => format!(
                "{anime_title} - Episode {} - {ep_title}",
                episode.info.number
            ),
            None => format!("{anime_title} - Episode {}", episode.info.number),
        };

        let video_id = format!("{}-ep{}", episode.data_id, episode.info.number);
        let webpage_url = format!(
            "https://9animetv.to/watch/{slug}-{anime_id}?ep={}",
            episode.data_id
        );

        let mut info = InfoDict::new(&video_id, &title, "9anime", &webpage_url);
        info.thumbnail = anime_metadata.thumbnail.clone();
        info.description = anime_metadata.description.clone();

        // The probed episode gets full formats + subtitles
        if index == probe_index
            && let Some((ref formats, ref hls_flags, ref subtitle_tracks)) = probe_result
        {
            info.formats = formats.clone();
            info.is_live = Some(hls_flags.is_live);

            if !subtitle_tracks.is_empty() {
                info.subtitles = Some(build_subtitle_map(subtitle_tracks));
            }

            info.propagate_duration();
        }
        // All other episodes: empty formats (lazy resolution marker)

        // Set playlist fields
        info.playlist = Some(anime_title.clone());
        info.playlist_title = Some(anime_title.clone());
        info.playlist_index = Some(position);
        info.playlist_count = Some(total);

        results.push(info);
    }

    if results.is_empty() {
        return Err(RdlpError::Extraction {
            message: format!(
                "Failed to extract any episodes from anime: {}",
                rdlp_redact::RedactedUrl::new(&url)
            ),
            url: Some(url.to_string().into()),
        });
    }

    info!(
        total = results.len(),
        probed = probe_result.is_some();
        "Playlist extraction complete (one episode probed, rest deferred)"
    );

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webpage_url_format() {
        let slug = "sword-art-online";
        let anime_id = "2274";
        let data_id = "26565";
        let url = format!("https://9animetv.to/watch/{slug}-{anime_id}?ep={data_id}");
        assert!(patterns::has_episode_param(&url));
        assert_eq!(patterns::extract_anime_id(&url), Some("2274".to_string()));
        assert_eq!(
            patterns::extract_episode_id(&url),
            Some("26565".to_string())
        );
    }
}
