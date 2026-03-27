//! ThumbnailStage — embeds thumbnail images into media files.
//!
//! This stage runs at index 7 (last) when `config.embed_thumbnail` is true.
//! Uses `msg.original_stem` for thumbnail discovery (not the UUID-renamed stem).
//! Non-fatal: failure logs a warning and passes through.
//!
//! For MP4-family containers: two-pass embedding —
//! 1. FFmpeg `attached_pic` stream (media player support)
//! 2. `mp4ameta` iTunes `covr` atom (Windows Explorer visibility)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info, warn};

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Supported thumbnail image formats.
const THUMBNAIL_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

/// Containers that support thumbnail embedding via FFmpeg.
const SUPPORTED_CONTAINERS: &[&str] = &[
    "mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac", "ogg", "opus",
];

/// Embeds thumbnail into the primary current file.
///
/// `should_run` triggers when `config.embed_thumbnail` is true.
/// Non-fatal: failures push a warning and pass through unchanged.
pub struct ThumbnailStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl ThumbnailStage {
    /// Create a new `ThumbnailStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Find a thumbnail file using `original_stem` for discovery.
    ///
    /// Searches `{parent}/{original_stem}.{jpg|jpeg|png|webp}`.
    /// Also tries the current file's stem as a fallback.
    fn find_thumbnail(media_file: &Path, original_stem: &str) -> Option<PathBuf> {
        let parent = media_file.parent()?;

        // Try original_stem first (most accurate after UUID renames).
        for ext in THUMBNAIL_EXTENSIONS {
            let path = parent.join(format!("{original_stem}.{ext}"));
            if path.exists() {
                return Some(path);
            }
        }

        // Fallback: try current file stem.
        let current_stem = media_file.file_stem()?.to_str()?;
        if current_stem != original_stem {
            for ext in THUMBNAIL_EXTENSIONS {
                let path = parent.join(format!("{current_stem}.{ext}"));
                if path.exists() {
                    return Some(path);
                }
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

    /// Check if the container is an MP4-family format (supports `covr` atom).
    fn is_mp4_family(extension: &str) -> bool {
        ["mp4", "m4a", "m4v", "mov"]
            .iter()
            .any(|c| c.eq_ignore_ascii_case(extension))
    }

    /// Write the iTunes `covr` metadata atom for Windows Explorer thumbnail visibility.
    ///
    /// Non-fatal: logs a warning on failure.
    async fn write_covr_atom(media_file: &Path, thumbnail_file: &Path) {
        let media = media_file.to_path_buf();
        let thumb = thumbnail_file.to_path_buf();

        let result = tokio::task::spawn_blocking(move || {
            let cover_bytes =
                std::fs::read(&thumb).context("thumbnail stage: failed to read thumbnail file")?;
            let ext = thumb.extension().and_then(|e| e.to_str()).unwrap_or("");
            let img = if ext.eq_ignore_ascii_case("png") {
                mp4ameta::Img::png(cover_bytes)
            } else {
                mp4ameta::Img::jpeg(cover_bytes)
            };
            let mut tag =
                mp4ameta::Tag::read_from_path(&media).unwrap_or_else(|_| mp4ameta::Tag::default());
            tag.set_artwork(img);
            tag.write_to_path(&media)
                .context("thumbnail stage: failed to write covr atom to media file")?;
            Ok::<(), anyhow::Error>(())
        })
        .await;

        match result {
            Ok(Ok(())) => debug!("ThumbnailStage: MP4 covr atom written"),
            Ok(Err(e)) => warn!("ThumbnailStage: failed to write covr atom: {e}"),
            Err(e) => warn!("ThumbnailStage: covr atom task panicked: {e}"),
        }
    }
}

#[async_trait]
impl PipelineStage for ThumbnailStage {
    fn name(&self) -> &str {
        "ThumbnailStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.embed_thumbnail
    }

    fn is_fatal(&self) -> bool {
        false
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let media_file = msg.tracker.primary();
        let extension: String = media_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        // Auto-remux containers that don't support thumbnail embedding (e.g. .ts → .mp4).
        let (media_file, extension) = if !Self::supports_thumbnail(&extension) {
            let remuxed_path = msg.tracker.temp_path(&media_file, "mp4");
            debug!(
                "ThumbnailStage: auto-remuxing {} → mp4 for thumbnail embedding",
                media_file.display()
            );

            let opts = RemuxOptions {
                faststart: true,
                ..Default::default()
            };
            match self
                .ffmpeg
                .remux(&media_file, &remuxed_path, &opts, None)
                .await
            {
                Ok(()) => {
                    msg.tracker.replace(vec![remuxed_path.clone()]);
                    (remuxed_path, "mp4".to_string())
                }
                Err(e) => {
                    warn!("ThumbnailStage: auto-remux to MP4 failed, skipping: {e}");
                    msg.warnings
                        .push(format!("Auto-remux for thumbnail embedding failed: {e}"));
                    msg.tracker.mark_temp(remuxed_path);
                    return Ok(msg);
                }
            }
        } else {
            (media_file, extension)
        };

        // Use original_stem for thumbnail discovery (per architecture constraint 5).
        let Some(thumbnail_file) = Self::find_thumbnail(&media_file, &msg.original_stem) else {
            debug!(
                "ThumbnailStage: no thumbnail file found for stem '{}'",
                msg.original_stem
            );
            msg.warnings.push(format!(
                "Thumbnail file not found for '{}'",
                msg.original_stem
            ));
            return Ok(msg);
        };

        info!(
            "ThumbnailStage: embedding thumbnail {} into {}",
            thumbnail_file.display(),
            media_file.display()
        );

        let temp_output = msg.tracker.temp_path(&media_file, &extension);

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_forwarder = stage_callback.as_ref().map(|cb| {
            let cb = cb.clone();
            rdlp_ffmpeg::LogForwarderGuard::new(std::sync::Arc::new(
                move |level: i32, msg: String| {
                    let trimmed = msg.trim_end();
                    if trimmed.is_empty() {
                        return;
                    }
                    let prefixed = match level {
                        l if l <= 16 => format!("[ERROR] {trimmed}"),
                        24 => format!("[WARN] {trimmed}"),
                        _ => trimmed.to_string(),
                    };
                    cb.on_log(&prefixed);
                },
            ))
        });
        let log_callback = if msg.config.verbose {
            stage_callback
        } else {
            None
        };

        match self
            .ffmpeg
            .embed_thumbnail(
                &media_file,
                &thumbnail_file,
                &temp_output,
                &extension,
                log_callback,
            )
            .await
        {
            Ok(()) => {
                debug!(
                    "ThumbnailStage: thumbnail embedded via FFmpeg: {}",
                    media_file.display()
                );

                // Promote output — old file becomes temp.
                msg.tracker.replace(vec![temp_output.clone()]);

                // For MP4-family: write covr atom for Windows Explorer.
                if Self::is_mp4_family(&extension) {
                    Self::write_covr_atom(&temp_output, &thumbnail_file).await;
                }

                // Clean up thumbnail unless --write-thumbnail was requested.
                if !msg.config.write_thumbnail {
                    msg.tracker.mark_temp(thumbnail_file);
                }
            }
            Err(e) => {
                warn!("ThumbnailStage: failed to embed thumbnail: {e}");
                msg.warnings
                    .push(format!("Thumbnail embedding failed: {e}"));
                msg.tracker.mark_temp(temp_output);
            }
        }

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_core::PostProcessConfig;
    use rdlp_types::InfoDict;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcessConfig) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test Video".to_string(),
                "TestExtractor".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(config),
            original_stem: "test".to_string(),
            is_hls: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn should_run_when_embed_thumbnail() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcessConfig {
            embed_thumbnail: true,
            ..PostProcessConfig::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcessConfig::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_not_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);
        assert!(!stage.is_fatal());
    }

    #[test]
    fn supports_thumbnail_containers() {
        assert!(ThumbnailStage::supports_thumbnail("mp4"));
        assert!(ThumbnailStage::supports_thumbnail("mkv"));
        assert!(ThumbnailStage::supports_thumbnail("mp3"));
        assert!(ThumbnailStage::supports_thumbnail("flac"));
        assert!(!ThumbnailStage::supports_thumbnail("ts"));
        assert!(!ThumbnailStage::supports_thumbnail("avi"));
    }

    #[test]
    fn is_mp4_family() {
        assert!(ThumbnailStage::is_mp4_family("mp4"));
        assert!(ThumbnailStage::is_mp4_family("m4a"));
        assert!(ThumbnailStage::is_mp4_family("mov"));
        assert!(!ThumbnailStage::is_mp4_family("mkv"));
        assert!(!ThumbnailStage::is_mp4_family("mp3"));
    }

    #[test]
    fn find_thumbnail_returns_none_for_missing() {
        let result = ThumbnailStage::find_thumbnail(
            &PathBuf::from("/nonexistent/video.mp4"),
            "original-title",
        );
        assert!(result.is_none());
    }

    #[test]
    fn find_thumbnail_finds_by_original_stem() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let thumb = dir.path().join("original-title.jpg");
        fs::write(&thumb, b"fake-jpg").unwrap();

        let media = dir.path().join("original-title.rdlp-tmp-abc123.mp4");
        let result = ThumbnailStage::find_thumbnail(&media, "original-title");
        assert_eq!(result, Some(thumb));
    }

    #[tokio::test]
    async fn process_warns_when_no_thumbnail_found() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcessConfig {
            embed_thumbnail: true,
            ..PostProcessConfig::default()
        };
        let mut msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        msg.original_stem = "nonexistent-stem".to_string();

        let result = stage.process(msg).await.unwrap();
        assert!(
            !result.warnings.is_empty(),
            "expected warning about missing thumbnail"
        );
    }
}
