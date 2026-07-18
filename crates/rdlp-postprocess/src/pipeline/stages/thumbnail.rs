//! `ThumbnailStage` — embeds thumbnail images into media files.
//!
//! This stage runs at index 7 (last) when `config.embed_thumbnail` is true.
//! Uses `msg.original_stem` for thumbnail discovery (not the UUID-renamed stem).
//! Non-fatal: failure logs a warning and passes through.
//!
//! For MP4-family containers: two-pass embedding —
//! 1. `FFmpeg` `attached_pic` stream (media player support)
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

/// Containers that support thumbnail embedding via `FFmpeg`.
const SUPPORTED_CONTAINERS: &[&str] = &[
    "mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac", "ogg", "opus",
];

/// Extensions the non-Matroska embed passes (`FFmpeg` `ATTACHED_PIC` stream copy,
/// `mp4ameta` `covr` atom) can consume without transcoding.
///
/// This is the SINGLE source of truth for "already MP4/attached-pic-safe" — used
/// both to decide whether a thumbnail needs normalizing (see `process`) and,
/// after normalization, to select the `covr` atom's image type (see
/// `write_covr_atom`). Every non-Matroska embed strategy in
/// `rdlp_ffmpeg::thumbnail` stream-copies the thumbnail's source codec into the
/// target container (see `embed_thumbnail_sync`), so any format outside this
/// list (e.g. `webp`) must be normalized first — the target containers have no
/// muxer tag for it.
const EMBEDDABLE_RASTER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// Containers that attach the thumbnail's source codec natively (Matroska) and
/// therefore never need image normalization.
const NATIVE_ATTACHMENT_CONTAINERS: &[&str] = &["mkv", "mka"];

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
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Find a thumbnail file using `original_stem` for discovery.
    ///
    /// Searches `{parent}/{original_stem}.{jpg|jpeg|png|webp}`.
    /// Also tries the current file's stem as a fallback.
    fn find_thumbnail(media_file: &Path, original_stem: &str) -> Option<PathBuf> {
        let parent = media_file.parent()?;

        // Try original_stem first (most accurate after UUID renames).
        if let Some(path) = THUMBNAIL_EXTENSIONS
            .iter()
            .map(|ext| parent.join(format!("{original_stem}.{ext}")))
            .find(|path| path.exists())
        {
            return Some(path);
        }

        // Fallback: try current file stem.
        let current_stem = media_file.file_stem()?.to_str()?;
        if current_stem != original_stem {
            return THUMBNAIL_EXTENSIONS
                .iter()
                .map(|ext| parent.join(format!("{current_stem}.{ext}")))
                .find(|path| path.exists());
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

    /// Check if the container attaches the thumbnail's source codec natively
    /// (Matroska), so it never needs image normalization.
    fn is_native_attachment_container(extension: &str) -> bool {
        NATIVE_ATTACHMENT_CONTAINERS
            .iter()
            .any(|c| c.eq_ignore_ascii_case(extension))
    }

    /// Check whether `path`'s extension is already MP4/attached-pic-safe
    /// (see `EMBEDDABLE_RASTER_EXTENSIONS`).
    fn is_embeddable_raster(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                EMBEDDABLE_RASTER_EXTENSIONS
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(ext))
            })
    }

    /// Normalize `thumbnail_file` to a tracker-owned temp `.jpg` when the
    /// target container can't consume its codec directly.
    ///
    /// Every non-Matroska embed strategy stream-copies the thumbnail's source
    /// codec into the target container (see `rdlp_ffmpeg::thumbnail`), so a
    /// codec outside `EMBEDDABLE_RASTER_EXTENSIONS` (e.g. `webp`) must be
    /// transcoded first or the mux fails (MP4/MOV/M4A/M4V) or the `covr` atom
    /// silently mislabels raw webp bytes as JPEG (MP4-family only). Matroska
    /// is exempt — it attaches the source codec natively. On transcode
    /// failure, falls back to the original thumbnail path so the caller's
    /// existing non-fatal embed-failure handling still applies.
    async fn normalize_thumbnail_for_embed(
        &self,
        msg: &mut PipelineMessage,
        extension: &str,
        thumbnail_file: &Path,
    ) -> PathBuf {
        if Self::is_native_attachment_container(extension)
            || Self::is_embeddable_raster(thumbnail_file)
        {
            return thumbnail_file.to_path_buf();
        }

        let normalized = msg.tracker.temp_path(thumbnail_file, "jpg");
        debug!(
            "ThumbnailStage: normalizing thumbnail {} → jpg for {extension} embed",
            thumbnail_file.display()
        );
        match self
            .ffmpeg
            .transcode_image(thumbnail_file, &normalized)
            .await
        {
            Ok(()) => {
                msg.tracker.mark_temp(normalized.clone());
                normalized
            }
            Err(e) => {
                warn!("ThumbnailStage: thumbnail normalization to jpg failed, using original: {e}");
                msg.warnings
                    .push(format!("Thumbnail normalization to jpg failed: {e}"));
                thumbnail_file.to_path_buf()
            }
        }
    }

    /// Write the iTunes `covr` metadata atom for Windows Explorer thumbnail visibility.
    ///
    /// Non-fatal: logs a warning on failure.
    async fn write_covr_atom(media_file: &Path, thumbnail_file: &Path) {
        let media = media_file.to_path_buf();
        let thumb = thumbnail_file.to_path_buf();

        let result = tokio::task::spawn_blocking(move || {
            // Safe: inside spawn_blocking closure — explicitly the correct place for blocking I/O.
            #[allow(clippy::disallowed_methods)]
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
    fn name(&self) -> &'static str {
        "ThumbnailStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.embed_thumbnail
    }

    fn is_fatal(&self) -> bool {
        false
    }

    #[allow(clippy::too_many_lines)]
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
        let (media_file, extension) = if Self::supports_thumbnail(&extension) {
            (media_file, extension)
        } else {
            let remuxed_path = msg.tracker.temp_path(&media_file, "mp4");
            debug!(
                "ThumbnailStage: auto-remuxing {} → mp4 for thumbnail embedding",
                media_file.display()
            );

            let opts = RemuxOptions {
                faststart: true,
                encoding_tool_override: msg.encoding_tool.clone(),
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

        let embed_source = self
            .normalize_thumbnail_for_embed(&mut msg, &extension, &thumbnail_file)
            .await;

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
        let log_callback = if msg.verbose { stage_callback } else { None };

        match self
            .ffmpeg
            .embed_thumbnail(
                &media_file,
                &embed_source,
                &temp_output,
                &extension,
                log_callback,
                msg.encoding_tool.clone(),
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
                // `embed_source` is guaranteed EMBEDDABLE_RASTER_EXTENSIONS
                // (jpg/jpeg/png) by `normalize_thumbnail_for_embed` above.
                if Self::is_mp4_family(&extension) {
                    Self::write_covr_atom(&temp_output, &embed_source).await;
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
// Safe: test fixtures — no async runtime in #[test] fns.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcess) -> PipelineMessage {
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
            verbose: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn should_run_when_embed_thumbnail() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcess {
            embed_thumbnail: true,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);
        let config = PostProcess {
            embed_thumbnail: false,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
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

    /// Slice 2 (#406 Task 3): `original_stem` is the CLEAN stem (marker-stripped
    /// by the orchestrator), while the main file in the tracker is seam-named
    /// (`My.Video.rdlp-tmp-<uuid>.mp4`). `find_thumbnail` must resolve the
    /// sidecar `My.Video.jpg` via `original_stem`, NOT via the seam-named stem
    /// (which would produce `My.Video.rdlp-tmp-<uuid>.jpg` — a non-existent path).
    #[test]
    fn thumbnail_discovery_uses_clean_original_stem_under_seam() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Thumbnail is written next to the clean stem, as the orchestrator produces.
        let thumb = dir.path().join("My.Video.jpg");
        fs::write(&thumb, b"fake-jpg").unwrap();

        // The media file on disk is seam-named (as FileTracker sees it at this stage).
        let seam_media = dir
            .path()
            .join("My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678.mp4");
        // original_stem carries the clean stem — set by Task 3 in the orchestrator.
        let original_stem = "My.Video";

        let result = ThumbnailStage::find_thumbnail(&seam_media, original_stem);
        assert_eq!(
            result,
            Some(thumb),
            "find_thumbnail must find My.Video.jpg via original_stem \
             even when the media file is seam-named"
        );
    }

    /// Negative companion to `thumbnail_discovery_uses_clean_original_stem_under_seam`:
    /// pins the regression direction. With the same clean sidecar `My.Video.jpg` on disk
    /// and the same seam-named media file, but `original_stem` set to the SEAM stem itself
    /// (what an unfixed Task 3 would have done), `find_thumbnail` returns `None` because:
    /// - Primary lookup tries `{seam_stem}.jpg` (doesn't exist; the sidecar is `My.Video.jpg`)
    /// - Fallback skips because `current_stem` == `original_stem` (both are the seam stem)
    /// - Result: `None`
    #[test]
    fn thumbnail_discovery_misses_when_original_stem_is_seam_stem() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        // Sidecar is still written with the clean stem.
        let thumb = dir.path().join("My.Video.jpg");
        fs::write(&thumb, b"fake-jpg").unwrap();

        // The media file on disk is seam-named.
        let seam_media = dir
            .path()
            .join("My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678.mp4");
        // But original_stem is SET TO THE SEAM STEM itself (the regression).
        let original_stem = "My.Video.rdlp-tmp-deadbeef12345678deadbeef12345678";

        let result = ThumbnailStage::find_thumbnail(&seam_media, original_stem);
        assert_eq!(
            result, None,
            "find_thumbnail must return None when original_stem is the seam stem \
             (because the sidecar My.Video.jpg exists but lookup tries My.Video.rdlp-tmp-*.jpg)"
        );
    }

    #[tokio::test]
    async fn process_warns_when_no_thumbnail_found() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg);

        let config = PostProcess {
            embed_thumbnail: true,
            ..PostProcess::default()
        };
        let mut msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        msg.original_stem = "nonexistent-stem".to_string();

        let result = stage.process(msg).await.unwrap();
        assert!(
            !result.warnings.is_empty(),
            "expected warning about missing thumbnail"
        );
    }

    // --- is_embeddable_raster / is_native_attachment_container (bugfix/thumbnail-webp-mp4-embed) ---

    #[test]
    fn embeddable_raster_accepts_jpg_jpeg_png() {
        assert!(ThumbnailStage::is_embeddable_raster(&PathBuf::from(
            "/tmp/thumb.jpg"
        )));
        assert!(ThumbnailStage::is_embeddable_raster(&PathBuf::from(
            "/tmp/thumb.JPEG"
        )));
        assert!(ThumbnailStage::is_embeddable_raster(&PathBuf::from(
            "/tmp/thumb.png"
        )));
    }

    /// Negative: webp (the reported bug — xHamster and others serve webp
    /// thumbnails) and an extensionless path are NOT already embeddable —
    /// they must go through `normalize_thumbnail_for_embed`.
    #[test]
    fn embeddable_raster_rejects_webp_and_other() {
        assert!(!ThumbnailStage::is_embeddable_raster(&PathBuf::from(
            "/tmp/thumb.webp"
        )));
        assert!(!ThumbnailStage::is_embeddable_raster(&PathBuf::from(
            "/tmp/thumb.gif"
        )));
        assert!(!ThumbnailStage::is_embeddable_raster(&PathBuf::from(
            "/tmp/thumb"
        )));
    }

    #[test]
    fn native_attachment_container_accepts_mkv_mka() {
        assert!(ThumbnailStage::is_native_attachment_container("mkv"));
        assert!(ThumbnailStage::is_native_attachment_container("MKA"));
    }

    /// Negative: MP4-family (and other non-Matroska) containers must NOT be
    /// treated as native-attachment — they need normalization for non-raster
    /// codecs. Regression guard for the bug: an unfixed check that also
    /// exempted "mp4" would silently re-introduce the webp mux failure.
    #[test]
    fn native_attachment_container_rejects_mp4_family() {
        assert!(!ThumbnailStage::is_native_attachment_container("mp4"));
        assert!(!ThumbnailStage::is_native_attachment_container("mov"));
        assert!(!ThumbnailStage::is_native_attachment_container("mp3"));
    }
}
