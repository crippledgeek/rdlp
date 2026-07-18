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
use rdlp_types::{THUMBNAIL_EXTENSIONS, ThumbnailFormat};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Containers that support thumbnail embedding via `FFmpeg`.
const SUPPORTED_CONTAINERS: &[&str] = &[
    "mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac", "ogg", "opus",
];

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
    /// Searches `{parent}/{original_stem}.{ext}` for each `ext` in
    /// [`rdlp_types::THUMBNAIL_EXTENSIONS`]. Also tries the current file's stem
    /// as a fallback.
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

    /// Normalize `thumbnail_file` to a tracker-owned temp `.jpg` when the
    /// target container can't consume its codec directly.
    ///
    /// Every non-Matroska embed strategy stream-copies the thumbnail's source
    /// codec into the target container (see `rdlp_ffmpeg::thumbnail`), so a
    /// codec the target muxer has no tag for (e.g. `webp` in MP4) must be
    /// transcoded first or the mux fails. Whether a tag exists is asked of
    /// `FFmpeg` via `container_accepts_image_codec` rather than hardcoded, so
    /// the answer always tracks the linked build instead of a list that can
    /// drift from it. Matroska is exempt — it carries the image as a file
    /// attachment rather than a stream, so no codec tag is involved.
    ///
    /// On transcode failure, falls back to the original thumbnail path so the
    /// caller's existing non-fatal embed-failure handling still applies —
    /// which is why downstream steps re-check the bytes rather than assuming
    /// this returned something normalized.
    async fn normalize_thumbnail_for_embed(
        &self,
        msg: &mut PipelineMessage,
        extension: &str,
        thumbnail_file: &Path,
    ) -> PathBuf {
        // Matroska is checked FIRST and separately: it carries the thumbnail as
        // an attachment, not a stream, so the muxer's stream-codec answer below
        // is simply not the question that governs it — asking would force a
        // needless transcode for every MKV thumbnail.
        if Self::is_native_attachment_container(extension) {
            return thumbnail_file.to_path_buf();
        }

        // Ask the muxer whether it can carry this image's codec, rather than
        // consulting a hardcoded list (#525). This is the same codec-tag lookup
        // that produced the original failure, and it reads the image's real
        // codec — FFmpeg probes content, so a mislabeled `.jpg` holding webp is
        // identified as webp here regardless of its name. An error (unopenable
        // or undecodable image) answers "not accepted", so normalization, the
        // safe direction, is the default.
        if self
            .ffmpeg
            .container_accepts_image_codec(extension, thumbnail_file)
            .await
            .unwrap_or(false)
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
                // Mark the (possibly partial) temp jpg for cleanup on the success
                // path too, mirroring the auto-remux-failure path below — otherwise
                // a failed-transcode partial lingers until the TempRegistry sweep.
                msg.tracker.mark_temp(normalized);
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
            // Pick the covr image type from the BYTES, never the extension.
            // The previous extension-based branch labelled anything not named
            // `.png` as JPEG, so a `.jpg` file holding webp bytes was written
            // into the atom tagged as JPEG — precisely the mislabel #519 set
            // out to prevent, reachable again through #525's misnamed sidecar.
            // A `debug_assert` on the extension could not catch it either,
            // since the extension was the thing that lied.
            //
            // Content outside mp4ameta's supported rasters is refused rather
            // than mislabeled. This is non-fatal: the FFmpeg `attached_pic`
            // pass has already embedded the thumbnail, so skipping covr costs
            // Windows Explorer visibility, not the thumbnail itself.
            //
            // The arms are listed exhaustively rather than with a catch-all so
            // that adding a ThumbnailFormat variant fails to compile here,
            // forcing a decision about whether mp4ameta can represent it. A
            // string match could not do that — which is how the previous
            // coupling to the embeddable set was lost silently.
            let img = match rdlp_types::sniff_thumbnail_format(&cover_bytes) {
                Some(ThumbnailFormat::Jpeg) => mp4ameta::Img::jpeg(cover_bytes),
                Some(ThumbnailFormat::Png) => mp4ameta::Img::png(cover_bytes),
                Some(ThumbnailFormat::Bmp) => mp4ameta::Img::bmp(cover_bytes),
                Some(
                    format @ (ThumbnailFormat::Gif | ThumbnailFormat::Tiff | ThumbnailFormat::WebP),
                ) => anyhow::bail!(
                    "thumbnail stage: covr atom cannot represent {} content",
                    format.extension()
                ),
                None => anyhow::bail!(
                    "thumbnail stage: covr atom requires a recognized image, found \
                     unrecognized data"
                ),
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
                // `embed_source` is USUALLY normalized by
                // `normalize_thumbnail_for_embed` above — but not guaranteed:
                // that helper falls back to the original file when the
                // transcode fails. `write_covr_atom` therefore re-checks the
                // bytes itself and refuses what mp4ameta cannot represent,
                // rather than trusting an invariant that can be violated.
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

    /// Forward direction: every string `SUPPORTED_CONTAINERS` (the REAL
    /// const, not a copy) advertises as thumbnail-capable must actually
    /// resolve to an embed strategy in `rdlp-ffmpeg`.
    ///
    /// This and the reverse-direction test below replace what used to be a
    /// same-crate agreement test in `rdlp-ffmpeg` asserting `for_container`
    /// against `MIRRORED_SUPPORTED_CONTAINERS` — a hand-copied duplicate of
    /// this very list. That guarded against nothing: editing the real
    /// `SUPPORTED_CONTAINERS` below left both the copy and the test green.
    /// Asserting against `rdlp_ffmpeg::supports_thumbnail_embed` here closes
    /// that gap (full consolidation of the two lists tracked by #533).
    #[test]
    fn supported_containers_all_resolve_to_a_strategy() {
        use std::str::FromStr as _;

        use rdlp_types::ContainerFormat;

        for container in SUPPORTED_CONTAINERS {
            let format = ContainerFormat::from_str(container)
                .unwrap_or_else(|_| panic!("'{container}' must parse as a ContainerFormat"));
            assert!(
                rdlp_ffmpeg::supports_thumbnail_embed(format),
                "'{container}' is listed in SUPPORTED_CONTAINERS but \
                 rdlp_ffmpeg::supports_thumbnail_embed returned false"
            );
        }
    }

    /// Reverse direction: every container `rdlp-ffmpeg` resolves an embed
    /// strategy for must be advertised by `SUPPORTED_CONTAINERS` — otherwise
    /// `ThumbnailStage` would never reach this code for that container at
    /// all, and the two would have quietly diverged.
    #[test]
    fn every_strategy_supported_format_is_in_supported_containers() {
        use strum::IntoEnumIterator as _;

        use rdlp_types::ContainerFormat;

        for format in ContainerFormat::iter() {
            if rdlp_ffmpeg::supports_thumbnail_embed(format) {
                // `as_ext()` (not `Display`/`to_string()`) is the canonical
                // extension string: `ContainerFormat`'s `Display` impl does
                // not necessarily print the same alias `SUPPORTED_CONTAINERS`
                // uses (e.g. `Mkv`'s `Display` is "matroska", not "mkv").
                let name = format.as_ext();
                assert!(
                    SUPPORTED_CONTAINERS.contains(&name),
                    "'{name}' resolves to a thumbnail strategy but is missing from \
                     SUPPORTED_CONTAINERS"
                );
            }
        }
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

    /// #521: discovery must locate the widened raster formats
    /// (`gif`/`bmp`/`tiff`), not only the original `jpg`/`jpeg`/`png`/`webp`.
    /// Fails against the pre-#521 list, which stopped at `webp` and so returned
    /// `None` for a `gif`/`bmp`/`tiff` sidecar (silent thumbnail skip).
    #[test]
    fn find_thumbnail_finds_widened_raster_formats() {
        use std::fs;
        use tempfile::TempDir;

        for ext in ["gif", "bmp", "tiff", "tif"] {
            let dir = TempDir::new().unwrap();
            let thumb = dir.path().join(format!("clip.{ext}"));
            fs::write(&thumb, b"fake-image").unwrap();

            let media = dir.path().join("clip.rdlp-tmp-abc123.mp4");
            let result = ThumbnailStage::find_thumbnail(&media, "clip");
            assert_eq!(
                result,
                Some(thumb),
                "find_thumbnail must discover a .{ext} sidecar via original_stem"
            );
        }
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
