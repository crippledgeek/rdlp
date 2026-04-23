//! 9anime extractor module.
//!
//! Extracts anime episodes from `9animetv.to` using the site's AJAX API
//! and Megacloud/Rapid-Cloud embed resolution.
//!
//! # Architecture
//!
//! The extractor is split into focused submodules:
//! - `patterns` - URL patterns and regex definitions
//! - `metadata` - HTML metadata extraction (title, thumbnail, description)
//! - `api` - AJAX endpoint helpers (server list, source resolution)
//! - `megacloud` - Embed page fetch, client key extraction, source decryption
//! - `playlist` - Season/full-anime download support
//!
//! # Extraction Flow
//!
//! 1. Parse anime ID and episode ID from the URL
//! 2. Fetch the episode page HTML for metadata
//! 3. Call `/ajax/episode/servers` for available streaming servers
//! 4. For each server (Vidcloud → Vidstreaming → DouVideo fallback):
//!    - Call `/ajax/episode/sources` to get the embed iframe URL
//!    - Resolve actual HLS URLs from the Megacloud embed
//! 5. Build `Format` objects for each quality variant
//!
//! # Supported URLs
//!
//! - `https://9animetv.to/watch/{slug}-{id}?ep={ep-id}` — single episode
//! - `https://9animetv.to/watch/{slug}-{id}` — all episodes (season)

pub mod api;
pub mod episodes;
pub mod megacloud;
pub mod metadata;
pub mod patterns;
pub mod playlist;
pub mod search;
pub mod search_patterns;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::{DownloadProtocol, Format, InfoDict};
use scraper::Html;

use std::collections::HashMap;

use crate::base::common::BaseExtractor;
use crate::hls::{HlsStreamFlags, detect_format_sizes_lazy};

/// 9anime episode extractor.
///
/// Resolves anime episode video sources through 9anime's AJAX API chain
/// and Megacloud/Rapid-Cloud embed decryption.
///
/// # Example
///
/// ```no_run
/// use rdlp_extractor::NineAnimeExtractor;
/// use rdlp_core::InfoExtractor;
///
/// let extractor = NineAnimeExtractor::new();
/// assert!(extractor.suitable("https://9animetv.to/watch/sword-art-online-2274?ep=26565"));
/// ```
pub struct NineAnimeExtractor;

