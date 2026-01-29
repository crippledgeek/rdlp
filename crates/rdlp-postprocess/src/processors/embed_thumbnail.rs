//! Thumbnail embedding post-processor.
//!
//! Embeds thumbnail images into media files using `ffmpeg-the-third` library
//! bindings (no CLI process spawning). Supports different embedding methods
//! based on the container format:
//! - MP4/M4A/MOV: Cover art with `ATTACHED_PIC` disposition
//! - MKV/MKA: Attachment stream with mimetype metadata
//! - MP3: Video stream with ID3v2 metadata
//! - FLAC/OGG/Opus: Video stream with `ATTACHED_PIC` disposition

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
        _config: &PostProcessConfig,
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
                info!(file:? = media_file.display(); "Thumbnail embedded");

                // Return thumbnail as temp file for cleanup
                Ok(PostProcessResult {
                    info: info.clone(),
                    files,
                    temp_files: vec![thumbnail_file],
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
