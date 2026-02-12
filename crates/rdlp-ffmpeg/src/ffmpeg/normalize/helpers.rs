//! Helper functions for audio normalization.
//!
//! Filter builders, JSON parsing, codec/extension maps, and the
//! three-tier mux retry wrapper (library encode → salvage → resilient).

use std::path::Path;

use log::{info, warn};

use crate::error::{PostProcessError, Result};

use super::super::log_capture::LogCaptureGuard;
use super::super::salvage::salvage_remux_sync;
use super::super::{FFmpegRunner, LoudnormMeasurements, NormalizeOptions};

/// Extra headroom (dB) subtracted from the alimiter ceiling to account for
/// inter-sample true peak overshoot and lossy encoder artifacts.
///
/// `alimiter` is a sample-level limiter — it clamps digital sample values but
/// EBU R128 true peak measurement uses 4× oversampling to detect inter-sample
/// peaks.  Resampling (e.g. 44.1→48 kHz) and lossy encoding (AAC, Opus) can
/// also introduce ~0.5-2 dB of peak overshoot.  1.5 dB headroom is standard
/// broadcast practice (ITU-R BS.1770-5 recommendation).
pub(super) const ALIMITER_TP_HEADROOM_DB: f64 = 1.5;

/// Minimum shortfall (LU) required to trigger limiter-boost fallback.
///
/// When `boost_enabled` is true and loudnorm pass 1 shows shortfall exceeding
/// this threshold, the normal loudnorm pass 2 is skipped in favor of a fixed
/// gain + hard limiter pass via `apply_peak_gain_sync()`.
pub(super) const LIMITER_BOOST_SHORTFALL_THRESHOLD: f64 = 6.0;

/// Minimum gain (dB) that triggers 4x oversampled limiting.
///
/// When peak-normalize gain >= this threshold, the filter chain upsamples to
/// 4x the encoder sample rate before `alimiter`, then downsamples back.
/// This simulates a true-peak limiter — `alimiter` is sample-level only, so
/// heavy gain + hard limiting creates near-square waveforms whose inter-sample
/// true peaks (measured at 4x by EBU R128) significantly exceed the sample
/// ceiling.  Below this threshold, inter-sample overshoot is negligible.
pub(super) const TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD: f64 = 6.0;

/// Build the alimiter filter spec with true-peak headroom.
///
/// Ceiling is `10^((target_tp - headroom) / 20)` in linear scale.
pub(super) fn build_alimiter_spec(target_tp: f64) -> String {
    let ceiling = 10f64.powf((target_tp - ALIMITER_TP_HEADROOM_DB) / 20.0);
    format!("alimiter=limit={ceiling:.6}:attack=5:release=50")
}

/// Build the loudnorm pass 2 core filter string (without alimiter).
///
/// Returns the loudnorm filter (optionally preceded by acompressor when
/// `opts.precompress` is true).  The caller is responsible for appending
/// the alimiter via [`build_alimiter_spec`] at the correct position in
/// the filter chain (after `aresample` so resampling overshoot is caught).
///
/// Default strategy: always `linear=true`.  FFmpeg's loudnorm with
/// `linear=true` falls back to dynamic internally when conditions aren't
/// met, so forcing `linear=false` is unnecessary and often produces worse
/// perceived loudness due to over-compression.
///
/// When `opts.force_dynamic` is true, uses `linear=false` instead.
pub(super) fn build_loudnorm_pass2_filter(
    opts: &NormalizeOptions,
    measurements: &LoudnormMeasurements,
) -> String {
    let shortfall = measurements.linear_shortfall(opts.target_i, opts.target_tp);
    let predicted_gain = measurements.predict_linear_gain(opts.target_i, opts.target_tp);

    info!(
        "Loudnorm analysis: desired_gain={:.1} dB, predicted_linear_gain={:.1} dB, \
         shortfall={:.1} LU",
        opts.target_i - measurements.input_i,
        predicted_gain,
        shortfall,
    );

    let linear_mode = if opts.force_dynamic {
        info!("Strategy: dynamic (forced via --loudnorm-dynamic)");
        "false"
    } else {
        info!(
            "Strategy: linear (shortfall={shortfall:.1} LU, \
             loudnorm handles internal fallback to dynamic if needed)"
        );
        "true"
    };

    let m = measurements;
    let loudnorm = format!(
        "loudnorm=I={:.1}:TP={:.1}:LRA={:.1}:measured_I={:.2}:measured_TP={:.2}:\
         measured_LRA={:.2}:measured_thresh={:.2}:offset={:.2}:linear={linear_mode}:\
         print_format=summary",
        opts.target_i,
        opts.target_tp,
        opts.target_lra,
        m.input_i,
        m.input_tp,
        m.input_lra,
        m.input_thresh,
        m.target_offset,
    );

    if opts.precompress {
        info!("Precompress enabled: prepending acompressor (threshold=-18dB, ratio=3:1)");
        format!(
            "acompressor=threshold=0.125893:ratio=3:attack=20:release=200:makeup=2:knee=6,\
             {loudnorm}"
        )
    } else {
        loudnorm
    }
}

