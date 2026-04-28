//! RecodeStage — transcodes video to a different container format.
//!
//! This stage runs at index 4 when `config.recode_video` is `Some` or
//! `config.recode_container` is `Some`.
//! Uses `rdlp_ffmpeg::FFmpegRunner::convert_video()`.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info, warn};

use rdlp_ffmpeg::ffmpeg::{audio_encoder_registry, video_codecs};
use rdlp_ffmpeg::{FFmpegRunner, PostProcessError, VideoConvertOptions};
use rdlp_types::{ContainerFormat, RecodeAudioMode};

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

    /// Determine if stream copy (remux) is possible for the codec/container
    /// combination. `video_codec` is the input file's video codec name as
    /// reported by ffprobe — still a string until the cross-extractor
    /// codec-typing migration lands.
    fn can_remux(input_ext: &str, output: ContainerFormat, video_codec: Option<&str>) -> bool {
        fn codec_is(codec: &str, names: &[&str]) -> bool {
            names.iter().any(|n| n.eq_ignore_ascii_case(codec))
        }

        match output {
            ContainerFormat::Mp4 | ContainerFormat::F4v => video_codec
                .is_some_and(|c| codec_is(c, &["h264", "avc", "h265", "hevc", "mpeg4", "av1"])),
            ContainerFormat::Mkv
            | ContainerFormat::Mka
            | ContainerFormat::Nut
            | ContainerFormat::Mxf
            | ContainerFormat::Avi => true,
            ContainerFormat::WebM | ContainerFormat::Ivf => {
                video_codec.is_some_and(|c| codec_is(c, &["vp8", "vp9", "av1"]))
            }
            ContainerFormat::ThreeGp => {
                video_codec.is_some_and(|c| codec_is(c, &["h264", "avc", "h263", "mpeg4"]))
            }
            ContainerFormat::Asf => {
                video_codec.is_some_and(|c| codec_is(c, &["wmv1", "wmv2", "h264", "avc", "mpeg4"]))
            }
            ContainerFormat::Mpg | ContainerFormat::Vob => video_codec.is_some_and(|c| {
                codec_is(c, &["mpeg1", "mpeg1video", "mpeg2", "mpeg2video", "mpeg4"])
            }),
            // Audio-only and any-other-video containers fall back to the
            // input/output ext compare.
            other => input_ext.eq_ignore_ascii_case(other.as_ext()),
        }
    }

    /// Pick the default video codec to encode toward when no explicit
    /// encoder override is given.
    fn default_codec_for(target: ContainerFormat) -> &'static str {
        match target {
            ContainerFormat::WebM | ContainerFormat::Ivf => "vp9",
            ContainerFormat::Ogg => "theora",
            ContainerFormat::Mpg | ContainerFormat::Vob => "mpeg2",
            _ => "h264",
        }
    }

    /// Default `(preset, crf)` for a known target codec. Returns `(None, None)`
    /// for codecs without a meaningful default — the codec falls back to its
    /// FFmpeg-shipped defaults.
    fn default_preset_crf_for_codec(target_codec: &str) -> (Option<String>, Option<u32>) {
        match target_codec {
            "h264" | "h265" | "hevc" | "vvc" | "h266" => (Some("medium".to_string()), Some(23)),
            "vp9" | "vp8" => (None, Some(30)),
            "av1" => (None, Some(28)),
            _ => (None, None),
        }
    }

    /// Build video conversion options.
    ///
    /// Returns `None` when an explicit encoder override is requested but not
    /// available in this FFmpeg build. `audio_codec` is `None` for copy,
    /// `Some(name)` for re-encode.
    fn build_convert_options(
        target: ContainerFormat,
        can_remux: bool,
        encoder_override: Option<&str>,
        audio_copy: bool,
        audio_codec: Option<String>,
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
                audio_copy,
                audio_codec,
                ..Default::default()
            });
        }

        let target_codec = Self::default_codec_for(target);
        let encoder = video_codecs::resolve_encoder(target_codec);
        let (preset, crf) = Self::default_preset_crf_for_codec(target_codec);

        Some(VideoConvertOptions {
            remux_only: false,
            video_codec: encoder.map(String::from),
            preset,
            crf,
            audio_copy,
            audio_codec,
            ..Default::default()
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
        msg.config.recode_video.is_some() || msg.config.recode_container.is_some()
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        // Resolve target container: prefer recode_container, fallback to
        // recode_video, fallback again to MP4.
        let target = msg
            .config
            .recode_container
            .or(msg.config.recode_video)
            .unwrap_or_else(|| {
                debug!("RecodeStage: no recode target configured; defaulting to MP4");
                ContainerFormat::Mp4
            });
        let target_ext = target.as_ext();

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        if input_ext.eq_ignore_ascii_case(target_ext) && msg.config.video_encoder.is_none() {
            debug!(
                "RecodeStage: file already in target format ({target_ext}) and no encoder override, skipping"
            );
            return Ok(msg);
        }

        info!(
            "RecodeStage: converting {} → {target_ext}",
            input_file.display()
        );

        let media_info = self
            .ffmpeg
            .probe(&input_file)
            .await
            .context("recode stage: failed to probe input file")?;
        // When a video encoder is explicitly requested, always transcode — never remux
        let can_remux = msg.config.video_encoder.is_none()
            && Self::can_remux(input_ext, target, media_info.video_codec.as_deref());

        if can_remux {
            debug!("RecodeStage: remuxing (stream copy)");
        } else {
            debug!("RecodeStage: transcoding video");
        }

        // Resolve audio mode — force Copy when audio normalization is active
        // (normalizer already re-encoded audio; re-encoding again degrades quality)
        let recode_audio = if msg.config.normalize_audio {
            if !matches!(msg.config.recode_audio, RecodeAudioMode::Copy) {
                warn!(
                    "RecodeStage: normalize_audio is active — forcing audio copy mode \
                     (re-encoding normalized audio degrades quality)"
                );
            }
            RecodeAudioMode::Copy
        } else {
            msg.config.recode_audio.clone()
        };

        // Derive audio_copy / audio_codec from the resolved mode
        let (audio_copy, audio_codec) = if can_remux {
            // Remux path always copies audio — no re-encoding needed
            (true, None)
        } else {
            match recode_audio {
                RecodeAudioMode::Copy => (true, None),
                RecodeAudioMode::Auto => {
                    let encoder =
                        audio_encoder_registry::select_audio_encoder_for_container(target);
                    debug!("RecodeStage: auto audio encoder for {target_ext}: {encoder}");
                    (false, Some(encoder.to_string()))
                }
                RecodeAudioMode::Encoder { ref name } => {
                    if !audio_encoder_registry::container_supports_audio_codec(target, name) {
                        warn!(
                            "RecodeStage: audio codec '{name}' may not be compatible with \
                             container '{target_ext}'; proceeding anyway"
                        );
                    }
                    let resolved = audio_encoder_registry::resolve_audio_encoder(name)
                        .unwrap_or_else(|| {
                            warn!(
                                "RecodeStage: audio encoder '{name}' not available; \
                                 falling back to container default"
                            );
                            audio_encoder_registry::select_audio_encoder_for_container(target)
                        });
                    debug!("RecodeStage: using audio encoder: {resolved}");
                    (false, Some(resolved.to_string()))
                }
            }
        };

        let output_path = msg.tracker.temp_path(&input_file, target_ext);

        let mut opts = match Self::build_convert_options(
            target,
            can_remux,
            msg.config.video_encoder.as_deref(),
            audio_copy,
            audio_codec,
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

        opts.verbose = msg.verbose;

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        // Bridge FFmpeg logs to the callback for real-time encoder output
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());

        // Log recode parameters to the UI
        if let Some(ref cb) = stage_callback {
            let video_info = opts.video_codec.as_deref().unwrap_or("copy");
            let preset_info = opts.preset.as_deref().unwrap_or("default");
            let crf_info = opts.crf.map_or("default".to_string(), |c| c.to_string());
            cb.on_log(&format!(
                "Recode: video={video_info} preset={preset_info} crf={crf_info}"
            ));
            if let Some(ref ac) = opts.audio_codec {
                cb.on_log(&format!("Recode: audio={ac}"));
            } else if opts.audio_copy {
                cb.on_log("Recode: audio=copy");
            }
            cb.on_log(&format!("Recode: container={target_ext}"));
        }

        let progress_callback =
            stage_callback
                .as_ref()
                .cloned()
                .map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                    Arc::new(move |frac| cb.on_progress(frac))
                });

        let log_callback: Option<Arc<dyn Fn(&str) + Send + Sync>> = if opts.verbose {
            stage_callback
                .as_ref()
                .cloned()
                .map(|cb| -> Arc<dyn Fn(&str) + Send + Sync> {
                    Arc::new(move |msg| cb.on_log(msg))
                })
        } else {
            None
        };

        self.ffmpeg
            .convert_video(
                &input_file,
                &output_path,
                &opts,
                progress_callback,
                log_callback,
            )
            .await
            .context("recode stage failed")?;

        // Capture the encoding_tool for downstream pass-through stages.
        {
            let audio_part = if let Some(ref ac) = opts.audio_codec {
                ac.as_str()
            } else if opts.audio_copy {
                "copy"
            } else {
                "none"
            };
            let video_part = opts.video_codec.as_deref().unwrap_or("libx264");
            msg.encoding_tool = Some(format!("{video_part} + {audio_part}"));
        }

        if let Some(ref cb) = stage_callback {
            cb.on_log(&format!(
                "Recode: complete → {}",
                output_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
        }
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

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcess) -> PipelineMessage {
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
            verbose: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
            encoding_tool: None,
        }
    }

    #[test]
    fn should_run_when_recode_video() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = RecodeStage::new(ffmpeg);

        let config = PostProcess {
            recode_video: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
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
            PostProcess::default(),
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
    fn can_remux_h264_to_mp4() {
        assert!(RecodeStage::can_remux(
            "mkv",
            ContainerFormat::Mp4,
            Some("h264")
        ));
    }

    #[test]
    fn cannot_remux_vp9_to_mp4() {
        assert!(!RecodeStage::can_remux(
            "webm",
            ContainerFormat::Mp4,
            Some("vp9")
        ));
    }

    #[test]
    fn can_remux_anything_to_mkv() {
        assert!(RecodeStage::can_remux(
            "mp4",
            ContainerFormat::Mkv,
            Some("h264")
        ));
        assert!(RecodeStage::can_remux(
            "webm",
            ContainerFormat::Mkv,
            Some("vp9")
        ));
    }

    #[test]
    fn build_convert_options_remux() {
        let opts = RecodeStage::build_convert_options(ContainerFormat::Mp4, true, None, true, None)
            .unwrap();
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
    }

    #[test]
    fn build_convert_options_transcode_mp4() {
        let opts =
            RecodeStage::build_convert_options(ContainerFormat::Mp4, false, None, true, None)
                .unwrap();
        assert!(!opts.remux_only);
        assert!(opts.video_codec.is_some());
        assert_eq!(opts.preset, Some("medium".to_string()));
        assert_eq!(opts.crf, Some(23));
        assert!(opts.audio_copy);
        assert!(opts.audio_codec.is_none());
    }

    #[test]
    fn build_convert_options_with_audio_codec() {
        let opts = RecodeStage::build_convert_options(
            ContainerFormat::Mp4,
            false,
            None,
            false,
            Some("libopus".to_string()),
        )
        .unwrap();
        assert!(!opts.audio_copy);
        assert_eq!(opts.audio_codec, Some("libopus".to_string()));
    }

    #[test]
    fn build_convert_options_unavailable_encoder_returns_none() {
        let result = RecodeStage::build_convert_options(
            ContainerFormat::Mp4,
            false,
            Some("nonexistent_enc_xyz"),
            true,
            None,
        );
        assert!(result.is_none());
    }
}
