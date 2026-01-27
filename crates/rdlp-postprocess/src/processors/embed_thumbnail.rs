//! Thumbnail embedding post-processor.
//!
//! Embeds thumbnail images into media files. Supports different embedding
//! methods based on the container format:
//! - MP4/M4A: Uses FFmpeg with cover art disposition
//! - MKV/MKA: Uses FFmpeg attachment
//! - MP3: Uses FFmpeg with ID3v2 tags

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::ffmpeg::FFmpegRunner;

/// Supported thumbnail formats.
const THUMBNAIL_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Containers that support thumbnail embedding.
const SUPPORTED_CONTAINERS: &[&str] = &[
    "mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac", "ogg", "opus",
];

/// Post-processor that embeds thumbnails into media files.
///
/// # Priority
/// This processor has priority 20 (runs after most other processing).
///
/// # When it runs
/// - When `embed_thumbnail` is true in config
/// - When a thumbnail file exists alongside the media file
pub struct EmbedThumbnail {
    ffmpeg: Arc<FFmpegRunner>,
}

impl EmbedThumbnail {
    /// Create a new thumbnail embedding processor.
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Find a thumbnail file for the given media file.
    fn find_thumbnail(&self, media_file: &Path) -> Option<PathBuf> {
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

    /// Build FFmpeg arguments for embedding thumbnail.
    fn build_embed_args(
        &self,
        media_file: &Path,
        thumbnail_file: &Path,
        output_file: &Path,
        container: &str,
    ) -> Vec<String> {
        // Input files
        let mut args = vec![
            "-i".to_string(),
            media_file.to_string_lossy().to_string(),
            "-i".to_string(),
            thumbnail_file.to_string_lossy().to_string(),
        ];

        match container.to_lowercase().as_str() {
            // MP4/M4A/MOV - use cover art
            "mp4" | "m4a" | "m4v" | "mov" => {
                args.push("-map".to_string());
                args.push("0".to_string());
                args.push("-map".to_string());
                args.push("1".to_string());
                args.push("-c".to_string());
                args.push("copy".to_string());
                args.push("-c:v:1".to_string());
                args.push("mjpeg".to_string()); // Convert to JPEG for compatibility
                args.push("-disposition:v:1".to_string());
                args.push("attached_pic".to_string());
            }
            // MKV/MKA - use attachment
            "mkv" | "mka" => {
                args.push("-map".to_string());
                args.push("0".to_string());
                args.push("-c".to_string());
                args.push("copy".to_string());
                args.push("-attach".to_string());
                args.push(thumbnail_file.to_string_lossy().to_string());
                args.push("-metadata:s:t".to_string());
                args.push("mimetype=image/jpeg".to_string());
                args.push("-metadata:s:t".to_string());
                args.push("filename=cover.jpg".to_string());
            }
            // MP3 - use ID3v2 APIC frame
            "mp3" => {
                args.push("-i".to_string());
                args.push(thumbnail_file.to_string_lossy().to_string());
                // Remove the duplicate -i we added above
                args.remove(args.len() - 1);
                args.remove(args.len() - 1);
                args.push("-i".to_string());
                args.push(thumbnail_file.to_string_lossy().to_string());

                args.push("-map".to_string());
                args.push("0:a".to_string());
                args.push("-map".to_string());
                args.push("1:v".to_string());
                args.push("-c:a".to_string());
                args.push("copy".to_string());
                args.push("-c:v".to_string());
                args.push("mjpeg".to_string());
                args.push("-id3v2_version".to_string());
                args.push("3".to_string());
                args.push("-metadata:s:v".to_string());
                args.push("title=Album cover".to_string());
                args.push("-metadata:s:v".to_string());
                args.push("comment=Cover (front)".to_string());
            }
            // FLAC/OGG/Opus - use metadata picture
            "flac" | "ogg" | "opus" => {
                args.push("-map".to_string());
                args.push("0".to_string());
                args.push("-map".to_string());
                args.push("1".to_string());
                args.push("-c".to_string());
                args.push("copy".to_string());
                args.push("-disposition:v".to_string());
                args.push("attached_pic".to_string());
            }
            _ => {
                // Fallback - try attachment method
                args.push("-map".to_string());
                args.push("0".to_string());
                args.push("-c".to_string());
                args.push("copy".to_string());
                args.push("-attach".to_string());
                args.push(thumbnail_file.to_string_lossy().to_string());
            }
        }

        args.push(output_file.to_string_lossy().to_string());

        args
    }
}

#[async_trait]
impl PostProcessor for EmbedThumbnail {
    fn name(&self) -> &str {
        "EmbedThumbnail"
    }

    fn priority(&self) -> i32 {
        20 // Near the end
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
        let thumbnail_file = match self.find_thumbnail(media_file) {
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

        // Build and run FFmpeg command
        let args = self.build_embed_args(media_file, &thumbnail_file, &temp_output, extension);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        match self.ffmpeg.run(&args_refs).await {
            Ok(_) => {
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
        assert!(!EmbedThumbnail::supports_thumbnail("txt"));
        assert!(!EmbedThumbnail::supports_thumbnail("avi"));
    }

    #[test]
    fn test_build_embed_args_mp4() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let processor = EmbedThumbnail::new(Arc::new(ffmpeg));

            let args = processor.build_embed_args(
                Path::new("video.mp4"),
                Path::new("video.jpg"),
                Path::new("output.mp4"),
                "mp4",
            );

            assert!(args.contains(&"-disposition:v:1".to_string()));
            assert!(args.contains(&"attached_pic".to_string()));
        }
    }

    #[test]
    fn test_build_embed_args_mkv() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let processor = EmbedThumbnail::new(Arc::new(ffmpeg));

            let args = processor.build_embed_args(
                Path::new("video.mkv"),
                Path::new("video.jpg"),
                Path::new("output.mkv"),
                "mkv",
            );

            assert!(args.contains(&"-attach".to_string()));
            assert!(args.iter().any(|a| a.contains("mimetype=")));
        }
    }
}
