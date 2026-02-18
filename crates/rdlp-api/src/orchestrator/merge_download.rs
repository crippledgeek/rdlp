//! Parallel download coordination for merge (video + audio) downloads
//!
//! Downloads video-only and audio-only streams in parallel via `tokio::join!`,
//! then passes both files to the postprocessor pipeline where `FFmpegMerger`
//! handles the actual muxing.

use super::Orchestrator;
use super::errors::*;
use log::{debug, info, warn};
use rdlp_core::Format;
use std::path::{Path, PathBuf};

/// Result of a successful merge download
#[derive(Debug, PartialEq, Eq)]
pub(super) struct MergeDownloadOutcome {
    /// Path to the downloaded video file
    pub video_path: PathBuf,
    /// Path to the downloaded audio file
    pub audio_path: PathBuf,
    /// Whether either stream was HLS
    pub is_hls: bool,
}

impl Orchestrator {
    /// Download video and audio streams in parallel for merge.
    ///
    /// Uses `tokio::join!` to download both streams concurrently via the
    /// existing `download_with_cdn_fallback()` method. The actual FFmpeg
    /// merge is handled downstream by the `FFmpegMerger` postprocessor.
    ///
    /// # Arguments
    /// * `video` - Video-only format to download
    /// * `audio` - Audio-only format to download
    /// * `base_output_path` - Base output path (used to derive per-stream filenames)
    ///
    /// # Returns
    /// - `Ok(Some(outcome))` -- both downloads succeeded
    /// - `Ok(None)` -- user cancelled
    /// - `Err(e)` -- one or both downloads failed
    pub(super) async fn download_merge_pair(
        &self,
        video: &Format,
        audio: &Format,
        base_output_path: &Path,
    ) -> Result<Option<MergeDownloadOutcome>> {
        let video_path = self.merge_stream_path(base_output_path, video, "video");
        let audio_path = self.merge_stream_path(base_output_path, audio, "audio");

        info!(
            video_format = video.format_id.as_str(),
            audio_format = audio.format_id.as_str(),
            video_path:? = video_path.display(),
            audio_path:? = audio_path.display();
            "Starting parallel merge download"
        );

        // Download both streams in parallel
        let (video_result, audio_result) = tokio::join!(
            self.download_with_cdn_fallback(video, &video_path, 0),
            self.download_with_cdn_fallback(audio, &audio_path, 0),
        );

        // Handle results — clean up on failure
        let video_outcome = match video_result {
            Ok(Some(outcome)) => outcome,
            Ok(None) => {
                self.cleanup_merge_file(&video_path).await;
                self.cleanup_merge_file(&audio_path).await;
                return Ok(None);
            }
            Err(e) => {
                warn!("Video download failed: {e}");
                self.cleanup_merge_file(&video_path).await;
                self.cleanup_merge_file(&audio_path).await;
                return Err(e);
            }
        };

        match audio_result {
            Ok(Some(audio_outcome)) => {
                let is_hls = video_outcome.is_hls || audio_outcome.is_hls;
                debug!(
                    video_hls = video_outcome.is_hls,
                    audio_hls = audio_outcome.is_hls;
                    "Merge download complete"
                );
                Ok(Some(MergeDownloadOutcome {
                    video_path,
                    audio_path,
                    is_hls,
                }))
            }
            Ok(None) => {
                self.cleanup_merge_file(&video_path).await;
                self.cleanup_merge_file(&audio_path).await;
                Ok(None)
            }
            Err(e) => {
                warn!("Audio download failed: {e}");
                self.cleanup_merge_file(&video_path).await;
                self.cleanup_merge_file(&audio_path).await;
                Err(e)
            }
        }
    }

    /// Derive the output path for one stream of a merge download.
    ///
    /// Produces paths like `/output/dir/Title.video.f137.mp4` for the video
    /// stream and `/output/dir/Title.audio.f140.m4a` for the audio stream.
    fn merge_stream_path(&self, base_output_path: &Path, format: &Format, label: &str) -> PathBuf {
        let stem = base_output_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let ext = &format.ext;
        let dir = base_output_path.parent().unwrap_or(Path::new("."));
        dir.join(format!("{stem}.{label}.{}.{ext}", format.format_id))
    }

    /// Clean up a partially downloaded merge stream file.
    async fn cleanup_merge_file(&self, path: &Path) {
        if path.exists() {
            if let Err(e) = tokio::fs::remove_file(path).await {
                warn!(path:? = path.display(); "Failed to clean up merge temp file: {e}");
            } else {
                debug!(path:? = path.display(); "Cleaned up merge temp file");
            }
        }
    }
}
