//! Post-processing pipeline for downloaded files
//!
//! Handles FFmpeg remuxing, metadata embedding, and audio extraction.

use super::{Orchestrator, Result};
use log::{debug, info, warn};
use rdlp_core::PostProcessConfig;
use std::path::{Path, PathBuf};

impl Orchestrator {
    /// Convert Config to PostProcessConfig
    pub(super) fn to_postprocess_config(&self) -> PostProcessConfig {
        PostProcessConfig {
            extract_audio: self.config.extract_audio,
            audio_format: self.config.audio_format.clone(),
            audio_quality: self.config.audio_quality.clone(),
            recode_video: self.config.recode_video.clone(),
            merge_output_format: self.config.merge_output_format.clone(),
            embed_thumbnail: self.config.embed_thumbnail,
            embed_metadata: self.config.embed_metadata,
            embed_subtitles: self.config.embed_subtitles,
            keep_video: self.config.keep_video,
            ffmpeg_location: self.config.ffmpeg_location.clone(),
            ffmpeg_args: self.config.ffmpeg_args.clone(),
        }
    }

    /// Check if post-processing is needed based on configuration
    pub(super) fn needs_postprocessing(&self) -> bool {
        self.config.extract_audio
            || self.config.embed_metadata
            || self.config.embed_thumbnail
            || self.config.recode_video.is_some()
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
            // Set merge_output_format to trigger FFmpeg remux for container fixup
            // This ensures proper moov atom placement and timestamp fixing
            pp_config.merge_output_format = Some("mp4".to_string());
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
            let temp_path = file.with_extension(format!("fixed.{ext}"));

            // Remux using library bindings (stream copy + faststart)
            match registry.remux_faststart(file, &temp_path).await {
                Ok(()) => {
                    // Replace original with fixed file
                    if let Err(e) = tokio::fs::remove_file(file).await {
                        warn!("Could not remove original file: {e}");
                    }
                    if let Err(e) = tokio::fs::rename(&temp_path, file).await {
                        warn!("Could not rename fixed file: {e}");
                        output_files.push(temp_path);
                    } else {
                        info!("Post-processed: faststart enabled, container fixed");
                        output_files.push(file.clone());
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

    /// Run post-processing for state machine downloads (single videos)
    ///
    /// Runs FFmpeg remux on all downloads for consistent output quality:
    /// - Faststart (moov atom at beginning)
    /// - Fixed timestamps
    /// - Proper MP4 container structure
    ///
    /// Also runs user-requested post-processing (extract audio, embed metadata, etc.)
    pub(super) async fn run_postprocessing_for_state_machine(
        &self,
        output_path: &Path,
        is_hls: bool,
    ) -> Result<PathBuf> {
        // Run FFmpeg remux on all downloads (both HLS and HTTP) for consistent output
        // This moves moov atom to start (faststart), fixes timestamps, ensures proper container
        if let Some(fixed_files) = self.ffmpeg_remux(&[output_path.to_path_buf()]).await {
            if let Some(fixed_path) = fixed_files.into_iter().next() {
                // If user also requested additional post-processing, run it
                if self.needs_postprocessing() {
                    let info = rdlp_core::InfoDict::new(
                        "unknown".to_string(),
                        fixed_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        "unknown".to_string(),
                        "unknown".to_string(),
                    );

                    let result = self
                        .run_postprocessing(&info, vec![fixed_path.clone()], is_hls)
                        .await?;
                    if let Some(path) = result.into_iter().next() {
                        return Ok(path);
                    }
                }
                return Ok(fixed_path);
            }
        }

        // If FFmpeg remux failed, check if user requested any post-processing
        if self.needs_postprocessing() {
            // Create a minimal InfoDict for post-processing
            let info = rdlp_core::InfoDict::new(
                "unknown".to_string(),
                output_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                "unknown".to_string(),
                "unknown".to_string(),
            );

            let result = self
                .run_postprocessing(&info, vec![output_path.to_path_buf()], is_hls)
                .await?;
            if let Some(path) = result.into_iter().next() {
                return Ok(path);
            }
        }

        Ok(output_path.to_path_buf())
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
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                // Match pattern: base_name.part{number}
                if filename.starts_with(base_name) && filename.contains(".part") {
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
            }
        }

        if deleted > 0 {
            info!(deleted; "Cleaned up leftover segment files");
        }
    }
}
