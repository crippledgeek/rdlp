//! Thumbnail download support
//!
//! Downloads thumbnail images from extracted metadata URLs and saves them
//! alongside media files for embedding or standalone use.

use super::Orchestrator;
use log::{debug, info, warn};
use rdlp_redact::RedactedUrl;
use std::path::{Path, PathBuf};

/// Derive a FALLBACK sidecar extension for a downloaded thumbnail from its URL.
///
/// Since #525 the on-disk name comes from the fetched bytes via
/// [`rdlp_types::sniff_thumbnail_format`]; this supplies the name only when the
/// content is unrecognized, because a URL's extension is a claim rather than a
/// fact — the CDN that prompted #525 serves WebP from a `.jpg` path.
///
/// Preserves the URL's trailing extension only when it is a recognized raster
/// thumbnail format (see [`rdlp_types::THUMBNAIL_EXTENSIONS`]); any query string
/// is stripped first. An unrecognized or absent extension defaults to `jpg`,
/// matching yt-dlp's behavior for extensionless thumbnail URLs.
///
/// This whitelist MUST stay aligned with the post-process discovery gate (both
/// draw from `THUMBNAIL_EXTENSIONS`): a format saved here that discovery does not
/// look for — or vice versa — silently drops the thumbnail.
fn thumbnail_extension_from_url(thumbnail_url: &str) -> String {
    thumbnail_url
        .rsplit('/')
        .next()
        .and_then(|filename| {
            // Strip query string, then take the trailing extension token.
            let filename = filename.split('?').next().unwrap_or(filename);
            filename.rsplit('.').next()
        })
        .filter(|ext| {
            rdlp_types::THUMBNAIL_EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
        .map_or_else(|| "jpg".to_owned(), str::to_lowercase)
}

impl Orchestrator {
    /// Download thumbnail image from URL and save alongside media file.
    ///
    /// Saves as `{media_stem}.{ext}` in the same directory as the media file.
    /// The extension is inferred from the fetched BYTES (#525) — a CDN may
    /// serve one format from another format's URL path — falling back to the
    /// URL's extension, and then to `jpg`, only for unrecognized content.
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

        // Name the sidecar for what the bytes ACTUALLY are, not what the URL
        // claims (#525). A CDN serving WebP from a `.jpg` path would otherwise
        // put WebP bytes in a `.jpg` file, and every later stage that reads the
        // extension as a proxy for the codec then makes the wrong call — the
        // embed gate skips normalization and the raw WebP reaches a muxer that
        // has no tag for it. Falls back to the URL's extension only when the
        // content is unrecognized, preserving the previous behavior there.
        let url_ext = thumbnail_extension_from_url(thumbnail_url);
        let ext = rdlp_types::sniff_thumbnail_format(&bytes).map_or_else(
            || {
                debug!(
                    url = RedactedUrl::new(thumbnail_url),
                    ext = url_ext.as_str();
                    "Thumbnail content unrecognized; naming from URL extension"
                );
                url_ext.clone()
            },
            |format| format.extension().to_owned(),
        );

        if ext != url_ext {
            // Worth surfacing: this is the mislabel that produced #525, and it
            // is otherwise invisible until an embed fails much later.
            debug!(
                url = RedactedUrl::new(thumbnail_url),
                claimed = url_ext.as_str(),
                actual = ext.as_str();
                "Thumbnail URL extension disagrees with its content; using content"
            );
        }

        // Build output path: {media_stem}.{ext}
        let thumbnail_path = super::container_resolver::sidecar_path(media_file, &ext);

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

#[cfg(test)]
mod tests {
    use super::thumbnail_extension_from_url;

    #[test]
    fn preserves_original_baseline_extensions() {
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.jpg"),
            "jpg"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.jpeg"),
            "jpeg"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.png"),
            "png"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.webp"),
            "webp"
        );
    }

    /// #521: the widened raster formats must be preserved on disk, not defaulted
    /// to `jpg`. Fails against the pre-#521 whitelist (`jpg/jpeg/png/webp`),
    /// which rejected these and saved e.g. GIF bytes under a `.jpg` name — the
    /// codec/extension mislabel that then breaks embedding.
    #[test]
    fn preserves_widened_raster_extensions() {
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.gif"),
            "gif"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.bmp"),
            "bmp"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.tiff"),
            "tiff"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.tif"),
            "tif"
        );
    }

    #[test]
    fn lowercases_recognized_uppercase_extension() {
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/COVER.GIF"),
            "gif"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/COVER.WEBP"),
            "webp"
        );
    }

    #[test]
    fn strips_query_string_before_extension() {
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.png?w=640&sig=abc"),
            "png"
        );
    }

    /// Negative: an extension we cannot decode (avif) or no extension at all
    /// defaults to `jpg` — never saved under its own unrecognized extension.
    #[test]
    fn defaults_unrecognized_or_missing_extension_to_jpg() {
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/cover.avif"),
            "jpg"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/thumb?w=640"),
            "jpg"
        );
        assert_eq!(
            thumbnail_extension_from_url("https://cdn/x/coverfile"),
            "jpg"
        );
    }
}
