//! Thumbnail download support
//!
//! Downloads thumbnail images from extracted metadata URLs and saves them
//! alongside media files for embedding or standalone use.

use super::Orchestrator;
use log::{debug, info, warn};
use std::path::{Path, PathBuf};

impl Orchestrator {
    /// Download thumbnail image from URL and save alongside media file.
    ///
    /// Saves as `{media_stem}.{ext}` in the same directory as the media file.
    /// The extension is inferred from the thumbnail URL (defaults to `jpg`).
    ///
    /// This is non-fatal: logs a warning and returns `None` on any error.
    /// The `EmbedThumbnail` post-processor will then skip embedding gracefully.
    ///
    /// # Returns
    ///
    /// - `Some(path)` — thumbnail downloaded successfully
    /// - `None` — no thumbnail URL available or download failed
    pub(super) async fn download_thumbnail(
        &self,
        info: &rdlp_core::InfoDict,
        media_file: &Path,
    ) -> Option<PathBuf> {
        // Get best thumbnail URL
        let thumbnail_url = info.thumbnail.as_deref().or_else(|| {
            info.thumbnails
                .as_ref()
                .and_then(|thumbs| thumbs.first())
                .map(|t| t.url.as_str())
        })?;

        debug!(url = thumbnail_url; "Downloading thumbnail");

        // Determine extension from URL (default to jpg)
        let ext = thumbnail_url
            .rsplit('/')
            .next()
            .and_then(|filename| {
                // Strip query string
                let filename = filename.split('?').next().unwrap_or(filename);
                filename.rsplit('.').next()
            })
            .and_then(|ext| {
                let ext = ext.to_lowercase();
                match ext.as_str() {
                    "jpg" | "jpeg" | "png" | "webp" => Some(ext),
                    _ => None,
                }
            })
            .unwrap_or_else(|| "jpg".to_string());

        // Build output path: {media_stem}.{ext}
        let stem = media_file.file_stem()?.to_str()?;
        let parent = media_file.parent()?;
        let thumbnail_path = parent.join(format!("{stem}.{ext}"));

        // Download using the shared HTTP client
        let response = match self
            .extraction_context
            .http_client
            .get(thumbnail_url)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                warn!("Failed to download thumbnail: {e}");
                return None;
            }
        };

        if !response.status().is_success() {
            warn!(
                status = response.status().as_u16();
                "Thumbnail download returned non-success status"
            );
            return None;
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to read thumbnail response: {e}");
                return None;
            }
        };

        if bytes.is_empty() {
            warn!("Thumbnail response was empty");
            return None;
        }

        // Write to disk
        if let Err(e) = tokio::fs::write(&thumbnail_path, &bytes).await {
            warn!("Failed to write thumbnail file: {e}");
            return None;
        }

        info!(
            path = thumbnail_path.display().to_string().as_str(),
            size = bytes.len();
            "Thumbnail downloaded"
        );

        Some(thumbnail_path)
    }
}
