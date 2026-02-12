//! Dispatch logic for audio normalization encode paths.
//!
//! Decides whether to encode audio-only or merge with video, handles
//! salvage retry, and builds mode-specific filter specs before delegating
//! to [`super::encode::FFmpegRunner::encode_audio_only_sync`].

use std::path::Path;

use crate::error::{PostProcessError, Result};

use super::super::{FFmpegRunner, LoudnormMeasurements, NormalizeOptions, PeakAnalysis};
use super::helpers::{
    TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD, audio_only_extension_for, build_alimiter_spec,
    build_loudnorm_pass2_filter, cli_fallback_loudnorm, cli_fallback_peak, with_mux_retry,
};

impl FFmpegRunner {
    /// Apply peak gain normalization: encode audio to temp, merge with video.
    pub(super) fn apply_peak_gain_sync(
        input: &Path,
        output: &Path,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        Self::dispatch_normalize_sync(
            input,
            output,
            opts.salvage,
            |inp, out, ext| Self::peak_encode_audio_only(inp, out, ext, analysis, opts),
            |f_in, f_out| cli_fallback_peak(f_in, f_out, analysis, opts),
        )
    }

    /// Encode peak-normalized audio to an output file (video streams discarded).
    fn peak_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        let gain_db = opts.target_peak_db - analysis.peak_db;
        let linear_limit = 10f64.powf(opts.target_peak_db / 20.0);
        Self::encode_audio_only_sync(
            input,
            output,
            final_output_ext,
            "peak encode",
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
    ) -> Result<()> {
        Self::dispatch_normalize_sync(
            input,
            output,
            opts.salvage,
            |inp, out, ext| Self::loudnorm_encode_audio_only(inp, out, ext, opts, measurements),
            |f_in, f_out| cli_fallback_loudnorm(f_in, f_out, opts, measurements),
        )
    }

    /// Encode loudnorm-normalized audio to an output file (video streams discarded).
    fn loudnorm_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
    ) -> Result<()> {
        let loudnorm_core = build_loudnorm_pass2_filter(opts, measurements);
        let limiter = build_alimiter_spec(opts.target_tp);
        Self::encode_audio_only_sync(
            input,
            output,
            final_output_ext,
            "loudnorm pass 2",
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
    /// Both peak and loudnorm share this pattern: check has_video → audio-only
    /// encode to temp → merge with original video → cleanup.  When `salvage` is
    /// true, wraps the encode with `with_mux_retry` for two-tier recovery
    /// (salvage remux → CLI fallback) on mux write failures.
    fn dispatch_normalize_sync(
        input: &Path,
        output: &Path,
        salvage: bool,
        encode_fn: impl Fn(&Path, &Path, &str) -> Result<()>,
        cli_fallback_fn: impl Fn(&Path, &Path) -> Result<()>,
    ) -> Result<()> {
        crate::ffmpeg::ensure_init()?;

        let has_video = {
            let ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open input {}: {e}", input.display()),
                }
            })?;
            ictx.streams()
                .best(ffmpeg_the_third::media::Type::Video)
                .is_some()
        };

        let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mp4");

        if has_video {
            let audio_ext = audio_only_extension_for(ext);
            let temp_audio = output.with_extension(format!("norm_audio.{audio_ext}"));

            if salvage {
                with_mux_retry(
                    input,
                    &temp_audio,
                    |effective_input| encode_fn(effective_input, &temp_audio, ext),
                    |fallback_in, fallback_out| cli_fallback_fn(fallback_in, fallback_out),
                )?;
            } else {
                encode_fn(input, &temp_audio, ext)?;
            }
            let merge_result = Self::merge_sync(
                input,
                &temp_audio,
                output,
                &super::super::RemuxOptions::default(),
            );
            let _ = std::fs::remove_file(&temp_audio);
            merge_result
        } else if salvage {
            with_mux_retry(
                input,
                output,
                |effective_input| encode_fn(effective_input, output, ext),
                |fallback_in, fallback_out| cli_fallback_fn(fallback_in, fallback_out),
            )
        } else {
            encode_fn(input, output, ext)
        }
    }
}