/// Build an audio filter graph with a custom filter spec string.
///
/// Creates: `abuffer → {filter_spec} → abuffersink`
pub(super) fn build_audio_filter_with_spec(
    decoder: &ffmpeg_the_third::decoder::Audio,
    ist_time_base: ffmpeg_the_third::Rational,
    filter_spec: &str,
) -> Result<ffmpeg_the_third::filter::Graph> {
    let mut graph = ffmpeg_the_third::filter::Graph::new();

    let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
        .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

    FFmpegRunner::add_abuffer_to_graph(
        &mut graph,
        "in",
        ist_time_base,
        decoder.rate(),
        decoder.format().name(),
        &decoder.ch_layout().description(),
    )?;
    graph
        .add(&abuffersink, "out", "")
        .map_err(|e| PostProcessError::FFmpegLibraryError {
            message: format!("failed to add abuffersink filter: {e}"),
        })?;

    FFmpegRunner::parse_and_validate_filter_graph(&mut graph, "in", "out", filter_spec)?;

    Ok(graph)
}

/// Parse loudnorm JSON output from captured FFmpeg log lines.
///
/// Looks for lines containing `"input_i"`, `"input_tp"`, etc. and extracts
/// the values from the JSON block emitted by `loudnorm print_format=json`.
pub(super) fn parse_loudnorm_json(lines: &[String]) -> Result<LoudnormMeasurements> {
    let full_text = lines.join("");

    let input_i = extract_json_value(&full_text, "input_i").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_i' in loudnorm output".into(),
        }
    })?;
    let input_tp = extract_json_value(&full_text, "input_tp").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_tp' in loudnorm output".into(),
        }
    })?;
    let input_lra = extract_json_value(&full_text, "input_lra").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_lra' in loudnorm output".into(),
        }
    })?;
    let input_thresh = extract_json_value(&full_text, "input_thresh").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'input_thresh' in loudnorm output".into(),
        }
    })?;
    let target_offset = extract_json_value(&full_text, "target_offset").ok_or_else(|| {
        PostProcessError::NormalizationFailed {
            message: "missing 'target_offset' in loudnorm output".into(),
        }
    })?;

    Ok(LoudnormMeasurements {
        input_i,
        input_tp,
        input_lra,
        input_thresh,
        target_offset,
    })
}

/// Extract a numeric value from loudnorm JSON output for a given key.
///
/// Handles the format: `"key" : "value"` where value may be a number string.
pub(super) fn extract_json_value(text: &str, key: &str) -> Option<f64> {
    let search = format!("\"{key}\"");
    let pos = text.find(&search)?;
    let after_key = &text[pos + search.len()..];

    let after_colon = after_key.find(':')? + 1;
    let value_start = &after_key[after_colon..];

    let quote_start = value_start.find('"')? + 1;
    let value_after_quote = &value_start[quote_start..];
    let quote_end = value_after_quote.find('"')?;
    let value_str = &value_after_quote[..quote_end];

    value_str.trim().parse::<f64>().ok()
}

/// Select the appropriate audio encoder for a container extension.
pub(super) fn select_audio_encoder_for_container(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "mp4" | "m4a" | "mov" | "f4v" | "3gp" => "aac",
        "webm" | "ogg" | "opus" => "libopus",
        "mkv" | "mka" => "libopus",
        "ts" | "mpg" => "aac",
        "avi" => "libmp3lame",
        "flv" => "aac",
        "mp3" => "libmp3lame",
        "flac" => "flac",
        "wav" => "pcm_s16le",
        _ => "aac",
    }
}

/// Get a sensible default bitrate (in bps) for an encoder.
pub(super) fn default_bitrate_for_encoder(encoder: &str) -> usize {
    match encoder {
        "aac" => 128_000,
        "libmp3lame" => 192_000,
        "libopus" => 128_000,
        "flac" | "pcm_s16le" => 0,
        _ => 128_000,
    }
}

