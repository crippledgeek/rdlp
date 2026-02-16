//! Post-processing pipeline for downloaded files
//!
//! Handles FFmpeg remuxing, metadata embedding, and audio extraction.

use super::{Orchestrator, Result};
use log::{debug, info, warn};
use rdlp_core::PostProcessConfig;
use std::path::PathBuf;

impl Orchestrator {
    /// Convert Config to PostProcessConfig
    pub(super) fn to_postprocess_config(&self) -> PostProcessConfig {
        PostProcessConfig {
            extract_audio: self.config.extract_audio,
            audio_format: self.config.audio_format,
            audio_quality: self.config.audio_quality.clone(),
            recode_video: self.config.recode_video,
            remux_container: self.config.remux_container,
            merge_output_format: self.config.merge_output_format,
            embed_thumbnail: self.config.embed_thumbnail,
            write_thumbnail: self.config.write_thumbnail,
            embed_metadata: self.config.embed_metadata,
            embed_subtitles: self.config.embed_subtitles,
            write_subtitles: self.config.write_subtitles,
            keep_video: self.config.keep_video,
            ffmpeg_location: self.config.ffmpeg_location.clone(),
            ffmpeg_args: self.config.ffmpeg_args.clone(),
            normalize_audio: self.config.normalize_audio,
            loudnorm: self.config.loudnorm,
            audio_gain_target: self.config.audio_gain_target,
            loudnorm_preset: self.config.loudnorm_preset.clone(),
            loudnorm_target_i: self.config.loudnorm_target_i,
            loudnorm_target_tp: self.config.loudnorm_target_tp,
            loudnorm_target_lra: self.config.loudnorm_target_lra,
            loudnorm_dynamic: self.config.loudnorm_dynamic,
            loudnorm_precompress: self.config.loudnorm_precompress,
            normalize_boost: self.config.normalize_boost,
            normalize_boost_db: self.config.normalize_boost_db,
        }
    }

    /// Check if post-processing is needed based on configuration
    pub(super) fn needs_postprocessing(&self) -> bool {
        self.config.extract_audio
            || self.config.embed_metadata
            || self.config.embed_thumbnail
            || self.config.embed_subtitles
            || self.config.recode_video.is_some()
            || self.config.remux_container.is_some()
            || self.config.normalize_audio
    }

    /// Run post-processing pipeline on downloaded file(s)
    ///
    /// # Arguments
    /// * `info` - Video metadata
    /// * `files` - Downloaded file paths
    /// * `is_hls` - Whether this was an HLS download (triggers automatic remux)
    ///
    /// # Returns
    /// * `Ok(paths)` - Processed file paths (may differ from input if conversion occurred)
    /// * `Err(e)` - Post-processing failed
    pub(super) async fn run_postprocessing(
        &self,
        info: &rdlp_core::InfoDict,
        files: Vec<PathBuf>,
        is_hls: bool,
    ) -> Result<Vec<PathBuf>> {
        debug!(
            "[PostProcess] Called: is_hls={is_hls}, registry={}",
            self.postprocessor_registry.is_some()
        );

        let registry = match &self.postprocessor_registry {
            Some(r) => r,
            None => {
                // No post-processor available - return files unchanged
                if self.needs_postprocessing() || is_hls {
                    warn!("Post-processing unavailable (FFmpeg not found)");
                    if is_hls {
                        warn!("HLS downloads may have container issues without FFmpeg remux");
                    }
                }
                return Ok(files);
            }
        };

        // For HLS downloads, always run FFmpeg remux to fix container
        // For other downloads, only run if explicitly configured
        let needs_processing = self.needs_postprocessing() || is_hls;
        if !needs_processing {
            return Ok(files);
        }

        // Build config - for HLS, enable remux even if not explicitly requested
        let mut pp_config = self.to_postprocess_config();
        if is_hls && pp_config.recode_video.is_none() && !pp_config.extract_audio {
            // Use the central resolver to determine the container format,
            // respecting user config and file extension before falling back.
            let resolved = super::container_resolver::ResolvedContainer::resolve(
                &self.config,
                files.first().map(|f| f.as_path()),
            );
            pp_config.merge_output_format = Some(resolved.format);
        }

        info!("Running post-processing pipeline...");

        if self.config.verbose {
            let processors = registry.list_processors();
            debug!("Available processors: {}", processors.join(", "));
        }

        // Run FFmpeg remux to fix container (faststart, timestamps)
        // Applied to both HLS and HTTP downloads for consistent output
        let result_files = if !self.config.extract_audio {
            self.ffmpeg_remux(&files).await.unwrap_or(files.clone())
        } else {
            files.clone()
        };

        // Run the full post-processing pipeline
        match registry
            .process(info, result_files.clone(), &pp_config)
            .await
        {
            Ok(result) => {
                if result.files != files {
                    info!("Post-processing complete");
                    if self.config.verbose {
                        for file in &result.files {
                            debug!(path:? = file.display(); "Output");
                        }
                    }
                }
                Ok(result.files)
            }
            Err(e) => {
                warn!("Post-processing failed: {e}");
                // Return remuxed files on failure (or original if remux failed)
                Ok(result_files)
            }
        }
    }

