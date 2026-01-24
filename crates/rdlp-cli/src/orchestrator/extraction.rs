//! Video information extraction

use super::{errors::*, Orchestrator};
use log::info;

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
    pub(super) async fn extract_video_info(&self, url: &str) -> Result<rdlp_core::InfoDict> {
        info!("Finding extractor for URL...");

        let extractor = self
            .extractor_registry
            .find_extractor(url)
            .ok_or_else(|| OrchestratorError::NoExtractor {
                url: url.to_string(),
            })?;

        info!("Using {} extractor", extractor.name());
        info!("Extracting video information...");

        let info = extractor
            .extract(url, &self.extraction_context)
            .await
            .map_err(OrchestratorError::ExtractionFailed)?;

        info!("Title: {}", info.title);
        info!("Found {} formats", info.formats.len());

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
    pub(super) async fn extract_playlist_info(&self, url: &str) -> Result<Vec<rdlp_core::InfoDict>> {
        info!("Finding extractor for URL...");

        let extractor = self
            .extractor_registry
            .find_extractor(url)
            .ok_or_else(|| OrchestratorError::NoExtractor {
                url: url.to_string(),
            })?;

        info!("Using {} extractor", extractor.name());
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
