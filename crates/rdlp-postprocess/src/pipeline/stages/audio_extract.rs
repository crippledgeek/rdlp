//! `AudioExtractStage` — extracts audio from video files.
//!
//! This stage runs at index 1 when `config.extract_audio` is true.
//! Uses `rdlp_ffmpeg::FFmpegRunner::extract_audio()`.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::debug;

use rdlp_ffmpeg::ffmpeg::{AudioCodecConfig, AudioExtractOptions, get_audio_codec};
use rdlp_ffmpeg::{FFmpegRunner, PostProcessError};

use crate::pipeline::stages::audio_convert;
use crate::pipeline::{PipelineMessage, PipelineStage};

/// Extracts audio from the primary current file.
///
/// `should_run` triggers when `config.extract_audio` is true.
pub struct AudioExtractStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl AudioExtractStage {
    /// Create a new `AudioExtractStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build extraction options from codec config and quality string.
    pub(crate) fn build_extract_options(
        codec_config: &AudioCodecConfig,
        can_copy: bool,
        quality: Option<&str>,
    ) -> AudioExtractOptions {
        let mut opts = AudioExtractOptions {
            // `from_static` is only const-eval-safe (invalid input = build
            // error) when called from a `const`/static-table context; this is
            // a plain runtime function body, so `new_static` is the correct
            // form here — an invalid entry degrades to `None` rather than
            // aborting the process.
            encoder_name: codec_config
                .encoder
                .and_then(|e| rdlp_types::media_name::AudioEncoderName::new_static(e).ok()),
            copy: can_copy,
            ..Default::default()
        };

        if can_copy {
            return opts;
        }

        if let Some(q) = quality {
            if let Ok(q_num) = q.parse::<u32>() {
                if let Some((min, max)) = codec_config.bitrate_range {
                    opts.bitrate_kbps = Some(q_num.clamp(min, max));
                }
            } else if let Some((worst, best)) = codec_config.quality_scale {
                // `q` is a free-text user input. Recognise the named
                // shortcuts; if the value is anything else, log a warning
                // and skip the override rather than silently inserting the
                // mid-quality default — that masked typos in --audio-quality.
                let scale: Option<u8> = if q.eq_ignore_ascii_case("best") || q == "0" {
                    Some(best)
                } else if q.eq_ignore_ascii_case("worst") || q == "9" || q == "10" {
                    Some(worst)
                } else if let Ok(parsed) = q.parse::<u8>() {
                    Some(parsed)
                } else {
                    log::warn!(
                        "AudioExtractStage: unrecognised --audio-quality value {q:?}; \
                         skipping quality override and using codec defaults"
                    );
                    None
                };
                if let Some(s) = scale {
                    opts.quality_scale = Some(i32::from(s));
                }
            }
        }

        opts
    }
}

#[async_trait]
impl PipelineStage for AudioExtractStage {
    fn name(&self) -> &'static str {
        "AudioExtractStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.extract_audio
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        // Check if input has audio.
        let media_info = self
            .ffmpeg
            .probe(&input_file)
            .await
            .context("audio extract stage: failed to probe input file")?;
        if !media_info.has_audio {
            return Err(PostProcessError::NoAudioStream.into());
        }

        let target_format = if let Some(f) = msg.config.audio_format {
            f.codec_name()
        } else {
            debug!("AudioExtractStage: no audio format configured; defaulting to MP3");
            "mp3"
        };

        let codec_config =
            get_audio_codec(target_format).ok_or_else(|| PostProcessError::UnsupportedFormat {
                format: target_format.to_string(),
                operation: "audio extraction".to_string(),
            })?;

        let can_copy = media_info
            .audio_codec
            .as_ref()
            .is_some_and(|c| c == target_format || (c == "aac" && target_format == "m4a"));

        if can_copy {
            debug!("AudioExtractStage: audio codec matches target {target_format}, copying stream");
        }

        let output_path = msg.tracker.temp_path(&input_file, codec_config.extension);

        let opts = Self::build_extract_options(
            codec_config,
            can_copy,
            msg.config.audio_quality.as_deref(),
        );

        audio_convert::run_audio_extract(
            &self.ffmpeg,
            &mut msg,
            audio_convert::AudioExtractJob {
                stage_name: self.name(),
                input: input_file,
                output: output_path,
                opts,
                summary: None,
                // `wav`'s row carries no encoder name — `extract_audio` defers
                // to the muxer's PCM default — so the target codec is what
                // names the encoding for the tag. `target_format` is always
                // one of the static `AudioFormat::codec_name()` values (or the
                // `"mp3"` default), so this never actually degrades to `None`
                // in practice — but `new_static` (not `from_static`) is the
                // correct constructor for a runtime function body: an invalid
                // value would surface as a missing tag, not a process abort.
                fallback_codec: rdlp_types::CodecName::new_static(target_format).ok(),
                error_context: "audio extract stage failed",
            },
        )
        .await?;

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use rdlp_types::InfoDict;
    use rdlp_types::PostProcess;

    use crate::pipeline::{FileTracker, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcess) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
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
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn should_run_when_extract_audio() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = AudioExtractStage::new(ffmpeg);

        let config = PostProcess {
            extract_audio: true,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = AudioExtractStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcess::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = AudioExtractStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }

    #[test]
    fn build_extract_options_mp3_bitrate() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = AudioExtractStage::build_extract_options(codec_config, false, Some("192"));
        assert_eq!(opts.bitrate_kbps, Some(192));
        assert_eq!(opts.quality_scale, None);
        assert!(!opts.copy);
    }

    #[test]
    fn build_extract_options_copy_ignores_quality() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = AudioExtractStage::build_extract_options(codec_config, true, Some("192"));
        assert!(opts.copy);
        assert_eq!(opts.bitrate_kbps, None);
        assert_eq!(opts.quality_scale, None);
    }

    /// The `wav` row carries **no** encoder name — `extract_audio` defers to
    /// the muxer's PCM default — which is why the shared extract helper takes
    /// a `fallback_codec`. Without it `audio_tag_component` would see
    /// `(copy: false, encoder: None)` and tag the file `"none"`, losing the
    /// codec that the pre-#637 inline code recorded. Pins the shape the
    /// fallback exists to preserve.
    #[test]
    fn wav_row_has_no_encoder_so_the_tag_needs_the_codec_fallback() {
        let codec_config = get_audio_codec("wav").expect("wav row exists");
        assert_eq!(
            codec_config.encoder, None,
            "wav defers to the muxer's PCM default"
        );

        let opts = AudioExtractStage::build_extract_options(codec_config, false, None);
        assert_eq!(opts.encoder_name, None);

        // Without the fallback the tag degrades to "none"...
        assert_eq!(
            rdlp_ffmpeg::ffmpeg::audio_tag_component(
                opts.copy,
                opts.encoder_name
                    .as_ref()
                    .map(rdlp_types::media_name::MediaName::as_str)
            ),
            "none"
        );
        // ...and with it, the codec name is recorded, as before #637.
        assert_eq!(
            rdlp_ffmpeg::ffmpeg::audio_tag_component(opts.copy, Some("wav")),
            "wav"
        );
    }

    #[test]
    fn build_extract_options_bitrate_clamped() {
        let codec_config = get_audio_codec("mp3").unwrap();
        let opts = AudioExtractStage::build_extract_options(codec_config, false, Some("999"));
        assert_eq!(opts.bitrate_kbps, Some(320)); // clamped to max
        let opts = AudioExtractStage::build_extract_options(codec_config, false, Some("1"));
        assert_eq!(opts.bitrate_kbps, Some(32)); // clamped to min
    }
}
