//! Helper functions for audio normalization.
//!
//! Filter builders, JSON parsing, codec/extension maps, and the
//! three-tier mux retry wrapper (library encode → salvage → resilient).
//!
//! # Lint allowances
//!
//! - `clippy::redundant_pub_crate`: `pub(crate)` functions in this private submodule
//!   are accessed from `normalize/mod.rs` and `normalize/encode.rs`.
//! - `clippy::option_if_let_else`: the two branches of the `if let Ok(container)` guard
//!   call into different subsystems; collapsing into `map_or_else` reduces readability.
//! - `clippy::match_same_arms`: the `aac | libfdk_aac` and `libopus` arms are kept
//!   distinct for future per-codec bitrate tuning.
//! - `clippy::expect_used`: `unwrap_err` replaced with `expect` plus invariant comment.

#![allow(
    clippy::redundant_pub_crate,
    clippy::option_if_let_else,
    clippy::match_same_arms,
    clippy::expect_used
)]

use std::path::Path;

use log::{debug, warn};

use rdlp_types::ContainerFormat;

use crate::error::{PostProcessError, Result};

use super::super::log_capture::LogCaptureGuard;
use super::super::salvage::salvage_remux_sync;
use super::super::{LoudnormMeasurements, NormalizeOptions};

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
/// Default strategy: always `linear=true`.  `FFmpeg`'s loudnorm with
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

/// Parse loudnorm JSON output from captured `FFmpeg` log lines.
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

/// Get a sensible default bitrate (in bps) for an encoder.
///
/// The `0` arm is the lossless sentinel: `bit_rate` is meaningless for these
/// encoders (they ignore it), and `encode.rs:174` prefers a non-zero input
/// bitrate when one is known anyway. Widened alongside the audio-default
/// registry (#618) to cover every lossless encoder now reachable through
/// `select_audio_encoder_for_container` (`.aiff`/`.caf` → `pcm_s16be`, `.wv`
/// → `wavpack`), not just the two that were reachable before.
pub(super) fn default_bitrate_for_encoder(
    encoder: &rdlp_types::media_name::AudioEncoderName,
) -> usize {
    match encoder.as_str() {
        "aac" | "libfdk_aac" => 128_000,
        "libmp3lame" => 192_000,
        "libopus" => 128_000,
        "flac" | "pcm_s16le" | "pcm_s16be" | "alac" | "wavpack" | "tta" => 0,
        _ => 128_000,
    }
}

/// Audio-only container used when nothing more specific fits, and for any
/// container whose own muxer is unsuitable for a long audio-only temp file.
const DEFAULT_AUDIO_ONLY: ContainerFormat = ContainerFormat::Mka;

/// Map a container to the audio-only container used for its temp files.
///
/// Uses MKA for all MOV-based formats to avoid the MOV muxer's ENOMEM issue.
/// The MOV muxer accumulates per-packet metadata in memory until trailer write,
/// causing allocation failures on long audio tracks. Matroska writes metadata
/// incrementally without unbounded buffering.
///
/// Takes a `ContainerFormat` and matches exhaustively rather than testing a
/// hand-written list of extension strings. The list previously omitted `m4v` —
/// harmless only because the fall-through default happened to return the same
/// answer the MOV-family arm would have. That is the same silent drift as #539,
/// one edit to the default away from becoming a real bug. With an exhaustive
/// match, a new `ContainerFormat` variant fails to compile until someone
/// classifies it.
///
/// Every mapping below preserves the previous behaviour exactly, including the
/// containers that reached `"mka"` via the old default.
pub(super) const fn audio_only_extension_for(container: ContainerFormat) -> &'static str {
    match container {
        // MOV/ISOBMFF family — routed to MKA for the ENOMEM reason above.
        ContainerFormat::Mp4
        | ContainerFormat::Mov
        | ContainerFormat::M4v
        | ContainerFormat::F4v
        | ContainerFormat::ThreeGp
        | ContainerFormat::M4a
        // Other video containers whose muxers we likewise avoid for audio-only.
        | ContainerFormat::Ts
        | ContainerFormat::Mpg
        | ContainerFormat::Flv
        | ContainerFormat::Mkv
        | ContainerFormat::Mka
        | ContainerFormat::WebM
        // Reached the old `else` default; kept on MKA deliberately.
        // The ASF family stays on MKA for its audio-only temp files: all three
        // parsed to one variant before #538 and mapped here, so listing them
        // together preserves that behaviour exactly.
        | ContainerFormat::Wmv
        | ContainerFormat::Wma
        | ContainerFormat::Asf
        | ContainerFormat::Mxf
        | ContainerFormat::Vob
        | ContainerFormat::Dv
        | ContainerFormat::Nut
        | ContainerFormat::Ivf
        | ContainerFormat::Aac
        | ContainerFormat::Aiff
        | ContainerFormat::Wv
        | ContainerFormat::Caf
        | ContainerFormat::Ac3 => DEFAULT_AUDIO_ONLY.as_ext(),
        ContainerFormat::Avi | ContainerFormat::Mp3 => ContainerFormat::Mp3.as_ext(),
        ContainerFormat::Ogg | ContainerFormat::Opus => ContainerFormat::Opus.as_ext(),
        ContainerFormat::Flac => ContainerFormat::Flac.as_ext(),
        ContainerFormat::Wav => ContainerFormat::Wav.as_ext(),
    }
}

