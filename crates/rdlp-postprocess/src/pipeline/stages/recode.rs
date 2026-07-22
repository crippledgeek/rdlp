//! `RecodeStage` — transcodes video to a different container format.
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

/// Named parameters for [`RecodeStage::build_convert_options`].
///
/// Replaces 4 positional arguments so the compiler catches argument-swap bugs,
/// especially the boolean `audio_copy` / `can_remux` pair.
#[derive(Debug, Clone)]
pub(super) struct RecodeParams {
    pub target: ContainerFormat,
    pub encoder_override: Option<String>,
    pub audio_copy: bool,
    pub audio_codec: Option<String>,
    /// Resolved-or-configured encoder thread count (None = auto at encode layer).
    pub threads: Option<u32>,
    /// Encoder preset override; None preserves the per-codec default preset.
    pub preset_override: Option<String>,
    /// VPX/AV1 deadline knob (e.g. `"good"`, `"best"`, `"realtime"`).
    pub deadline: Option<String>,
    /// VPX/AV1 cpu-used knob.
    pub cpu_used: Option<i32>,
    /// VVC/x265/SVT-AV1 speed-level knob.
    pub speed_level: Option<u32>,
}

/// Transcodes video to a different container/codec.
///
/// `should_run` triggers when `config.recode_video` is `Some`.
pub struct RecodeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl RecodeStage {
    /// Create a new `RecodeStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
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
            // All three ASF-family spellings share one muxer and therefore one
            // codec-compatibility answer. Listed explicitly rather than left to
            // the `other` arm below: falling through would silently swap this
            // codec check for an extension compare when #538 split the variants.
            ContainerFormat::Wmv | ContainerFormat::Wma | ContainerFormat::Asf => {
                video_codec.is_some_and(|c| codec_is(c, &["wmv1", "wmv2", "h264", "avc", "mpeg4"]))
            }
            ContainerFormat::Mpg | ContainerFormat::Vob => video_codec.is_some_and(|c| {
                codec_is(c, &["mpeg1", "mpeg1video", "mpeg2", "mpeg2video", "mpeg4"])
            }),
            // Audio-only and any-other-video containers fall back to the
            // input/output ext compare.
            //
            // Enumerated rather than left as `other =>` so a newly-added
            // `ContainerFormat` fails to compile here instead of silently
            // inheriting an extension compare in place of a codec check. #538
            // is why: splitting `Asf` into three variants added two containers
            // to this match, and a catch-all would have swapped the ASF
            // codec-compatibility list above for an ext compare with no build
            // error and no test failure. Same discipline as
            // `embed_strategy::for_container` (#525).
            ContainerFormat::Mov
            | ContainerFormat::M4v
            | ContainerFormat::Ts
            | ContainerFormat::Flv
            | ContainerFormat::Dv
            | ContainerFormat::Ogg
            | ContainerFormat::M4a
            | ContainerFormat::Mp3
            | ContainerFormat::Wav
            | ContainerFormat::Flac
            | ContainerFormat::Opus
            | ContainerFormat::Aac
            | ContainerFormat::Aiff
            | ContainerFormat::Wv
            | ContainerFormat::Caf
            | ContainerFormat::Ac3 => input_ext.eq_ignore_ascii_case(output.as_ext()),
        }
    }

    /// Pick the default video codec to encode toward when no explicit
    /// encoder override is given. Delegates to the single source in `rdlp-ffmpeg`
    /// so the recode pipeline and `validate_speed_controls` never resolve differently.
    const fn default_codec_for(target: ContainerFormat) -> &'static str {
        rdlp_ffmpeg::default_codec_for_container(target)
    }

    /// Default `(preset, crf)` for a known target codec. Returns `(None, None)`
    /// for codecs without a meaningful default — the codec falls back to its
    /// FFmpeg-shipped defaults.
    fn default_preset_crf_for_codec(target_codec: &str) -> (Option<String>, Option<u32>) {
        match target_codec {
            "h264" => (Some("medium".to_string()), Some(23)),
            // x265's native default is 28; reusing x264's 23 on x265 is ~5 CRF
            // points too low and yields oversized HEVC output.
            "h265" | "hevc" => (Some("medium".to_string()), Some(28)),
            "vp9" | "vp8" => (None, Some(30)),
            "av1" => (None, Some(28)),
            // VVC/EVC/AVS/APV are not x264-style-CRF encoders (libvvenc/libxavs2
            // take qp+preset; libxeve needs rc_mode=CRF) — defer to encoder defaults.
            _ => (None, None),
        }
    }

    /// Whether `encoder` can be muxed into `target`. AVS2 (`libxavs2`) only
    /// muxes into Matroska — `libxavs2 -> MP4` writes nothing — so a recode to
    /// any other container must be refused with a clear reason rather than
    /// producing an empty file.
    fn encoder_container_compatible(encoder: &str, target: ContainerFormat) -> bool {
        encoder != "libxavs2" || matches!(target, ContainerFormat::Mkv)
    }

    /// Build video conversion options.
    ///
    /// Returns `None` when an explicit encoder override is requested but not
    /// available in this `FFmpeg` build. `audio_codec` is `None` for copy,
    /// `Some(name)` for re-encode.
    fn build_convert_options(
        params: &RecodeParams,
        can_remux: bool,
    ) -> Option<VideoConvertOptions> {
        if can_remux {
            return Some(VideoConvertOptions {
                remux_only: true,
                audio_copy: true,
                ..Default::default()
            });
        }

        if let Some(ref requested) = params.encoder_override {
            let encoder_name = video_codecs::resolve_encoder(requested)?;
            let (default_preset, crf) = Self::default_preset_crf(encoder_name);
            let preset = params.preset_override.clone().or(default_preset);
            return Some(VideoConvertOptions {
                remux_only: false,
                video_codec: Some(encoder_name.to_string()),
                preset,
                crf,
                threads: params.threads,
                deadline: params.deadline.clone(),
                cpu_used: params.cpu_used,
                speed_level: params.speed_level,
                audio_copy: params.audio_copy,
                audio_codec: params.audio_codec.clone(),
                ..Default::default()
            });
        }

        let target_codec = Self::default_codec_for(params.target);
        let encoder = video_codecs::resolve_encoder(target_codec);
        let (default_preset, crf) = Self::default_preset_crf_for_codec(target_codec);
        let preset = params.preset_override.clone().or(default_preset);

        Some(VideoConvertOptions {
            remux_only: false,
            video_codec: encoder.map(String::from),
            preset,
            crf,
            threads: params.threads,
            deadline: params.deadline.clone(),
            cpu_used: params.cpu_used,
            speed_level: params.speed_level,
            audio_copy: params.audio_copy,
            audio_codec: params.audio_codec.clone(),
            ..Default::default()
        })
    }

    /// Resolves `(audio_copy, audio_codec)` for a transcode (never called on
    /// the remux fast path, which always copies audio unconditionally).
    ///
    /// `has_audio` gates everything else: `setup_audio_pipeline` only creates
    /// an audio output stream when the input actually has an audio stream
    /// (`transcode/video_transcode_phases.rs`), so a video-only source has
    /// always made the resolved encoder irrelevant. Refusing before this
    /// check (Important-4, #618 fix wave 1) was a regression — a video-only
    /// `.ivf` recode used to succeed and started hard-failing.
    fn resolve_audio_params(
        recode_audio: &RecodeAudioMode,
        target: ContainerFormat,
        has_audio: bool,
    ) -> Result<(bool, Option<String>), PostProcessError> {
        if !has_audio {
            return Ok((true, None));
        }

        let target_ext = target.as_ext();
        match recode_audio {
            RecodeAudioMode::Copy => Ok((true, None)),
            RecodeAudioMode::Auto => {
                // `None` does not strictly mean "this container cannot carry
                // audio" — see `muxer_defaults::declared_codec`'s doc for the
                // other two causes (no muxer claims the extension; ABI skew
                // between the muxer and libavcodec tables). Refuse truthfully
                // rather than guessing which cause applies, and rather than
                // silently dropping the audio track or defaulting to AAC
                // (FFmpeg itself would refuse with a cryptic "Invalid
                // argument" if we didn't).
                let Some(encoder) =
                    audio_encoder_registry::select_audio_encoder_for_container(target)
                else {
                    return Err(PostProcessError::UnsupportedFormat {
                        format: target_ext.to_string(),
                        operation: format!(
                            "no audio encoder could be determined for container \
                             '{target_ext}'; use --recode-audio=copy with a \
                             video-only source, or choose a different container"
                        ),
                    });
                };
                debug!("RecodeStage: auto audio encoder for {target_ext}: {encoder}");
                Ok((false, Some(encoder.to_string())))
            }
            RecodeAudioMode::Encoder { name } => {
                if !audio_encoder_registry::container_supports_audio_codec(target, name) {
                    warn!(
                        "RecodeStage: audio codec '{name}' may not be compatible with \
                         container '{target_ext}'; proceeding anyway"
                    );
                }
                let resolved = audio_encoder_registry::resolve_audio_encoder(name).or_else(|| {
                    warn!(
                        "RecodeStage: audio encoder '{name}' not available; \
                         falling back to container default"
                    );
                    audio_encoder_registry::select_audio_encoder_for_container(target)
                });
                let Some(resolved) = resolved else {
                    return Err(PostProcessError::UnsupportedFormat {
                        format: target_ext.to_string(),
                        operation: format!(
                            "audio encoder '{name}' is unavailable and no audio \
                             encoder could be determined for container '{target_ext}'"
                        ),
                    });
                };
                debug!("RecodeStage: using audio encoder: {resolved}");
                Ok((false, Some(resolved.to_string())))
            }
        }
    }

    fn default_preset_crf(encoder: &str) -> (Option<String>, Option<u32>) {
        // Explicit per-encoder match (not substring): x265's default crf is 28,
        // not x264's 23; and libopenh264/kvazaar (substring "264"/"kvazaar") are
        // NOT x264-style-CRF encoders, so they must NOT inherit crf 23.
        match encoder {
            "libx264" | "libx264rgb" => (Some("medium".to_string()), Some(23)),
            "libx265" => (Some("medium".to_string()), Some(28)),
            "libvpx-vp9" | "libvpx" => (None, Some(30)),
            "libsvtav1" | "libaom-av1" | "librav1e" => (None, Some(28)),
            // libvvenc/libxeve/libxavs2/liboapv/libkvazaar/libopenh264/… defer to
            // the encoder's own rate control.
            _ => (None, None),
        }
    }
}

