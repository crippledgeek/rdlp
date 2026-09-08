//! Sequential download coordination for merge (video + audio) downloads
//!
//! Downloads video-only and audio-only streams sequentially (video first,
//! then audio) so progress updates don't interleave. Both files are then
//! passed to the postprocessor pipeline where `MergeStage` handles muxing.

use super::Orchestrator;
use super::errors::Result;
use crate::events::Event;
use log::{debug, info, warn};
use rdlp_types::Format;
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
    /// Download video and audio streams sequentially for merge.
    ///
    /// Downloads video first (progress 0-100%), then audio (progress resets
    /// 0-100%). Sequential download prevents progress bar jitter from two
    /// streams emitting interleaved progress updates to the same job ID.
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
        let video_path = Self::merge_stream_path(base_output_path, video, "video");
        let audio_path = Self::merge_stream_path(base_output_path, audio, "audio");

        info!(
            video_format = video.format_id.as_str(),
            audio_format = audio.format_id.as_str(),
            video_path:? = video_path.display(),
            audio_path:? = audio_path.display();
            "Starting sequential merge download (video then audio)"
        );

        // Download video first
        let _ = self.event_tx.try_send(Event::PlaylistItemStarted {
            id: self.download_id,
            index: 1,
            total: 2,
            url: video.url.clone(),
            title: "Video".to_string(),
        });
        let video_outcome = match self.download_with_cdn_fallback(video, &video_path, 0).await {
            Ok(Some(outcome)) => outcome,
            Ok(None) => {
                self.cleanup_merge_file(&video_path).await;
                return Ok(None);
            }
            Err(e) => {
                warn!("Video download failed: {e}");
                self.cleanup_merge_file(&video_path).await;
                return Err(e);
            }
        };
        let _ = self.event_tx.try_send(Event::UnitCompleted {
            id: self.download_id,
            index: 1,
        });

        // Then download audio
        let _ = self.event_tx.try_send(Event::PlaylistItemStarted {
            id: self.download_id,
            index: 2,
            total: 2,
            url: audio.url.clone(),
            title: "Audio".to_string(),
        });
        match self.download_with_cdn_fallback(audio, &audio_path, 0).await {
            Ok(Some(audio_outcome)) => {
                let _ = self.event_tx.try_send(Event::UnitCompleted {
                    id: self.download_id,
                    index: 2,
                });
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
    ///
    /// # Security
    ///
    /// `format.format_id` and `format.ext` are both extractor-populated from
    /// remote data, so this is the same stem+parent+`format!`+`join` shape as
    /// the subtitle sidecar. It shares
    /// [`container_resolver::sidecar_path`](super::container_resolver::sidecar_path)
    /// rather than repeating it, so the sanitization cannot drift between the
    /// two.
    fn merge_stream_path(base_output_path: &Path, format: &Format, label: &str) -> PathBuf {
        super::container_resolver::sidecar_path(
            base_output_path,
            &format!("{label}.{}.{}", format.format_id, format.ext),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `format_id` and `ext` are extractor-populated from remote data, so
    /// `merge_stream_path` carries the same injection surface as the
    /// subtitle sidecar. It shares `sidecar_path`, so this asserts the
    /// shared guarantee reaches it — on the resolved path, not a substring.
    #[test]
    fn merge_stream_path_neutralizes_hostile_format_metadata() {
        let format = Format::new(
            "../../etc",
            "https://example.test/v.mp4",
            "cron.d/pwn",
            rdlp_types::DownloadProtocol::Https,
        );
        let path =
            Orchestrator::merge_stream_path(Path::new("/tmp/out/video.mkv"), &format, "video");
        assert_eq!(path.parent(), Some(Path::new("/tmp/out")));
        assert!(
            !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
            "retains a `..` component: {path:?}"
        );
    }

    /// Drift guard, not a fix: no reachable `format_id`/`ext` forges a part
    /// name today (`sidecar_path`'s reserved-name rewrite sees to that), and
    /// merge streams are never resumed anyway. This fails if someone
    /// re-inlines the stem+join here for readability — merge is the only
    /// caller composing a multi-segment suffix, so it is where that would
    /// happen.
    #[test]
    fn merge_stream_path_never_forges_a_part_name() {
        let base = Path::new("/tmp/out/Title.mkv");
        let format = Format::new(
            "rdlp-part",
            "https://example.test/v.mp4",
            "mkv",
            rdlp_types::DownloadProtocol::Https,
        );
        for label in ["video", "audio"] {
            let path = Orchestrator::merge_stream_path(base, &format, label);
            assert_ne!(path, super::super::naming::part_path(base));
            let name = path.file_name().and_then(|n| n.to_str()).unwrap();
            assert!(
                !name.contains(super::super::naming::PART_MARKER),
                "{label}: name spells the part marker: {name:?}"
            );
        }
    }

    #[test]
    fn merge_stream_path_preserves_ordinary_format_metadata() {
        let format = Format::new(
            "f137",
            "https://example.test/v.mp4",
            "mp4",
            rdlp_types::DownloadProtocol::Https,
        );
        let path =
            Orchestrator::merge_stream_path(Path::new("/tmp/out/Title.mkv"), &format, "video");
        assert_eq!(path, PathBuf::from("/tmp/out/Title.video.f137.mp4"));
    }
}
