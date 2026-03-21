//! RecodeStage — transcodes video to a different container format.
//!
//! This stage runs at index 4 when `config.recode_video` is `Some`.
//! Uses `rdlp_ffmpeg::FFmpegRunner::convert_video()`.

use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};

use rdlp_core::ContainerFormat;
use rdlp_ffmpeg::{FFmpegRunner, PostProcessError, VideoConvertOptions};
use rdlp_ffmpeg::ffmpeg::video_codecs;

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Transcodes video to a different container/codec.
///
/// `should_run` triggers when `config.recode_video` is `Some`.
pub struct RecodeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl RecodeStage {
    /// Create a new `RecodeStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Check if the format is a supported container.
    fn is_supported_container(format: &str) -> bool {
        format.parse::<ContainerFormat>().is_ok()
    }

    /// Determine if stream copy (remux) is possible for the codec/container combination.
    fn can_remux(input_ext: &str, output_ext: &str, video_codec: Option<&str>) -> bool {
        fn codec_is(codec: &str, names: &[&str]) -> bool {
            names.iter().any(|n| n.eq_ignore_ascii_case(codec))
        }
        fn ext_is(ext: &str, exts: &[&str]) -> bool {
            exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
        }

        if ext_is(output_ext, &["mp4", "f4v"]) {
            video_codec
                .is_some_and(|c| codec_is(c, &["h264", "avc", "h265", "hevc", "mpeg4", "av1"]))
        } else if ext_is(output_ext, &["mkv", "mka", "nut", "mxf"]) {
            true
        } else if output_ext.eq_ignore_ascii_case("webm") {
            video_codec.is_some_and(|c| codec_is(c, &["vp8", "vp9", "av1"]))
        } else if output_ext.eq_ignore_ascii_case("ivf") {
            video_codec.is_some_and(|c| codec_is(c, &["vp8", "vp9", "av1"]))
        } else if output_ext.eq_ignore_ascii_case("3gp") {
            video_codec.is_some_and(|c| codec_is(c, &["h264", "avc", "h263", "mpeg4"]))
        } else if output_ext.eq_ignore_ascii_case("asf") {
            video_codec.is_some_and(|c| codec_is(c, &["wmv1", "wmv2", "h264", "avc", "mpeg4"]))
        } else if ext_is(output_ext, &["mpg", "vob"]) {
            video_codec.is_some_and(|c| {
                codec_is(c, &["mpeg1", "mpeg1video", "mpeg2", "mpeg2video", "mpeg4"])
            })
        } else if output_ext.eq_ignore_ascii_case("avi") {
            true
        } else {
            input_ext.eq_ignore_ascii_case(output_ext)
        }
    }

    /// Build video conversion options.
    ///
    /// Returns `None` when an explicit encoder override is requested but not available.
    fn build_convert_options(
        target_format: &str,
        can_remux: bool,
        encoder_override: Option<&str>,
    ) -> Option<VideoConvertOptions> {
        if can_remux {
            return Some(VideoConvertOptions {
                remux_only: true,
                audio_copy: true,
                ..Default::default()
            });
        }

        if let Some(requested) = encoder_override {
            let encoder_name = video_codecs::resolve_encoder(requested)?;
            let (preset, crf) = Self::default_preset_crf(encoder_name);
            return Some(VideoConvertOptions {
                remux_only: false,
                video_codec: Some(encoder_name.to_string()),
                preset,
                crf,
                audio_copy: true,
            });
        }

        let target_codec = match target_format {
            "webm" | "ivf" => "vp9",
            "ogg" => "theora",
            "mpg" | "vob" => "mpeg2",
            _ => "h264",
        };

        let encoder = video_codecs::resolve_encoder(target_codec);
        let (preset, crf) = match target_codec {
            "h264" | "h265" | "hevc" | "vvc" | "h266" => (Some("medium".to_string()), Some(23)),
            "vp9" | "vp8" => (None, Some(30)),
            "av1" => (None, Some(28)),
            _ => (None, None),
        };

        Some(VideoConvertOptions {
            remux_only: false,
            video_codec: encoder.map(String::from),
            preset,
            crf,
            audio_copy: true,
        })
    }

