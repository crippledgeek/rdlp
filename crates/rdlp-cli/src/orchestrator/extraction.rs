//! Video information extraction

use super::{Orchestrator, errors::*};
use log::info;
use tracing::instrument;

impl Orchestrator {
    /// Extract video information from URL
    ///
    /// Finds the appropriate extractor for the given URL and extracts metadata
    /// including title, formats, and other video information.
    ///
    /// # Errors
    /// Returns an error if:
    /// - No extractor is found for the URL
    /// - Extraction fails
    #[instrument(skip(self), fields(url = %url))]
    pub(super) async fn extract_video_info(&self, url: &str) -> Result<rdlp_core::InfoDict> {
        info!("Finding extractor for URL...");

        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: url.to_string(),
            }
        })?;

        info!("Using extractor: {}", extractor.name());
        info!("Extracting video information...");

        let mut info = extractor
            .extract(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        // Log extracted metadata
        info!("Title: {}", info.title);

        if let Some(ref uploader) = info.uploader {
            info!("Uploader: {uploader}");
        }
        if let Some(ref channel) = info.channel {
            info!("Channel: {channel}");
        }
        if let Some(duration) = info.duration {
            let mins = (duration / 60.0) as u32;
            let secs = (duration % 60.0) as u32;
            info!("Duration: {mins}:{secs:02}");
        }
        if let Some(views) = info.view_count {
            info!("Views: {views}");
        }
        if let Some(rating) = info.average_rating {
            info!("Rating: {rating:.0}%");
        }
        if let Some(ref tags) = info.tags {
            if !tags.is_empty() {
                info!("Tags: {}", tags.len());
            }
        }

        info!("Found {} formats", info.formats.len());

        // Auto-set Referer header on all formats that don't already have one.
        // Many CDNs (PornHub, XHamster, etc.) require a Referer to serve content.
        if !info.webpage_url.is_empty() {
            let referer = info.webpage_url.clone();
            for fmt in &mut info.formats {
                let headers = fmt
                    .http_headers
                    .get_or_insert_with(std::collections::HashMap::new);
                headers
                    .entry("Referer".to_string())
                    .or_insert(referer.clone());
            }
        }

        Ok(info)
    }

    /// Extract playlist information from URL
    ///
    /// Finds the appropriate extractor and attempts playlist extraction.
    /// Returns a vector of InfoDict - one for each video in the playlist.
    ///
    /// # Returns
    /// - Single video: Vec with one InfoDict
    /// - Playlist: Vec with multiple InfoDict entries
    ///
    /// # Errors
    /// Returns an error if:
    /// - No extractor is found for the URL
    /// - Extraction fails
    #[instrument(skip(self), fields(url = %url))]
    pub(super) async fn extract_playlist_info(
        &self,
        url: &str,
    ) -> Result<Vec<rdlp_core::InfoDict>> {
        info!("Finding extractor for URL...");

        let extractor = self.extractor_registry.find_extractor(url).ok_or_else(|| {
            OrchestratorError::NoExtractor {
                url: url.to_string(),
            }
        })?;

        info!("Using extractor: {}", extractor.name());
        info!("Extracting playlist information...");

        let infos = extractor
            .extract_playlist(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        if infos.len() == 1 {
            info!("Single video: {}", infos[0].title);
        } else {
            let playlist_title = infos[0]
                .playlist_title
                .as_deref()
                .unwrap_or("Unnamed Playlist");
            info!("Playlist: {playlist_title}");
            info!("Found {} videos", infos.len());
        }

        Ok(infos)
    }
}