    /// Run FFmpeg remux on downloaded files to fix container format
    ///
    /// This performs a stream copy (no re-encoding) via library bindings to:
    /// - Move moov atom to beginning of file (faststart)
    /// - Fix timestamps
    /// - Ensure proper MP4 container structure
    ///
    /// Applied to both HLS and HTTP downloads for consistent output quality.
    pub(super) async fn ffmpeg_remux(&self, files: &[PathBuf]) -> Option<Vec<PathBuf>> {
        let registry = self.postprocessor_registry.as_ref()?;
        let processors = registry.list_processors();
        if processors.is_empty() {
            return None;
        }

        let mut output_files = Vec::new();

        for file in files {
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("mp4");

            // MPEG-TS needs remux for seeking/thumbnails.
            // Use the resolver to determine the target container.
            let target_ext = if ext.eq_ignore_ascii_case("ts") {
                let resolved = super::container_resolver::ResolvedContainer::resolve(
                    &self.config,
                    Some(file.as_path()),
                );
                resolved.format.as_ext()
            } else {
                ext
            };
            let temp_path = file.with_extension(format!("fixed.{target_ext}"));

            // Remux using library bindings (stream copy + faststart)
            match registry.remux_faststart(file, &temp_path).await {
                Ok(()) => {
                    // Replace original with fixed file
                    if let Err(e) = tokio::fs::remove_file(file).await {
                        warn!("Could not remove original file: {e}");
                    }
                    let final_path = file.with_extension(target_ext);
                    if let Err(e) = tokio::fs::rename(&temp_path, &final_path).await {
                        warn!("Could not rename fixed file: {e}");
                        output_files.push(temp_path);
                    } else {
                        info!("Post-processed: faststart enabled, container fixed");
                        output_files.push(final_path);
                    }
                }
                Err(e) => {
                    debug!("FFmpeg remux failed: {e}");
                    // Clean up temp file
                    let _ = tokio::fs::remove_file(&temp_path).await;
                    output_files.push(file.clone());
                }
            }
        }

        Some(output_files)
    }

    /// Clean up leftover HLS segment files from interrupted downloads
    ///
    /// When HLS downloads are interrupted via Ctrl+C, segment files like
    /// `filename.part0`, `filename.part1`, etc. may be left behind.
    /// This function removes them before starting a new download.
    pub(super) async fn cleanup_leftover_segments(&self, dir: &std::path::Path, base_name: &str) {
        if !dir.exists() {
            return;
        }

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(_) => return,
        };

        let mut deleted = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
                continue;
            };

            // Match pattern: base_name.part{number}
            if !filename.starts_with(base_name) || !filename.contains(".part") {
                continue;
            }

            // Verify it's a segment file (has numeric suffix after .part)
            if let Some(part_idx) = filename.rfind(".part") {
                let suffix = &filename[part_idx + 5..];
                if suffix.chars().all(|c| c.is_ascii_digit())
                    && tokio::fs::remove_file(&path).await.is_ok()
                {
                    deleted += 1;
                }
            }
        }

        if deleted > 0 {
            info!(deleted; "Cleaned up leftover segment files");
        }
    }
}
