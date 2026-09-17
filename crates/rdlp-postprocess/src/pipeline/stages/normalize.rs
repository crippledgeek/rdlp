//! `NormalizeStage` — normalizes audio levels in media files.
//!
//! This stage runs at index 2 when `config.normalize_audio` is true.
//! Supports peak mode and EBU R128 loudnorm two-pass mode.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use log::{debug, info};

use rdlp_ffmpeg::{AudioNormMode, FFmpegRunner, NormalizeOptions, PostProcessError};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Normalizes audio in the primary current file.
///
/// `should_run` triggers when `config.normalize_audio` is true.
pub struct NormalizeStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl NormalizeStage {
    /// Create a new `NormalizeStage`.
    #[must_use]
    pub const fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build normalization options from `PostProcess`.
    ///
    /// Every default comes through `PostProcess::effective_normalize()`, the
    /// single resolution step — this function holds no `unwrap_or(<literal>)`
    /// of its own (#611).
    pub(crate) fn build_options(config: &rdlp_types::PostProcess) -> NormalizeOptions {
        let mode = if config.loudnorm {
            AudioNormMode::Loudnorm
        } else {
            AudioNormMode::Peak
        };

        let effective = config.effective_normalize();

        NormalizeOptions {
            mode,
            target_peak_db: effective.peak_target_db,
            target_i: effective.targets.integrated_lufs,
            target_tp: effective.targets.true_peak_dbtp,
            target_lra: effective.targets.range_lu,
            salvage: true,
            force_dynamic: config.loudnorm_dynamic,
            precompress: config.loudnorm_precompress,
            boost_enabled: config.normalize_boost,
            boost_gain_db: effective.boost_gain_db,
        }
    }
}

#[async_trait]
impl PipelineStage for NormalizeStage {
    fn name(&self) -> &'static str {
        "NormalizeStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.normalize_audio
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();

        let media_info = self
            .ffmpeg
            .probe(&input_file)
            .await
            .context("normalize stage: failed to probe input file")?;
        if !media_info.has_audio {
            return Err(PostProcessError::NoAudioStream.into());
        }

        let opts = Self::build_options(&msg.config);
        let mode_name = match opts.mode {
            AudioNormMode::Peak => "peak",
            AudioNormMode::Loudnorm => "loudnorm (EBU R128)",
        };

        info!(
            "NormalizeStage: normalizing audio ({}) for {}",
            mode_name,
            input_file.display()
        );

        let ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_else(|| {
                debug!("NormalizeStage: no extension, defaulting to mp4");
                "mp4"
            });

        let output_path = msg.tracker.temp_path(&input_file, ext);

        let stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));
        let _log_bridge = stage_callback
            .as_ref()
            .and_then(|cb| rdlp_ffmpeg::bridge_ffmpeg_logs(cb).ok());
        let callback = stage_callback.map(|cb| -> Arc<dyn Fn(f64) + Send + Sync> {
            Arc::new(move |frac| cb.on_progress(rdlp_types::Progress::from_f64(frac)))
        });

        self.ffmpeg
            .normalize_audio(
                &input_file,
                &output_path,
                &opts,
                callback,
                Some(msg.cancel.clone()),
            )
            .await
            .context("normalize stage failed")?;

        // Capture the encoding_tool for downstream pass-through stages.
        msg.encoding_tool = Some(format!("normalize ({mode_name})"));

        info!(
            "NormalizeStage: normalization complete: {}",
            output_path.display()
        );

        msg.tracker.replace(vec![output_path]);

        Ok(msg)
    }
}

