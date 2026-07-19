//! Dispatch logic for audio normalization encode paths.
//!
//! Decides whether to encode audio-only or merge with video, handles
//! salvage retry, and builds mode-specific filter specs before delegating
//! to [`FFmpegRunner::encode_audio_only_sync`].

use std::path::Path;

use anyhow::Context as _;
use tokio_util::sync::CancellationToken;

use crate::error::PostProcessError;

use super::super::{FFmpegRunner, LoudnormMeasurements, NormalizeOptions, PeakAnalysis};
use super::encode::EncodeCallCtx;
use super::helpers::{
    TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD, audio_only_extension_for_ext, build_alimiter_spec,
    build_loudnorm_pass2_filter, with_mux_retry,
};

impl FFmpegRunner {
    /// Apply peak gain normalization: encode audio to temp, merge with video.
    pub(super) fn apply_peak_gain_sync(
        input: &Path,
        output: &Path,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        Self::dispatch_normalize_sync(
            input,
            output,
            opts.salvage,
            progress_fn,
            cancel,
            |inp, out, ext, resilient, pfn| {
                let ctx = EncodeCallCtx {
                    progress_fn: pfn,
                    cancel,
                };
                Self::peak_encode_audio_only(inp, out, ext, analysis, opts, resilient, &ctx)
            },
        )
    }

    /// Encode peak-normalized audio to an output file (video streams discarded).
    fn peak_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
        resilient: bool,
        ctx: &EncodeCallCtx<'_>,
    ) -> anyhow::Result<()> {
        let gain_db = opts.target_peak_db - analysis.peak_db;
        let linear_limit = 10f64.powf(opts.target_peak_db / 20.0);
        Self::encode_audio_only_sync(
            input,
            output,
            final_output_ext,
            "peak encode",
            resilient,
            ctx,
            |fmt, rate, ch_layout| {
                let oversample_prefix = if gain_db >= TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD {
                    let rate_4x = rate * 4;
                    format!("aresample={rate_4x},")
                } else {
                    String::new()
                };
                format!(
                    "volume={gain_db:.6}dB,{oversample_prefix}aresample,\
                     alimiter=limit={linear_limit:.6}:attack=5:release=50,\
                     aformat=sample_fmts={fmt}:sample_rates={rate}:\
                     channel_layouts={ch_layout}",
                )
            },
        )
    }

    /// Loudnorm pass 2: apply normalization with measured values.
    pub(super) fn loudnorm_pass2_sync(
        input: &Path,
        output: &Path,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        Self::dispatch_normalize_sync(
            input,
            output,
            opts.salvage,
            progress_fn,
            cancel,
            |inp, out, ext, resilient, pfn| {
                let ctx = EncodeCallCtx {
                    progress_fn: pfn,
                    cancel,
                };
                Self::loudnorm_encode_audio_only(inp, out, ext, opts, measurements, resilient, &ctx)
            },
        )
    }

    /// Encode loudnorm-normalized audio to an output file (video streams discarded).
    fn loudnorm_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
        resilient: bool,
        ctx: &EncodeCallCtx<'_>,
    ) -> anyhow::Result<()> {
        let loudnorm_core = build_loudnorm_pass2_filter(opts, measurements);
        let limiter = build_alimiter_spec(opts.target_tp);
        Self::encode_audio_only_sync(
            input,
            output,
            final_output_ext,
            "loudnorm pass 2",
            resilient,
            ctx,
            |fmt, rate, ch_layout| {
                format!(
                    "aformat=sample_fmts=dbl,{loudnorm_core},aresample,\
                     {limiter},aformat=sample_fmts={fmt}:sample_rates={rate}:\
                     channel_layouts={ch_layout}",
                )
            },
        )
    }

    /// Common dispatch for normalize encode → merge with optional salvage retry.
    ///
    /// Both peak and loudnorm share this pattern: check `has_video` → audio-only
    /// encode to temp → merge with original video → cleanup.  When `salvage` is
    /// true, wraps the encode with `with_mux_retry` for three-tier recovery
    /// (salvage remux → resilient open) on mux write failures.
    fn dispatch_normalize_sync(
        input: &Path,
        output: &Path,
        salvage: bool,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
        encode_fn: impl Fn(
            &Path,
            &Path,
            &str,
            bool,
            Option<&(dyn Fn(f64) + Send + Sync)>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        crate::ffmpeg::ensure_init()?;

        let has_video = {
            let ictx = ffmpeg_the_third::format::input(input)
                .map_err(PostProcessError::from)
                .with_context(|| {
                    format!(
                        "failed to open input for normalize dispatch {}",
                        input.display()
                    )
                })?;
            ictx.streams()
                .best(ffmpeg_the_third::media::Type::Video)
                .is_some()
        };

        let ext = output
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_else(|| {
                log::info!("No output extension in normalize dispatch; defaulting to mp4");
                "mp4"
            });

        if has_video {
            let audio_ext = audio_only_extension_for_ext(ext);
            let temp_audio = output.with_extension(format!("norm_audio.{audio_ext}"));

            if salvage {
                with_mux_retry(input, &temp_audio, |effective_input, resilient| {
                    Ok(encode_fn(
                        effective_input,
                        &temp_audio,
                        ext,
                        resilient,
                        progress_fn,
                    )?)
                })?;
            } else {
                encode_fn(input, &temp_audio, ext, false, progress_fn)?;
            }
            let merge_result = Self::merge_sync(
                input,
                &temp_audio,
                output,
                &super::super::RemuxOptions::default(),
                progress_fn,
                // Cancel-gate the final stream-copy merge for large files (#340).
                cancel,
            );
            // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
            #[allow(clippy::disallowed_methods)]
            if let Err(e) = std::fs::remove_file(&temp_audio) {
                log::warn!(
                    "Failed to remove temp audio file {}: {e}",
                    temp_audio.display()
                );
            }
            merge_result
        } else if salvage {
            with_mux_retry(input, output, |effective_input, resilient| {
                Ok(encode_fn(
                    effective_input,
                    output,
                    ext,
                    resilient,
                    progress_fn,
                )?)
            })?;
            Ok(())
        } else {
            encode_fn(input, output, ext, false, progress_fn)
        }
    }
}