/// Map a container extension to an audio-only container extension for temp files.
///
/// Uses MKA for all MOV-based formats to avoid the MOV muxer's ENOMEM issue.
/// The MOV muxer accumulates per-packet metadata in memory until trailer write,
/// causing allocation failures on long audio tracks. Matroska writes metadata
/// incrementally without unbounded buffering.
pub(super) fn audio_only_extension_for(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "mp4" | "m4a" | "mov" | "f4v" | "3gp" | "ts" | "mpg" | "flv" => "mka",
        "mkv" | "mka" | "webm" => "mka",
        "avi" | "mp3" => "mp3",
        "ogg" | "opus" => "opus",
        "flac" => "flac",
        "wav" => "wav",
        _ => "mka",
    }
}

/// Three-tier recovery for mux failures during audio normalization.
///
/// - Tier 1: Library encode (normal input open).
/// - Tier 2: Salvage-remux input → retry library encode (one attempt only).
/// - Tier 3: Library encode with resilient input (discardcorrupt+genpts flags).
/// - Never overwrites the original input.
/// - Salvage temp is deleted on success unless `RDLP_KEEP_SALVAGE=1`.
/// - Salvage temp is kept on failure for post-mortem analysis.
///
/// Loudnorm pass 1 measurements remain valid after salvage/resilient retry
/// because salvage uses stream copy (audio bit-identical) and resilient
/// mode only affects demuxer behavior (discard corrupt packets, regenerate
/// timestamps), not the audio content of valid packets.
pub(super) fn with_mux_retry<F>(input: &Path, output: &Path, encode_fn: F) -> Result<()>
where
    F: Fn(&Path, bool) -> Result<()>,
{
    let keep_salvage = std::env::var("RDLP_KEEP_SALVAGE")
        .map(|v| v == "1")
        .unwrap_or(false);

    // Tier 1: library encode (normal input open)
    match encode_fn(input, false) {
        Ok(()) => return Ok(()),
        Err(e) if !e.is_salvage_retryable() => {
            if matches!(&e, PostProcessError::NormalizationFailed { message } if message.contains("all") && message.contains("packets"))
            {
                warn!("Decoder failed on all packets — skipping salvage/resilient retry");
            }
            return Err(e);
        }
        Err(e) => {
            warn!("Encode failed with mux error, attempting one-shot salvage retry: {e}");
        }
    }

    // Clean up potentially corrupt partial output before retry
    let _ = std::fs::remove_file(output);
    if output.exists() {
        warn!(
            "Cannot remove partial output {}; skipping salvage, trying resilient open",
            output.display()
        );
        return encode_fn(input, true);
    }

    // Tier 2: salvage remux → retry library encode (ONE attempt only)
    match salvage_remux_sync(input) {
        Ok(salvaged) => {
            let result = encode_fn(&salvaged, false);
            if result.is_ok() {
                if keep_salvage {
                    info!(
                        "RDLP_KEEP_SALVAGE=1: keeping salvage temp {}",
                        salvaged.display()
                    );
                } else {
                    let _ = std::fs::remove_file(&salvaged);
                }
                return result;
            }
            warn!(
                "Salvage retry also failed, trying resilient open: {}",
                result.as_ref().unwrap_err()
            );
            if !keep_salvage {
                info!(
                    "Keeping salvage temp for post-mortem: {}",
                    salvaged.display()
                );
            }
            let _ = std::fs::remove_file(output);
            if output.exists() {
                warn!(
                    "Cannot remove failed retry output {}; trying resilient open",
                    output.display()
                );
            }
        }
        Err(e) => {
            warn!("Salvage remux failed, trying resilient open: {e}");
        }
    }

    // Tier 3: library encode with resilient input (discardcorrupt+genpts)
    info!("Attempting resilient encode (discardcorrupt+genpts)...");
    encode_fn(input, true)
}

/// Capture loudnorm pass 1 JSON from FFmpeg log output.
///
/// Sets up a [`LogCaptureGuard`], returns it for the caller to hold during
/// the analysis decode loop. After the loop finishes and the filter graph
/// is dropped, the caller retrieves captured lines and calls
/// [`parse_loudnorm_json`].
pub(super) fn begin_loudnorm_capture() -> Result<LogCaptureGuard> {
    LogCaptureGuard::begin()
}
