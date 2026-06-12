//! Thumbnail download support
//!
//! Downloads thumbnail images from extracted metadata URLs and saves them
//! alongside media files for embedding or standalone use.

use super::Orchestrator;
use log::{debug, info, warn};
use rdlp_redact::RedactedUrl;
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
        info: &rdlp_types::InfoDict,
        media_file: &Path,
    ) -> Option<PathBuf> {
        // Streaming size cap (20 MB) — adversarial CDN can't OOM the host
        // by sending a 1 GB image before the cap fires.
        const MAX_THUMBNAIL_BYTES: usize = 20 * 1024 * 1024;
        use futures_util::StreamExt;

        // Get best thumbnail URL
        let thumbnail_url = info.thumbnail.as_deref().or_else(|| {
            info.thumbnails
                .as_ref()
                .and_then(|thumbs| thumbs.first())
                .map(|t| t.url.as_str())
        })?;

        debug!(url = RedactedUrl::new(thumbnail_url); "Downloading thumbnail");

        // Determine extension from URL (default to jpg)
        let ext = thumbnail_url
            .rsplit('/')
            .next()
            .and_then(|filename| {
                // Strip query string
                let filename = filename.split('?').next().unwrap_or(filename);
                filename.rsplit('.').next()
            })
            .filter(|ext| {
                ["jpg", "jpeg", "png", "webp"]
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            })
            .map_or_else(|| "jpg".to_owned(), str::to_lowercase);

        // Build output path: {media_stem}.{ext}
        let thumbnail_path = super::container_resolver::sidecar_path(media_file, &ext);

        // SSRF gate: extractor-sourced URLs flow through here, and an
        // attacker-controlled extractor could return a private/loopback
        // URL in `info.thumbnail`.
        if let Err(e) = rdlp_security::validate_url_security(thumbnail_url) {
            warn!(url = RedactedUrl::new(thumbnail_url); "Thumbnail URL rejected by security gate: {e}");
            return None;
        }

        // Build Referer from the thumbnail URL origin (CDNs often require this)
        let referer = url::Url::parse(thumbnail_url)
            .ok()
            .map(|u| format!("{}://{}/", u.scheme(), u.host_str().unwrap_or("")));

        // Download using the shared HTTP client
        let mut request = self.extraction_context.http_client.get(thumbnail_url);
        if let Some(ref referer) = referer {
            request = request.header("Referer", referer);
        }

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(url = RedactedUrl::new(thumbnail_url); "Failed to download thumbnail: {e}");
                return None;
            }
        };

        if !response.status().is_success() {
            warn!(
                url = RedactedUrl::new(thumbnail_url),
                status = response.status().as_u16();
                "Thumbnail download returned non-success status"
            );
            return None;
        }

        let mut stream = response.bytes_stream();
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    if bytes.len().saturating_add(chunk.len()) > MAX_THUMBNAIL_BYTES {
                        warn!(
                            url = RedactedUrl::new(thumbnail_url);
                            "Thumbnail exceeds {MAX_THUMBNAIL_BYTES}-byte cap; aborting"
                        );
                        return None;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Some(Err(e)) => {
                    warn!("Failed to read thumbnail response: {e}");
                    return None;
                }
                None => break,
            }
        }

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