impl NineAnimeExtractor {
    /// Create a new 9anime extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NineAnimeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve video formats for a single episode by its episode data-id.
///
/// Fetches servers, tries each in preference order (Vidcloud → Vidstreaming
/// → DouVideo), extracts HLS sources from Megacloud, and enriches formats
/// with resolution/codec/duration metadata.
///
/// Shared between single-episode `extract()` and playlist `extract_season()`.
pub(crate) async fn resolve_episode_formats(
    episode_id: &str,
    ctx: &ExtractionContext,
) -> Result<(Vec<Format>, HlsStreamFlags, Vec<megacloud::SubtitleTrack>)> {
    let mut servers = api::fetch_servers(episode_id, ctx).await?;
    if servers.is_empty() {
        return Err(RdlpError::Extraction {
            message: "No streaming servers found for this episode".to_string(),
            url: None,
        });
    }

    api::sort_by_preference(&mut servers);

    debug!(
        servers = servers.len();
        "Found streaming servers, attempting extraction"
    );

    let mut last_error = None;
    let mut all_formats = Vec::new();
    // Capture subtitle tracks per audio type so the orchestrator gets
    // subtitles timed for the correct video version (SUB vs DUB).
    let mut sub_subtitle_tracks: Vec<megacloud::SubtitleTrack> = Vec::new();
    let mut dub_subtitle_tracks: Vec<megacloud::SubtitleTrack> = Vec::new();

    for server in &servers {
        debug!(
            server:% = server.server_name,
            audio:% = server.audio_type,
            data_id:% = server.data_id;
            "Trying server"
        );

        // Resolve embed URL
        let source = match api::fetch_source(&server.data_id, ctx).await {
            Ok(s) => s,
            Err(e) => {
                debug!(
                    server:% = server.server_name;
                    "Failed to fetch source: {e}"
                );
                last_error = Some(e);
                continue;
            }
        };

        // Extract video sources from Megacloud embed
        match megacloud::extract_sources(&source.embed_url, ctx).await {
            Ok(mega_sources) => {
                debug!(
                    server:% = server.server_name,
                    sources = mega_sources.sources.len(),
                    tracks = mega_sources.tracks.len();
                    "Megacloud extraction succeeded"
                );

                // Capture subtitle tracks per audio type (first of each)
                if !mega_sources.tracks.is_empty() {
                    match server.audio_type {
                        api::AudioType::Sub if sub_subtitle_tracks.is_empty() => {
                            sub_subtitle_tracks = mega_sources.tracks.clone();
                        }
                        api::AudioType::Dub if dub_subtitle_tracks.is_empty() => {
                            dub_subtitle_tracks = mega_sources.tracks.clone();
                        }
                        _ => {}
                    }
                }

                // Build formats from the resolved sources
                for (i, src) in mega_sources.sources.iter().enumerate() {
                    let format_id = format!(
                        "{}-{}-{}",
                        server.server_name.to_lowercase(),
                        server.audio_type,
                        i
                    );
                    let protocol = if src.source_type == "hls" {
                        DownloadProtocol::M3u8
                    } else {
                        DownloadProtocol::Https
                    };

                    let audio_label = server.audio_type.to_string();
                    let mut format = Format::new(&format_id, &src.url, "mp4", protocol);
                    format.format_note =
                        Some(format!("{} ({})", server.audio_type, server.server_name));
                    format.language = Some(audio_label);

                    // Set Referer to the embed URL so CDN M3U8 fetches aren't
                    // blocked by Cloudflare during HLS variant detection.
                    let mut headers = HashMap::new();
                    headers.insert("Referer".to_string(), source.embed_url.clone());
                    format.http_headers = Some(headers);

                    all_formats.push(format);
                }

                // Stop trying servers once we have both SUB and DUB with
                // CDN redundancy (≥2 formats per type for fallback).
                if !mega_sources.sources.is_empty() {
                    let sub_count = all_formats
                        .iter()
                        .filter(|f| f.format_note.as_ref().is_some_and(|n| n.contains("SUB")))
                        .count();
                    let dub_count = all_formats
                        .iter()
                        .filter(|f| f.format_note.as_ref().is_some_and(|n| n.contains("DUB")))
                        .count();

                    let remaining_provides_value = servers.iter().any(|s| {
                        s.data_id != server.data_id
                            && ((s.audio_type == api::AudioType::Sub && sub_count < 2)
                                || (s.audio_type == api::AudioType::Dub && dub_count < 2))
                    });

                    if !remaining_provides_value {
                        break;
                    }
                }
            }
            Err(e) => {
                debug!(
                    server:% = server.server_name;
                    "Megacloud extraction failed: {e}"
                );
                last_error = Some(e);
            }
        }
    }

    if all_formats.is_empty() {
        return Err(last_error.unwrap_or_else(|| RdlpError::Extraction {
            message: "No video sources found from any server".to_string(),
            url: None,
        }));
    }

    // Enrich HLS formats with resolution, codecs, duration, segments
    let (mut all_formats, hls_flags) = detect_format_sizes_lazy(all_formats, ctx, "9anime").await;

    // Restore audio type label in format_note (enrichment overwrites it)
    for f in &mut all_formats {
        if let Some(lang) = &f.language {
            match &f.format_note {
                Some(note) if !note.contains(lang) => {
                    f.format_note = Some(format!("{lang} {note}"));
                }
                None => {
                    f.format_note = Some(lang.clone());
                }
                _ => {}
            }
        }
    }

    // Prefer SUB subtitle tracks (correct timing for Japanese audio).
    // Fall back to DUB tracks if no SUB server provided subtitles.
    let subtitle_tracks = if !sub_subtitle_tracks.is_empty() {
        sub_subtitle_tracks
    } else {
        dub_subtitle_tracks
    };

    Ok((all_formats, hls_flags, subtitle_tracks))
}

/// Convert Megacloud subtitle tracks into an `InfoDict`-compatible subtitle map.
///
/// Groups tracks by label (language name), using the URL's file extension
/// (usually "vtt") as the subtitle format.
fn build_subtitle_map(
    tracks: &[megacloud::SubtitleTrack],
) -> HashMap<String, Vec<rdlp_types::Subtitle>> {
    let mut map: HashMap<String, Vec<rdlp_types::Subtitle>> = HashMap::new();

    for track in tracks {
        // Derive extension from URL (e.g., ".../en.vtt" → "vtt")
        let ext = track.url.rsplit('.').next().unwrap_or("vtt").to_string();

        let subtitle = rdlp_types::Subtitle {
            url: track.url.clone(),
            ext,
            name: Some(track.label.clone()),
        };

        map.entry(track.label.clone()).or_default().push(subtitle);
    }

    map
}

#[async_trait]
impl InfoExtractor for NineAnimeExtractor {
    fn name(&self) -> &str {
        "9anime"
    }

    fn valid_url(&self) -> &regex::Regex {
        &patterns::WATCH_URL_LOOSE_PATTERN
    }

    fn priority(&self) -> i32 {
        0
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        // Extract IDs from URL
        let anime_id = patterns::extract_anime_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!("Could not extract anime ID from URL: {url}"),
            url: Some(url.to_string()),
        })?;

        let episode_id =
            patterns::extract_episode_id(url).ok_or_else(|| RdlpError::Extraction {
                message: format!(
                    "Could not extract episode ID from URL: {url}. \
                 Use a URL with ?ep= parameter."
                ),
                url: Some(url.to_string()),
            })?;

