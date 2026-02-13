//! Season/playlist extraction for 9anime.
//!
//! Downloads all episodes of an anime when the URL has no `?ep=` parameter.
//! Uses parallel extraction with bounded concurrency to resolve each episode's
//! video sources through the Megacloud/Rapid-Cloud chain.
//!
//! # Performance
//!
//! - Concurrency: 3 parallel episode extractions (lower than PornHub's 4
//!   due to Megacloud's tighter rate limits)
//! - Per-episode timeout: 60 seconds (accounts for cipher decryption)

use crate::base::common::MAX_PLAYLIST_SIZE;
use futures::stream::{self, StreamExt};
use log::{debug, info, warn};
use rdlp_core::{ExtractionContext, InfoDict, RdlpError, Result};
use scraper::Html;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;

use super::{api, build_subtitle_map, metadata, patterns, resolve_episode_formats};
use crate::base::common::BaseExtractor;

/// Timeout for extracting a single episode (60 seconds).
const EPISODE_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Number of concurrent episode extractions.
const CONCURRENT_EXTRACTIONS: usize = 3;

/// Extract all episodes of an anime as a playlist.
///
/// # Flow
///
/// 1. Parse anime_id from URL
/// 2. Fetch watch page for shared metadata (title, thumbnail, description)
/// 3. Fetch full episode list via AJAX
/// 4. Resolve formats for each episode in parallel
/// 5. Return sorted `Vec<InfoDict>` with playlist fields set
pub async fn extract_season(url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
    let anime_id = patterns::extract_anime_id(url).ok_or_else(|| {
        RdlpError::Extraction(format!("Could not extract anime ID from URL: {url}"))
    })?;

    info!(anime_id:%; "Extracting 9anime season (all episodes)");

    // Fetch watch page for shared metadata
    let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
    let anime_metadata = {
        let html = Html::parse_document(&webpage);
        metadata::extract_metadata(&html, &webpage)
    };

    let anime_title = anime_metadata.title.clone();
    info!(title:% = anime_title; "Anime metadata resolved");

    // Fetch full episode list
    let episodes = api::fetch_all_episodes(&anime_id, ctx).await?;

    if episodes.is_empty() {
        return Err(RdlpError::Extraction(format!(
            "No episodes found for anime ID {anime_id}"
        )));
    }

    let total = episodes.len();
    info!(total; "Found episodes in anime");

    // Security: limit playlist size
    if total > MAX_PLAYLIST_SIZE {
        return Err(RdlpError::Extraction(format!(
            "Playlist too large: {total} episodes (max: {MAX_PLAYLIST_SIZE})"
        )));
    }

    // Progress counter
    let completed = Arc::new(AtomicUsize::new(0));

    // Build extraction futures for each episode
    let extraction_futures = episodes.into_iter().enumerate().map(|(index, episode)| {
        let position = index + 1;
        let anime_title = anime_title.clone();
        let thumbnail = anime_metadata.thumbnail.clone();
        let description = anime_metadata.description.clone();
        let completed = Arc::clone(&completed);

        async move {
            let ep_label = format!(
                "Episode {} ({})",
                episode.info.number,
                episode.info.title.as_deref().unwrap_or("untitled")
            );

            debug!(position, total, episode:% = ep_label; "Extracting episode");

            let result = timeout(
                EPISODE_EXTRACTION_TIMEOUT,
                resolve_episode_formats(&episode.data_id, ctx),
            )
            .await;

            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;

            match result {
                Ok(Ok((formats, hls_flags, subtitle_tracks))) => {
                    if formats.is_empty() {
                        warn!(
                            done, total, episode:% = ep_label;
                            "No formats resolved for episode"
                        );
                        return None;
                    }

                    // Build episode title
                    let title = match &episode.info.title {
                        Some(ep_title) => format!(
                            "{anime_title} - Episode {} - {ep_title}",
                            episode.info.number
                        ),
                        None => format!("{anime_title} - Episode {}", episode.info.number),
                    };

                    let video_id = format!("{}-ep{}", episode.data_id, episode.info.number);

                    let mut info = InfoDict::new(
                        &video_id,
                        &title,
                        "9anime",
                        format!("https://9animetv.to/watch/episode-{}", episode.data_id),
                    );
                    info.formats = formats;
                    info.thumbnail = thumbnail;
                    info.description = description;
                    info.is_live = Some(hls_flags.is_live);

                    // Populate subtitles from Megacloud tracks
                    if !subtitle_tracks.is_empty() {
                        info.subtitles = Some(build_subtitle_map(&subtitle_tracks));
                    }

                    info.propagate_duration();

                    // Set playlist fields
                    info.playlist = Some(anime_title.clone());
                    info.playlist_title = Some(anime_title);
                    info.playlist_index = Some(position);
                    info.playlist_count = Some(total);

                    debug!(
                        done, total, episode:% = ep_label;
                        "Episode extraction succeeded"
                    );

                    Some((position, info))
                }
                Ok(Err(e)) => {
                    warn!(
                        done, total, episode:% = ep_label;
                        "Failed to extract episode: {e}"
                    );
                    None
                }
                Err(_) => {
                    warn!(
                        done, total, episode:% = ep_label;
                        "Timed out extracting episode"
                    );
                    None
                }
            }
        }
    });

    // Process extractions concurrently with bounded parallelism
    let results: Vec<Option<(usize, InfoDict)>> = stream::iter(extraction_futures)
        .buffer_unordered(CONCURRENT_EXTRACTIONS)
        .collect()
        .await;

    // Collect successful extractions and sort by playlist position
    let mut extracted: Vec<(usize, InfoDict)> = results.into_iter().flatten().collect();
    extracted.sort_by_key(|(pos, _)| *pos);

    let results: Vec<InfoDict> = extracted.into_iter().map(|(_, info)| info).collect();

    if results.is_empty() {
        return Err(RdlpError::Extraction(format!(
            "Failed to extract any episodes from anime: {url}"
        )));
    }

    info!(
        extracted = results.len(), total;
        "Successfully extracted episodes"
    );

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(CONCURRENT_EXTRACTIONS, 3);
        assert_eq!(EPISODE_EXTRACTION_TIMEOUT, Duration::from_secs(60));
    }
}
