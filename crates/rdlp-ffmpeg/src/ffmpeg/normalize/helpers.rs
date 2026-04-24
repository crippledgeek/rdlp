//! Helper functions for audio normalization.
//!
//! Filter builders, JSON parsing, codec/extension maps, and the
//! three-tier mux retry wrapper (library encode → salvage → resilient).

use std::path::Path;

use log::{debug, warn};

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

    debug!(
        "Loudnorm analysis: desired_gain={:.1} dB, predicted_linear_gain={:.1} dB, \
         shortfall={:.1} LU",
        opts.target_i - measurements.input_i,
        predicted_gain,
        shortfall,
    );

    let linear_mode = if opts.force_dynamic {
        debug!("Strategy: dynamic (forced via --loudnorm-dynamic)");
        "false"
    } else {
        debug!(
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
        debug!("Precompress enabled: prepending acompressor (threshold=-18dB, ratio=3:1)");
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
pub(crate) fn build_audio_filter_with_spec(
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
///
/// Handles both video containers and audio-only output formats (e.g., mp3, flac, wav).
/// For recognized audio-only extensions, returns the canonical encoder directly.
/// For video containers, delegates to [`audio_encoder_registry::select_audio_encoder_for_container`].
/// Falls back to the best available AAC encoder for unknown extensions.
pub(super) fn select_audio_encoder_for_container(ext: &str) -> &'static str {
    // Audio-only output formats: these are not ContainerFormat variants but
    // appear as extensions when normalizing standalone audio files.
    if ext.eq_ignore_ascii_case("mp3") {
        return "libmp3lame";
    }
    if ext.eq_ignore_ascii_case("flac") {
        return "flac";
    }
    if ext.eq_ignore_ascii_case("wav") {
        return "pcm_s16le";
    }

    // For proper container formats, delegate to the registry
    if let Ok(container) = ext.parse::<rdlp_types::ContainerFormat>() {
        crate::ffmpeg::audio_encoder_registry::select_audio_encoder_for_container(container)
    } else {
        crate::ffmpeg::audio_codecs::preferred_aac_encoder()
    }
}

/// Get a sensible default bitrate (in bps) for an encoder.
pub(super) fn default_bitrate_for_encoder(encoder: &str) -> usize {
    match encoder {
        "aac" | "libfdk_aac" => 128_000,
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
    if ext.eq_ignore_ascii_case("mp4")
        || ext.eq_ignore_ascii_case("m4a")
        || ext.eq_ignore_ascii_case("mov")
        || ext.eq_ignore_ascii_case("f4v")
        || ext.eq_ignore_ascii_case("3gp")
        || ext.eq_ignore_ascii_case("ts")
        || ext.eq_ignore_ascii_case("mpg")
        || ext.eq_ignore_ascii_case("flv")
        || ext.eq_ignore_ascii_case("mkv")
        || ext.eq_ignore_ascii_case("mka")
        || ext.eq_ignore_ascii_case("webm")
    {
        "mka"
    } else if ext.eq_ignore_ascii_case("avi") || ext.eq_ignore_ascii_case("mp3") {
        "mp3"
    } else if ext.eq_ignore_ascii_case("ogg") || ext.eq_ignore_ascii_case("opus") {
        "opus"
    } else if ext.eq_ignore_ascii_case("flac") {
        "flac"
    } else if ext.eq_ignore_ascii_case("wav") {
        "wav"
    } else {
        "mka"
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
    let keep_salvage = std::env::var("RDLP_KEEP_SALVAGE").is_ok_and(|v| v == "1");

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
    // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
    #[allow(clippy::disallowed_methods)]
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
                    debug!(
                        "RDLP_KEEP_SALVAGE=1: keeping salvage temp {}",
                        salvaged.display()
                    );
                } else {
                    // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
                    #[allow(clippy::disallowed_methods)]
                    let _ = std::fs::remove_file(&salvaged);
                }
                return result;
            }
            warn!(
                "Salvage retry also failed, trying resilient open: {}",
                result.as_ref().unwrap_err()
            );
            if !keep_salvage {
                debug!(
                    "Keeping salvage temp for post-mortem: {}",
                    salvaged.display()
                );
            }
            // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
            #[allow(clippy::disallowed_methods)]
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
    debug!("Attempting resilient encode (discardcorrupt+genpts)...");
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
