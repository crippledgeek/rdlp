//! Video information extraction

use super::{errors::*, Orchestrator};

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
        println!("🔍 Finding extractor for URL...");

        let extractor = self
            .extractor_registry
            .find_extractor(url)
            .ok_or_else(|| OrchestratorError::NoExtractor {
                url: url.to_string(),
            })?;

        println!("✓ Using {} extractor", extractor.name());
        println!("📊 Extracting video information...");

        let info = extractor
            .extract(url, &self.extraction_context)
            .await
            .map_err(|e| OrchestratorError::ExtractionFailed(e.into()))?;

        println!("✓ Title: {}", info.title);
        println!("✓ Found {} formats", info.formats.len());

        Ok(info)
    }
}