        let slug = patterns::extract_slug(url).unwrap_or_default();

        info!(anime_id:%, episode_id:%, slug:%; "Extracting 9anime episode");

        // Fetch the watch page for metadata
        let webpage = BaseExtractor::fetch_webpage(url, ctx).await?;
        let anime_metadata = {
            let html = Html::parse_document(&webpage);
            metadata::extract_metadata(&html, &webpage)
        };

        // Fetch episode info (for title/number) in parallel with format
        // resolution. The episode info call is cheap (single AJAX fetch)
        // and doesn't overlap with server resolution.
        let (formats_result, episode_info) = tokio::join!(
            resolve_episode_formats(&episode_id, ctx),
            episodes::fetch_episode_info(&anime_id, &episode_id, ctx),
        );

        let (all_formats, hls_flags, subtitle_tracks) = formats_result?;
        let episode_info = episode_info.ok().flatten();

        // Build InfoDict
        let video_id = format!("{anime_id}-ep{episode_id}");
        let title = match &episode_info {
            Some(ep) => match &ep.title {
                Some(ep_title) => format!(
                    "{} - Episode {} - {ep_title}",
                    anime_metadata.title, ep.number
                ),
                None => format!("{} - Episode {}", anime_metadata.title, ep.number),
            },
            None => anime_metadata.title,
        };

        let mut info = InfoDict::new(video_id, title, "9anime", url);
        info.formats = all_formats;
        info.thumbnail = anime_metadata.thumbnail;
        info.description = anime_metadata.description;
        info.is_live = Some(hls_flags.is_live);

        // Populate subtitles from Megacloud tracks
        if !subtitle_tracks.is_empty() {
            info.subtitles = Some(build_subtitle_map(&subtitle_tracks));
        }

        info.propagate_duration();

        Ok(info)
    }

    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        if patterns::has_episode_param(url) {
            // Single episode — delegate to extract()
            Ok(vec![self.extract(url, ctx).await?])
        } else {
            // No ?ep= parameter — extract all episodes as a playlist
            playlist::extract_season(url, ctx).await
        }
    }

    /// Lightweight format resolution for lazily-extracted playlist entries.
    ///
    /// Skips the watch page fetch and episode info AJAX call (metadata is
    /// already available from playlist extraction). Only resolves Megacloud
    /// video sources and subtitles, avoiding Cloudflare rate-limiting.
    async fn extract_lazy(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let episode_id =
            patterns::extract_episode_id(url).ok_or_else(|| RdlpError::Extraction {
                message: format!(
                    "Could not extract episode ID from URL: {url}. \
                 Use a URL with ?ep= parameter."
                ),
                url: Some(url.to_string()),
            })?;

        debug!(episode_id:%; "Lazily resolving 9anime episode formats");

        let (formats, hls_flags, subtitle_tracks) =
            resolve_episode_formats(&episode_id, ctx).await?;

        let mut info = InfoDict::new("", "", "9anime", url);
        info.formats = formats;
        info.is_live = Some(hls_flags.is_live);

        if !subtitle_tracks.is_empty() {
            info.subtitles = Some(build_subtitle_map(&subtitle_tracks));
        }

        info.propagate_duration();

        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let extractor = NineAnimeExtractor::new();
        assert_eq!(extractor.name(), "9anime");
    }

    #[test]
    fn test_suitable() {
        let extractor = NineAnimeExtractor::new();
        assert!(extractor.suitable("https://9animetv.to/watch/sword-art-online-2274?ep=26565"));
        assert!(extractor.suitable("https://9animetv.to/watch/one-piece-100?ep=12345"));
        // Season URL (no ?ep=) should also be suitable
        assert!(extractor.suitable("https://9animetv.to/watch/sword-art-online-2274"));
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://9animetv.to/home"));
    }

    #[test]
    fn test_default() {
        let _extractor = NineAnimeExtractor;
    }

    #[test]
    fn test_build_subtitle_map() {
        let tracks = vec![
            megacloud::SubtitleTrack {
                url: "https://example.com/en.vtt".to_string(),
                label: "English".to_string(),
                is_default: true,
            },
            megacloud::SubtitleTrack {
                url: "https://example.com/ja.vtt".to_string(),
                label: "Japanese".to_string(),
                is_default: false,
            },
        ];

        let map = build_subtitle_map(&tracks);

        assert_eq!(map.len(), 2);
        assert!(map.contains_key("English"));
        assert!(map.contains_key("Japanese"));

        let en = &map["English"];
        assert_eq!(en.len(), 1);
        assert_eq!(en[0].ext, "vtt");
        assert_eq!(en[0].name.as_deref(), Some("English"));
    }

    #[test]
    fn test_build_subtitle_map_empty() {
        let map = build_subtitle_map(&[]);
        assert!(map.is_empty());
    }
}