/// Boundary form of [`audio_only_extension_for`] for callers holding only a path
/// extension. An unrecognised extension keeps the previous default.
pub(super) fn audio_only_extension_for_ext(ext: &str) -> &'static str {
    ext.parse::<ContainerFormat>()
        .map_or(DEFAULT_AUDIO_ONLY.as_ext(), audio_only_extension_for)
}

/// Resolves the audio encoder for a normalize output extension.
///
/// `final_output_ext` is the USER's output path extension, so two distinct
/// failure modes must stay distinct:
///
/// - The extension doesn't parse into a [`ContainerFormat`] at all. This
///   degrades gracefully to AAC, exactly like the sibling
///   [`audio_only_extension_for_ext`] — normalization is about container
///   *defaults* (#618), not about rejecting extensions it doesn't recognise,
///   and the prior behaviour (before #618) was this same AAC fallback.
/// - The extension parses but the registry can't determine an encoder for
///   it (container genuinely carries no audio, or the muxer/codec tables
///   yielded nothing usable — see
///   [`super::super::audio_encoder_registry::select_audio_encoder_for_container`]).
///   This is a real refusal and is reported truthfully: the extension goes
///   in `format`, never mislabelled as a codec name.
pub(super) fn resolve_normalize_audio_encoder(
    final_output_ext: &str,
    label: &str,
) -> Result<rdlp_types::media_name::AudioEncoderName> {
    if let Ok(container) = final_output_ext.parse::<ContainerFormat>() {
        super::super::audio_encoder_registry::select_audio_encoder_for_container(container)
            .ok_or_else(|| PostProcessError::UnsupportedFormat {
                format: final_output_ext.to_string(),
                operation: format!(
                    "audio normalization ({label}): no audio encoder could be \
                     determined for container '{final_output_ext}'"
                ),
            })
    } else {
        warn!(
            "audio normalization ({label}): unrecognized output extension \
             '{final_output_ext}'; falling back to AAC"
        );
        super::super::audio_encoder_registry::preferred_audio_encoder("aac").ok_or_else(|| {
            PostProcessError::UnsupportedCodec {
                codec: "aac".to_string(),
                operation: format!("audio normalization ({label})"),
            }
        })
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
            // is_ok() returned false above — expect_err is safe here.
            warn!(
                "Salvage retry also failed, trying resilient open: {}",
                result
                    .as_ref()
                    .expect_err("result is Err (is_ok checked above)")
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

/// Capture loudnorm pass 1 JSON from `FFmpeg` log output.
///
/// Sets up a [`LogCaptureGuard`], returns it for the caller to hold during
/// the analysis decode loop. After the loop finishes and the filter graph
/// is dropped, the caller retrieves captured lines and calls
/// [`parse_loudnorm_json`].
pub(super) fn begin_loudnorm_capture() -> Result<LogCaptureGuard> {
    LogCaptureGuard::begin()
}