#[cfg(test)]
// float_cmp: the defaults are constants propagated unchanged from their owner,
// so exact equality is the oracle; an epsilon would accept a drifted value.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use rdlp_types::{EffectiveNormalize, InfoDict, LoudnormPreset, PostProcess};

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
    fn should_run_when_normalize_audio() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = NormalizeStage::new(ffmpeg);

        let config = PostProcess {
            normalize_audio: true,
            ..PostProcess::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = NormalizeStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcess::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = NormalizeStage::new(ffmpeg);
        assert!(stage.is_fatal());
    }

    // --- build_options: every default is read from its owner (#611) ---
    //
    // The oracles are `EffectiveNormalize::PEAK_TARGET_DB` / `BOOST_GAIN_DB`
    // and `LoudnormPreset::targets()`, not literals: a literal here would be
    // a fourth copy of the number, and it is exactly a copy going stale that
    // this stage used to carry (`unwrap_or(-1.0)`, `unwrap_or(12.0)`).

    #[test]
    fn build_options_peak_defaults() {
        let config = PostProcess {
            normalize_audio: true,
            ..PostProcess::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Peak);
        assert_eq!(opts.target_peak_db, EffectiveNormalize::PEAK_TARGET_DB);
    }

    #[test]
    fn build_options_loudnorm_default_preset() {
        let config = PostProcess {
            normalize_audio: true,
            loudnorm: true,
            ..PostProcess::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert_eq!(opts.mode, AudioNormMode::Loudnorm);
        let targets = LoudnormPreset::default().targets();
        assert_eq!(opts.target_i, targets.integrated_lufs);
        assert_eq!(opts.target_tp, targets.true_peak_dbtp);
        assert_eq!(opts.target_lra, targets.range_lu);
    }

    /// The unset targets follow the CHOSEN preset, not a fixed one.
    #[test]
    fn build_options_loudnorm_targets_follow_the_preset() {
        for preset in [
            LoudnormPreset::Broadcast,
            LoudnormPreset::Streaming,
            LoudnormPreset::Loud,
        ] {
            let config = PostProcess {
                normalize_audio: true,
                loudnorm: true,
                loudnorm_preset: Some(preset),
                ..PostProcess::default()
            };
            let opts = NormalizeStage::build_options(&config);
            let targets = preset.targets();
            assert_eq!(opts.target_i, targets.integrated_lufs, "{preset:?}");
            assert_eq!(opts.target_tp, targets.true_peak_dbtp, "{preset:?}");
            assert_eq!(opts.target_lra, targets.range_lu, "{preset:?}");
        }
    }

    #[test]
    fn build_options_individual_overrides() {
        let config = PostProcess {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some(LoudnormPreset::Broadcast),
            loudnorm_target_i: Some(-16.0),
            loudnorm_target_tp: Some(-1.5),
            ..PostProcess::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert_eq!(opts.target_i, -16.0);
        assert_eq!(opts.target_tp, -1.5);
        assert_eq!(
            opts.target_lra,
            LoudnormPreset::Broadcast.targets().range_lu,
            "LRA not overridden: stays the preset's"
        );
    }

    #[test]
    fn build_options_boost_enabled() {
        let config = PostProcess {
            normalize_audio: true,
            normalize_boost: true,
            normalize_boost_db: Some(8.0),
            ..PostProcess::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert!(opts.boost_enabled);
        assert_eq!(opts.boost_gain_db, 8.0);
    }

    #[test]
    fn build_options_boost_default_gain() {
        let config = PostProcess {
            normalize_audio: true,
            normalize_boost: true,
            normalize_boost_db: None,
            ..PostProcess::default()
        };
        let opts = NormalizeStage::build_options(&config);
        assert!(opts.boost_enabled);
        assert_eq!(opts.boost_gain_db, EffectiveNormalize::BOOST_GAIN_DB);
    }

    /// The stage's options equal the resolver's output field-for-field, so
    /// there is exactly one resolution step between config and `FFmpeg`.
    #[test]
    fn build_options_equals_effective_normalize() {
        let config = PostProcess {
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some(LoudnormPreset::Loud),
            audio_gain_target: Some(-2.5),
            ..PostProcess::default()
        };
        let opts = NormalizeStage::build_options(&config);
        let eff = config.effective_normalize();
        assert_eq!(opts.target_peak_db, eff.peak_target_db);
        assert_eq!(opts.target_i, eff.targets.integrated_lufs);
        assert_eq!(opts.target_tp, eff.targets.true_peak_dbtp);
        assert_eq!(opts.target_lra, eff.targets.range_lu);
        assert_eq!(opts.boost_gain_db, eff.boost_gain_db);
    }
}
