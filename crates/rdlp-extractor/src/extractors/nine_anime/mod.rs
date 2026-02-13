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
//! - `https://9animetv.to/watch/{slug}-{id}?ep={ep-id}`

pub mod api;
pub mod megacloud;
pub mod metadata;
pub mod patterns;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{
    DownloadProtocol, ExtractionContext, Format, InfoDict, InfoExtractor, RdlpError, Result,
};
use scraper::Html;

use crate::base::common::BaseExtractor;

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

        // Fetch available servers
        let mut servers = api::fetch_servers(&episode_id, ctx).await?;
        if servers.is_empty() {
            return Err(RdlpError::Extraction(
                "No streaming servers found for this episode".to_string(),
            ));
        }

        // Sort by preference (Vidcloud → Vidstreaming → DouVideo)
        api::sort_by_preference(&mut servers);

        info!(
            servers = servers.len();
            "Found streaming servers, attempting extraction"
        );

        // Try each server until one succeeds
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

                        let mut format = Format::new(&format_id, &src.url, ext, protocol);
                        format.format_note =
                            Some(format!("{} ({})", server.audio_type, server.server_name));

                        all_formats.push(format);
                    }

                    // If we got at least one format, we can stop trying servers
                    // of the same audio type. But continue to get both SUB and DUB.
                    if !mega_sources.sources.is_empty() {
                        // Check if we have both SUB and DUB formats
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

        // Build InfoDict
        let video_id = format!("{anime_id}-ep{episode_id}");
        let title = if let Some(ref ep_num) = anime_metadata.episode_number {
            format!("{} - Episode {ep_num}", anime_metadata.title)
        } else {
            anime_metadata.title
        };

        let mut info = InfoDict::new(video_id, title, "9anime", url);
        info.formats = all_formats;
        info.thumbnail = anime_metadata.thumbnail;
        info.description = anime_metadata.description;

        Ok(info)
    }

    async fn extract_playlist(&self, url: &str, ctx: &ExtractionContext) -> Result<Vec<InfoDict>> {
        // Single episode extraction — no playlist support yet
        Ok(vec![self.extract(url, ctx).await?])
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
        assert!(!extractor.suitable("https://youtube.com/watch?v=test"));
        assert!(!extractor.suitable("https://9animetv.to/home"));
    }

    #[test]
    fn test_default() {
        let _extractor = NineAnimeExtractor::default();
    }
}
