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
pub mod megacloud;
pub mod metadata;
pub mod patterns;
pub mod playlist;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{
    DownloadProtocol, ExtractionContext, Format, InfoDict, InfoExtractor, RdlpError, Result,
};
use scraper::Html;

use crate::base::common::BaseExtractor;
use crate::hls::{HlsStreamFlags, detect_format_sizes};

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
) -> Result<(Vec<Format>, HlsStreamFlags)> {
    let mut servers = api::fetch_servers(episode_id, ctx).await?;
    if servers.is_empty() {
        return Err(RdlpError::Extraction(
            "No streaming servers found for this episode".to_string(),
        ));
    }

    api::sort_by_preference(&mut servers);

    info!(
        servers = servers.len();
        "Found streaming servers, attempting extraction"
    );

    let mut last_error = None;
    let mut all_formats = Vec::new();

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
                warn!(
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
                info!(
                    server:% = server.server_name,
                    sources = mega_sources.sources.len(),
                    tracks = mega_sources.tracks.len();
                    "Megacloud extraction succeeded"
                );

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

                    let ext = "mp4";
                    let audio_label = server.audio_type.to_string();

                    let mut format = Format::new(&format_id, &src.url, ext, protocol);
                    format.format_note =
                        Some(format!("{} ({})", server.audio_type, server.server_name));
                    format.language = Some(audio_label);

                    all_formats.push(format);
                }

                // Stop trying servers once we have both SUB and DUB (or no
                // remaining servers offer the missing type).
                if !mega_sources.sources.is_empty() {
                    let has_sub = all_formats
                        .iter()
                        .any(|f| f.format_note.as_ref().is_some_and(|n| n.contains("SUB")));
                    let has_dub = all_formats
                        .iter()
                        .any(|f| f.format_note.as_ref().is_some_and(|n| n.contains("DUB")));
                    let remaining_has_other_type = servers.iter().any(|s| {
                        s.data_id != server.data_id
                            && ((s.audio_type == api::AudioType::Sub && !has_sub)
                                || (s.audio_type == api::AudioType::Dub && !has_dub))
                    });

                    if !remaining_has_other_type {
                        break;
                    }
                }
            }
            Err(e) => {
                warn!(
                    server:% = server.server_name;
                    "Megacloud extraction failed: {e}"
                );
                last_error = Some(e);
            }
        }
    }

    if all_formats.is_empty() {
        return Err(last_error.unwrap_or_else(|| {
            RdlpError::Extraction("No video sources found from any server".to_string())
        }));
    }

    // Enrich HLS formats with resolution, codecs, duration, segments
    let (mut all_formats, hls_flags) = detect_format_sizes(all_formats, ctx, "9anime").await;

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

    Ok((all_formats, hls_flags))
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
        let anime_id = patterns::extract_anime_id(url).ok_or_else(|| {
            RdlpError::Extraction(format!("Could not extract anime ID from URL: {url}"))
        })?;

        let episode_id = patterns::extract_episode_id(url).ok_or_else(|| {
            RdlpError::Extraction(format!(
                "Could not extract episode ID from URL: {url}. \
                 Use a URL with ?ep= parameter."
            ))
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
            api::fetch_episode_info(&anime_id, &episode_id, ctx),
        );

        let (all_formats, hls_flags) = formats_result?;
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
        let _extractor = NineAnimeExtractor::default();
    }
}