#[async_trait]
impl PipelineStage for RecodeStage {
    fn name(&self) -> &'static str {
        "RecodeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.recode_video.is_some() || msg.config.recode_container.is_some()
    }

    #[allow(clippy::too_many_lines)]
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

        // Derive audio_copy / audio_codec from the resolved mode. Remux
        // always copies audio (no re-encoding needed); otherwise resolution
        // depends on whether the input has an audio stream at all (Important-4).
        let (audio_copy, audio_codec) = if can_remux {
            (true, None)
        } else {
            Self::resolve_audio_params(&recode_audio, target, media_info.audio_codec.is_some())?
        };

        let output_path = msg.tracker.temp_path(&input_file, target_ext);

        // Refuse a container-incompatible encoder up front with a TRUTHFUL error
        // (distinct from the generic "encoder not available" below) — e.g. AVS2
        // into MP4. Only relevant when actually transcoding (not remuxing).
        if !can_remux
            && let Some(req) = msg.config.video_encoder.as_deref()
            && let Some(enc) = video_codecs::resolve_encoder(req)
            && !Self::encoder_container_compatible(enc, target)
        {
            return Err(PostProcessError::UnsupportedFormat {
                format: enc.to_string(),
                operation: format!("{enc} only muxes into MKV; requested container {target:?}"),
            }
            .into());
        }

        let Some(mut opts) = Self::build_convert_options(
            &RecodeParams {
                target,
                encoder_override: msg.config.video_encoder.clone(),
                audio_copy,
                audio_codec,
                threads: msg.config.recode_threads,
                preset_override: msg.config.recode_preset.clone(),
                deadline: msg.config.recode_deadline.map(|d| d.as_str().to_string()),
                cpu_used: msg.config.recode_cpu_used,
                speed_level: msg.config.recode_speed_level,
            },
            can_remux,
        ) else {
            let requested = msg.config.video_encoder.as_deref().unwrap_or("");
            return Err(PostProcessError::UnsupportedFormat {
                format: requested.to_string(),
                operation: format!(
                    "video encoder '{requested}' is not available in this FFmpeg build"
                ),
            }
            .into());
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
            let crf_info = opts
                .crf
                .map_or_else(|| "default".to_string(), |c| c.to_string());
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
                .clone()
                .map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
                    Arc::new(move |frac| cb.on_progress(rdlp_types::Progress::from_f64(frac)))
                });

        let log_callback: Option<Arc<dyn Fn(&str) + Send + Sync>> = if opts.verbose {
            stage_callback
                .clone()
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
                Some(msg.cancel.clone()),
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

    #[test]
    fn hevc_default_crf_is_28_not_23() {
        assert_eq!(
            RecodeStage::default_preset_crf_for_codec("h265"),
            (Some("medium".into()), Some(28))
        );
        assert_eq!(
            RecodeStage::default_preset_crf_for_codec("hevc"),
            (Some("medium".into()), Some(28))
        );
        assert_eq!(
            RecodeStage::default_preset_crf("libx265"),
            (Some("medium".into()), Some(28))
        );
    }

    #[test]
    fn h264_default_crf_stays_23() {
        assert_eq!(
            RecodeStage::default_preset_crf_for_codec("h264"),
            (Some("medium".into()), Some(23))
        );
        assert_eq!(
            RecodeStage::default_preset_crf("libx264"),
            (Some("medium".into()), Some(23))
        );
    }

    /// #618 fix-wave-1, Important-5: exercises `resolve_audio_params` itself
    /// (real `rdlp-postprocess` code), not just the registry it wraps.
    #[test]
    fn resolve_audio_params_auto_ivf_with_audio_is_refused_truthfully() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let err =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Auto, ContainerFormat::Ivf, true)
                .expect_err("ivf has no audio encoder to resolve");
        let msg = err.to_string();
        assert!(
            msg.contains("no audio encoder could be determined"),
            "error should name the real cause, got: {msg}"
        );
    }

    #[test]
    fn resolve_audio_params_auto_mp4_with_audio_resolves_aac_family() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let (audio_copy, audio_codec) =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Auto, ContainerFormat::Mp4, true)
                .expect("mp4 always resolves an aac-family encoder");
        assert!(!audio_copy);
        let codec = audio_codec.expect("mp4 auto mode must pick an encoder");
        assert!(
            codec == "aac" || codec == "libfdk_aac",
            "expected an aac-family encoder for mp4, got {codec}"
        );
    }

    #[test]
    fn resolve_audio_params_bogus_encoder_on_ivf_with_audio_is_refused() {
        rdlp_ffmpeg::ffmpeg::ensure_init().expect("ffmpeg init");
        let mode = RecodeAudioMode::Encoder {
            name: "definitely_not_a_real_encoder_xyz".to_string(),
        };
        let err = RecodeStage::resolve_audio_params(&mode, ContainerFormat::Ivf, true)
            .expect_err("bogus encoder + no container default on ivf must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("unavailable") && msg.contains("no audio"),
            "error should explain both causes, got: {msg}"
        );
    }

    /// #618 fix-wave-1, Important-4 regression guard: a video-only source
    /// recoding to a container with no resolvable audio default (e.g.
    /// `.ivf`) must NOT be refused — `setup_audio_pipeline` only creates an
    /// audio stream when the input actually has one
    /// (`transcode/video_transcode_phases.rs`), so the chosen/default
    /// encoder was always irrelevant for video-only sources.
    #[test]
    fn resolve_audio_params_auto_ivf_without_audio_succeeds_as_copy() {
        let result =
            RecodeStage::resolve_audio_params(&RecodeAudioMode::Auto, ContainerFormat::Ivf, false);
        assert_eq!(result.unwrap(), (true, None));
    }

    #[test]
    fn new_codecs_get_no_forced_crf() {
        assert_eq!(RecodeStage::default_preset_crf("libvvenc"), (None, None));
        assert_eq!(RecodeStage::default_preset_crf("libxeve"), (None, None));
        assert_eq!(RecodeStage::default_preset_crf("libxavs2"), (None, None));
        assert_eq!(
            RecodeStage::default_preset_crf_for_codec("vvc"),
            (None, None)
        );
        assert_eq!(
            RecodeStage::default_preset_crf_for_codec("h266"),
            (None, None)
        );
    }

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
            cancel: tokio_util::sync::CancellationToken::new(),
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
    fn build_convert_options_uses_recode_params_remux() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, true).unwrap();
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
    }

    #[test]
    fn avs2_only_muxes_into_mkv() {
        // AVS2 (libxavs2) recode is refused for non-MKV containers (the caller
        // turns this into a truthful "only muxes into MKV" error). Pure helper,
        // no FFmpeg needed.
        assert!(!RecodeStage::encoder_container_compatible(
            "libxavs2",
            ContainerFormat::Mp4
        ));
        assert!(!RecodeStage::encoder_container_compatible(
            "libxavs2",
            ContainerFormat::WebM
        ));
        assert!(RecodeStage::encoder_container_compatible(
            "libxavs2",
            ContainerFormat::Mkv
        ));
        // Other encoders are unaffected by the AVS2-specific guard.
        assert!(RecodeStage::encoder_container_compatible(
            "libx265",
            ContainerFormat::Mp4
        ));
        assert!(RecodeStage::encoder_container_compatible(
            "libvvenc",
            ContainerFormat::Mp4
        ));
    }

    #[test]
    fn build_convert_options_uses_recode_params_transcode() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: false,
            audio_codec: Some("libopus".to_string()),
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).unwrap();
        assert!(!opts.audio_copy);
        assert_eq!(opts.audio_codec, Some("libopus".to_string()));
    }

    #[test]
    fn build_convert_options_remux() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, true).unwrap();
        assert!(opts.remux_only);
        assert!(opts.audio_copy);
    }

    #[test]
    fn build_convert_options_transcode_mp4() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).unwrap();
        assert!(!opts.remux_only);
        assert!(opts.video_codec.is_some());
        assert_eq!(opts.preset, Some("medium".to_string()));
        assert_eq!(opts.crf, Some(23));
        assert!(opts.audio_copy);
        assert!(opts.audio_codec.is_none());
    }

    #[test]
    fn build_convert_options_with_audio_codec() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: None,
            audio_copy: false,
            audio_codec: Some("libopus".to_string()),
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).unwrap();
        assert!(!opts.audio_copy);
        assert_eq!(opts.audio_codec, Some("libopus".to_string()));
    }

    #[test]
    fn build_convert_options_unavailable_encoder_returns_none() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: Some("nonexistent_enc_xyz".to_string()),
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let result = RecodeStage::build_convert_options(&params, false);
        assert!(result.is_none());
    }

    #[test]
    fn threads_and_preset_override_flow_into_options() {
        let params = RecodeParams {
            target: ContainerFormat::Mkv,
            encoder_override: Some("libx265".to_string()),
            audio_copy: true,
            audio_codec: None,
            threads: Some(6),
            preset_override: Some("faster".to_string()),
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).expect("encoder available");
        assert_eq!(opts.threads, Some(6));
        assert_eq!(opts.preset.as_deref(), Some("faster"));
    }

    #[test]
    fn preset_none_keeps_per_codec_default() {
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: Some("libx265".to_string()),
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: None,
            cpu_used: None,
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).expect("encoder available");
        assert_eq!(opts.preset.as_deref(), Some("medium"));
        assert_eq!(opts.threads, None);
    }

    #[test]
    fn deadline_and_cpu_used_thread_into_convert_options() {
        // deadline/cpu-used aren't meaningful for libx264, but build_convert_options
        // threads them regardless — that's what this test verifies.
        let params = RecodeParams {
            target: ContainerFormat::Mp4,
            encoder_override: Some("libx264".to_string()),
            audio_copy: true,
            audio_codec: None,
            threads: None,
            preset_override: None,
            deadline: Some("good".to_string()),
            cpu_used: Some(2),
            speed_level: None,
        };
        let opts = RecodeStage::build_convert_options(&params, false).expect("libx264 available");
        assert_eq!(opts.deadline.as_deref(), Some("good"));
        assert_eq!(opts.cpu_used, Some(2));
        assert_eq!(opts.speed_level, None);
    }
}
