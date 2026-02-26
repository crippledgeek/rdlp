//! Audio normalization via FFmpeg library bindings.
//!
//! Two modes:
//! - **Peak**: Analyze peak/RMS levels via `astats` filter frame metadata,
//!   then apply `volume` + `alimiter` filters to normalize to a target peak.
//! - **Loudnorm**: EBU R128 two-pass normalization via `loudnorm` filter.
//!   Pass 1 captures measurements from FFmpeg log output, pass 2 applies
//!   them with `linear=true` for high-quality correction.

mod analysis;
mod dispatch;
mod encode;
mod helpers;
mod io_diag;

#[cfg(test)]
mod tests;

use std::path::Path;

use log::{debug, info, warn};

use crate::error::{PostProcessError, Result};

use super::salvage::prepare_input_with_salvage;
use super::{AudioNormMode, FFmpegRunner, NormalizeOptions, PeakAnalysis};

pub(crate) use io_diag::dump_io_state;

use helpers::{ALIMITER_TP_HEADROOM_DB, LIMITER_BOOST_SHORTFALL_THRESHOLD};

impl FFmpegRunner {
    /// Normalize audio levels in a media file.
    ///
    /// Video streams are copied without re-encoding. Audio is decoded,
    /// filtered (volume/limiter or loudnorm), and re-encoded with an
    /// appropriate codec for the output container.
    ///
    /// When `opts.salvage` is true (default), corrupt Matroska/WebM containers
    /// are automatically detected and remuxed to a clean temporary file before
    /// normalization. This prevents EBML structural errors from cascading into
    /// muxer ENOMEM failures during the encode pipeline.
    pub async fn normalize_audio(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("normalize_audio", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, opts.salvage)?;

            let result = match opts.mode {
                AudioNormMode::Peak => Self::normalize_peak_sync(&effective_input, &output, &opts),
                AudioNormMode::Loudnorm => {
                    Self::normalize_loudnorm_sync(&effective_input, &output, &opts)
                }
            };

            // Clean up salvage temp file regardless of success/failure
            if let Some(ref temp) = salvage_temp {
                let _ = std::fs::remove_file(temp);
            }

            result
        })
        .await
    }

    /// Peak normalization: analyze then apply gain + limiter.
    fn normalize_peak_sync(input: &Path, output: &Path, opts: &NormalizeOptions) -> Result<()> {
        let analysis = Self::analyze_peak_sync(input, opts.target_peak_db)?;

        debug!(
            "Peak analysis: peak={:.1} dBFS, RMS={:.1} dBFS, gain={:.1} dB",
            analysis.peak_db, analysis.rms_db, analysis.gain_db
        );

        // Skip if gain adjustment is negligible
        if analysis.gain_db.abs() < 0.5 {
            debug!("Audio already near target peak, skipping normalization");
            std::fs::copy(input, output).map_err(|e| PostProcessError::IoError {
                message: format!("failed to copy file: {e}"),
                source: e,
            })?;
            return Ok(());
        }

        Self::apply_peak_gain_sync(input, output, &analysis, opts)
    }

    /// EBU R128 loudnorm two-pass normalization.
    fn normalize_loudnorm_sync(input: &Path, output: &Path, opts: &NormalizeOptions) -> Result<()> {
        info!("Loudnorm pass 1: analyzing EBU R128 levels...");
        let measurements = Self::loudnorm_pass1_sync(input, opts)?;

        debug!(
            "Loudnorm measurements: I={:.1} LUFS, TP={:.1} dBTP, LRA={:.1} LU",
            measurements.input_i, measurements.input_tp, measurements.input_lra
        );

        if measurements.input_i < -35.0 {
            warn!(
                "Very quiet source ({:.1} LUFS) — normalization will amplify noise",
                measurements.input_i,
            );
        }

        // LimiterBoost: if enabled and shortfall exceeds threshold, use fixed
        // gain + hard limiter instead of loudnorm pass 2.
        let shortfall = measurements.linear_shortfall(opts.target_i, opts.target_tp);
        if opts.boost_enabled {
            if shortfall > LIMITER_BOOST_SHORTFALL_THRESHOLD {
                debug!(
                    "LimiterBoost: shortfall={shortfall:.1} LU, \
                     gain={:.1} dB — using fixed gain + limiter",
                    opts.boost_gain_db
                );
                Self::apply_limiter_boost_sync(input, output, opts)?;
                Self::verify_loudness_sync(output, opts)?;
                return Ok(());
            }
            debug!(
                "LimiterBoost: enabled but shortfall={shortfall:.1} LU <= \
                 {LIMITER_BOOST_SHORTFALL_THRESHOLD:.1} threshold — using standard loudnorm",
            );
        }

        info!("Loudnorm pass 2: applying normalization...");
        Self::loudnorm_pass2_sync(input, output, opts, &measurements)?;

        // Verify output loudness against targets
        Self::verify_loudness_sync(output, opts)?;

        Ok(())
    }

    /// LimiterBoost fallback: fixed gain + hard limiter via `apply_peak_gain_sync`.
    ///
    /// Constructs a synthetic [`PeakAnalysis`] so that `apply_peak_gain_sync`
    /// computes `gain_db = boost_gain_db` and `linear_limit = ceiling`.
    /// The ceiling is `target_tp - headroom` to stay within true-peak budget.
    fn apply_limiter_boost_sync(
        input: &Path,
        output: &Path,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        let ceiling_db = opts.target_tp - ALIMITER_TP_HEADROOM_DB;
        let limit_linear = 10f64.powf(ceiling_db / 20.0);
        debug!(
            "LimiterBoost: gain_db={:.1}, ceiling_db={:.1} (TP={:.1} - headroom={:.1}), \
             limit_linear={:.6}",
            opts.boost_gain_db, ceiling_db, opts.target_tp, ALIMITER_TP_HEADROOM_DB, limit_linear,
        );

        let synthetic_analysis = PeakAnalysis {
            peak_db: ceiling_db - opts.boost_gain_db,
            rms_db: -99.0,
            gain_db: opts.boost_gain_db,
        };

        let mut boost_opts = opts.clone();
        boost_opts.target_peak_db = ceiling_db;

        Self::apply_peak_gain_sync(input, output, &synthetic_analysis, &boost_opts)
    }
}
