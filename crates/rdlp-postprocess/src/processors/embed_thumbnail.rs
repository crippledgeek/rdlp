//! Thumbnail embedding post-processor.
//!
//! Embeds thumbnail images into media files using `ffmpeg-the-third` library
//! bindings (no CLI process spawning). Supports different embedding methods
//! based on the container format:
//! - MP4/M4A/MOV: Cover art with `ATTACHED_PIC` disposition + iTunes `covr` atom
//! - MKV/MKA: Attachment stream with mimetype metadata
//! - MP3: Video stream with ID3v2 metadata
//! - FLAC/OGG/Opus: Video stream with `ATTACHED_PIC` disposition
//!
//! For MP4 containers, a second pass writes the thumbnail into the iTunes `covr`
//! metadata atom using `mp4ameta`. This is needed because Windows Explorer reads
//! cover art from the `covr` atom, not from `attached_pic` streams.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

/// Supported thumbnail formats.
const THUMBNAIL_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Containers that support thumbnail embedding.
const SUPPORTED_CONTAINERS: &[&str] = &[
    "mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac", "ogg", "opus",
];

ffmpeg_processor!(
    EmbedThumbnail,
    "EmbedThumbnail",
    20,
    "Post-processor that embeds thumbnails into media files.\n\n\
     # Priority\n\
     This processor has priority 20 (runs after most other processing).\n\n\
     # When it runs\n\
     - When `embed_thumbnail` is true in config\n\
     - When a thumbnail file exists alongside the media file"
);

impl EmbedThumbnail {
    /// Find a thumbnail file for the given media file.
    fn find_thumbnail(media_file: &Path) -> Option<PathBuf> {
        let stem = media_file.file_stem()?.to_str()?;
        let parent = media_file.parent()?;

        for ext in THUMBNAIL_EXTENSIONS {
            let thumbnail_path = parent.join(format!("{stem}.{ext}"));
            if thumbnail_path.exists() {
                return Some(thumbnail_path);
            }
        }

        None
    }

    /// Check if the container supports thumbnail embedding.
    fn supports_thumbnail(extension: &str) -> bool {
        SUPPORTED_CONTAINERS
            .iter()
            .any(|c| c.eq_ignore_ascii_case(extension))
    }

    /// Check if the container is an MP4-family format (supports covr atom).
    fn is_mp4_family(extension: &str) -> bool {
        matches!(
            extension.to_lowercase().as_str(),
            "mp4" | "m4a" | "m4v" | "mov"
        )
    }

    /// Write the iTunes `covr` metadata atom for Windows Explorer thumbnail visibility.
    ///
    /// This is a second pass after FFmpeg embedding. Non-fatal: logs a warning on failure.
    async fn write_covr_atom(media_file: &Path, thumbnail_file: &Path) {
        let media = media_file.to_path_buf();
        let thumb = thumbnail_file.to_path_buf();

        let result = tokio::task::spawn_blocking(move || {
            let cover_bytes = std::fs::read(&thumb)?;

            let img = match thumb
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase()
                .as_str()
            {
                "png" => mp4ameta::Img::png(cover_bytes),
                _ => mp4ameta::Img::jpeg(cover_bytes),
            };

            let mut tag = mp4ameta::Tag::read_from_path(&media)
                .unwrap_or_else(|_| mp4ameta::Tag::default());
            tag.set_artwork(img);
            tag.write_to_path(&media)?;

            Ok::<(), anyhow::Error>(())
        })
        .await;

        match result {
            Ok(Ok(())) => info!("MP4 covr atom written for Windows Explorer"),
            Ok(Err(e)) => warn!("Failed to write MP4 covr atom: {e}"),
            Err(e) => warn!("covr atom task panicked: {e}"),
        }
    }
}

#[async_trait]
impl PostProcessor for EmbedThumbnail {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.embed_thumbnail
    }

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
    ) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let media_file = &files[0];
        let extension = media_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Check if container supports thumbnails
        if !Self::supports_thumbnail(extension) {
            debug!(extension; "Container does not support thumbnail embedding");
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        // Find thumbnail file
        let thumbnail_file = match Self::find_thumbnail(media_file) {
            Some(path) => path,
            None => {
                debug!(file:? = media_file.display(); "No thumbnail file found");
                return Ok(PostProcessResult::new(info.clone(), files));
            }
        };

        info!(
            thumbnail:? = thumbnail_file.display(),
            media:? = media_file.display();
            "Embedding thumbnail"
        );

        // Create temp output file
        let temp_output = media_file.with_extension(format!("thumb.{extension}"));

        // Embed via library bindings
        match self
            .ffmpeg
            .embed_thumbnail(media_file, &thumbnail_file, &temp_output, extension)
            .await
        {
            Ok(()) => {
                // Replace original with temp
                tokio::fs::rename(&temp_output, media_file).await?;
                info!(file:? = media_file.display(); "Thumbnail embedded via FFmpeg");

                // For MP4-family: write covr atom so Windows Explorer shows the thumbnail
                debug!(extension, is_mp4 = Self::is_mp4_family(extension); "Checking covr atom eligibility");
                if Self::is_mp4_family(extension) {
                    Self::write_covr_atom(media_file, &thumbnail_file).await;
                }

                // Clean up thumbnail unless --write-thumbnail was requested
                let temp_files = if config.write_thumbnail {
                    Vec::new()
                } else {
                    vec![thumbnail_file]
                };
                Ok(PostProcessResult {
                    info: info.clone(),
                    files,
                    temp_files,
                })
            }
            Err(e) => {
                warn!("Failed to embed thumbnail: {e}");
                // Clean up temp file if it exists
                let _ = tokio::fs::remove_file(&temp_output).await;

                // Non-fatal - return original files
                Ok(PostProcessResult::new(info.clone(), files))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_thumbnail() {
        assert!(EmbedThumbnail::supports_thumbnail("mp4"));
        assert!(EmbedThumbnail::supports_thumbnail("MP4"));
        assert!(EmbedThumbnail::supports_thumbnail("mkv"));
        assert!(EmbedThumbnail::supports_thumbnail("mp3"));
        assert!(EmbedThumbnail::supports_thumbnail("flac"));
        assert!(EmbedThumbnail::supports_thumbnail("ogg"));
        assert!(EmbedThumbnail::supports_thumbnail("opus"));
        assert!(!EmbedThumbnail::supports_thumbnail("txt"));
        assert!(!EmbedThumbnail::supports_thumbnail("avi"));
    }
}