    fn default_preset_crf(encoder: &str) -> (Option<String>, Option<u32>) {
        if encoder.contains("264") || encoder.contains("265") || encoder.contains("kvazaar") {
            (Some("medium".to_string()), Some(23))
        } else if encoder.contains("vpx") {
            (None, Some(30))
        } else if encoder.contains("av1")
            || encoder.contains("svt")
            || encoder.contains("aom")
            || encoder.contains("rav1e")
        {
            (None, Some(28))
        } else {
            (None, None)
        }
    }
}

#[async_trait]
impl PipelineStage for RecodeStage {
    fn name(&self) -> &str {
        "RecodeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.recode_video.is_some()
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        let target_format = match msg.config.recode_video {
            Some(c) => c.as_ext(),
            None => {
                debug!("RecodeStage: no recode target configured; defaulting to MP4");
                "mp4"
            }
        };

        if !Self::is_supported_container(target_format) {
            return Err(PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "video conversion".to_string(),
            }
            .into());
        }

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if input_ext.eq_ignore_ascii_case(target_format) {
            debug!(
                "RecodeStage: file already in target format ({}), skipping",
                target_format
            );
            return Ok(msg);
        }

        info!(
            "RecodeStage: converting {} → {}",
            input_file.display(),
            target_format
        );

        let media_info = self.ffmpeg.probe(&input_file).await?;
        let can_remux = Self::can_remux(input_ext, target_format, media_info.video_codec.as_deref());

        if can_remux {
            debug!("RecodeStage: remuxing (stream copy)");
        } else {
            debug!("RecodeStage: transcoding video");
        }

        let output_path = msg.tracker.temp_path(&input_file, target_format);

        let opts = match Self::build_convert_options(
            target_format,
            can_remux,
            msg.config.video_encoder.as_deref(),
        ) {
            Some(o) => o,
            None => {
                let requested = msg.config.video_encoder.as_deref().unwrap_or("");
                return Err(PostProcessError::UnsupportedFormat {
                    format: requested.to_string(),
                    operation: format!(
                        "video encoder '{requested}' is not available in this FFmpeg build"
                    ),
                }
                .into());
            }
        };

        let callback = msg
            .callback_factory
            .as_ref()
            .map(|f| f(self.name()))
            .map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                Arc::new(move |frac| cb.on_progress(frac))
            });

        self.ffmpeg
            .convert_video(&input_file, &output_path, &opts, callback)
            .await?;

        info!("RecodeStage: converted to {}", output_path.display());

        msg.tracker.replace(vec![output_path]);

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_core::{InfoDict, PostProcessConfig};

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcessConfig) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test".to_string(),
                "Test".to_string(),
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
    fn should_run_when_recode_video() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);

        let config = PostProcessConfig {
            recode_video: Some(ContainerFormat::Mkv),
            ..PostProcessConfig::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcessConfig::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }

    #[test]
    fn is_supported_container() {
        assert!(RecodeStage::is_supported_container("mp4"));
        assert!(RecodeStage::is_supported_container("mkv"));
        assert!(RecodeStage::is_supported_container("webm"));
        assert!(!RecodeStage::is_supported_container("xyz"));
    }

    #[test]
    fn can_remux_h264_to_mp4() {
        assert!(RecodeStage::can_remux("mkv", "mp4", Some("h264")));
    }

    #[test]
    fn cannot_remux_vp9_to_mp4() {
        assert!(!RecodeStage::can_remux("webm", "mp4", Some("vp9")));
    }

    #[test]
    fn can_remux_anything_to_mkv() {
        assert!(RecodeStage::can_remux("mp4", "mkv", Some("h264")));
        assert!(RecodeStage::can_remux("webm", "mkv", Some("vp9")));
    }

    #[test]
    fn build_convert_options_remux() {
        let opts = RecodeStage::build_convert_options("mp4", true, None).unwrap();
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
    }

    #[test]
    fn build_convert_options_transcode_mp4() {
        let opts = RecodeStage::build_convert_options("mp4", false, None).unwrap();
        assert!(!opts.remux_only);
        assert!(opts.video_codec.is_some());
        assert_eq!(opts.preset, Some("medium".to_string()));
        assert_eq!(opts.crf, Some(23));
    }

    #[test]
    fn build_convert_options_unavailable_encoder_returns_none() {
        let result = RecodeStage::build_convert_options("mp4", false, Some("nonexistent_enc_xyz"));
        assert!(result.is_none());
    }
}
