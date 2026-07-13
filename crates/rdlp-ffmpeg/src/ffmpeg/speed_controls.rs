//! Shared recode-encoder resolution + strict per-encoder speed-control validation.
//!
//! The validator and `RecodeStage` MUST resolve the target encoder identically;
//! both go through [`resolve_recode_encoder`] so they cannot drift.

use thiserror::Error;

use super::video_codecs::{KnobField, KnobKindDef, resolve_encoder, speed_controls_def};

/// Return the default video codec to encode toward for a given container
/// extension (e.g. `"webm"` → `"vp9"`, `"mp4"` → `"h264"`).
///
/// This is the **single source of truth** for the container → codec mapping.
/// Both the speed-control validator and `RecodeStage` delegate here so they
/// can never resolve differently.
#[must_use]
pub fn default_codec_for_container(container: &str) -> &'static str {
    match container.to_ascii_lowercase().as_str() {
        "webm" | "ivf" => "vp9",
        "ogg" => "theora",
        "mpg" | "vob" => "mpeg2",
        _ => "h264",
    }
}

/// Resolve the encoder a recode will actually use.
///
/// Precedence mirrors `RecodeStage`: explicit `video_encoder` override first,
/// else the target container's default codec. Self-initializes `FFmpeg`.
#[must_use]
pub fn resolve_recode_encoder(
    video_encoder: Option<&str>,
    recode_video: Option<&str>,
    recode_container: Option<&str>,
) -> Option<&'static str> {
    super::ensure_init().ok();
    if let Some(ve) = video_encoder.filter(|s| !s.is_empty()) {
        return resolve_encoder(ve);
    }
    let target = recode_container.or(recode_video)?;
    resolve_encoder(default_codec_for_container(target))
}

/// Strict per-encoder speed-control validation error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpeedControlError {
    /// Knob not applicable to the resolved encoder.
    #[error("{knob:?} is not applicable to encoder '{encoder}'")]
    NotApplicable {
        /// The knob that was rejected.
        knob: KnobField,
        /// The encoder name the knob was checked against.
        encoder: String,
    },
    /// Choice value not in the allowed set.
    #[error("{knob:?} value '{value}' is not one of {choices:?}")]
    NotInChoices {
        /// The knob whose value was rejected.
        knob: KnobField,
        /// The supplied value.
        value: String,
        /// The encoder's allowed choices.
        choices: Vec<String>,
    },
    /// Int knob given a non-integer value.
    #[error("{knob:?} value '{value}' is not an integer")]
    NotInteger {
        /// The knob whose value failed to parse.
        knob: KnobField,
        /// The supplied (non-integer) value.
        value: String,
    },
    /// Int value outside the encoder's range.
    #[error("{knob:?} value {value} out of range {min}..={max}")]
    OutOfRange {
        /// The knob whose value was out of range.
        knob: KnobField,
        /// The supplied value.
        value: i32,
        /// Minimum allowed value (inclusive).
        min: i32,
        /// Maximum allowed value (inclusive).
        max: i32,
    },
}

fn check_knob(
    field: KnobField,
    value: Option<&str>,
    encoder: &str,
) -> Result<(), SpeedControlError> {
    let Some(val) = value else {
        return Ok(());
    };
    let def = speed_controls_def(encoder)
        .iter()
        .find(|k| k.field == field)
        .ok_or_else(|| SpeedControlError::NotApplicable {
            knob: field,
            encoder: encoder.to_string(),
        })?;
    match def.kind {
        KnobKindDef::Choice { choices, .. } => {
            if choices.contains(&val) {
                Ok(())
            } else {
                Err(SpeedControlError::NotInChoices {
                    knob: field,
                    value: val.to_string(),
                    choices: choices.iter().map(|s| (*s).to_string()).collect(),
                })
            }
        }
        KnobKindDef::Int { min, max, .. } => {
            let n: i32 = val.parse().map_err(|_| SpeedControlError::NotInteger {
                knob: field,
                value: val.to_string(),
            })?;
            if (min..=max).contains(&n) {
                Ok(())
            } else {
                Err(SpeedControlError::OutOfRange {
                    knob: field,
                    value: n,
                    min,
                    max,
                })
            }
        }
    }
}

/// Strictly validate speed-control values against the resolved encoder's descriptor.
///
/// `None` knobs and an unresolvable encoder are accepted (an unresolvable
/// encoder means no recode runs). Self-initializes `FFmpeg`.
///
/// # Errors
///
/// Returns [`SpeedControlError`] when a knob is inapplicable to the encoder or a
/// value is outside the encoder's allowed set/range.
pub fn validate_speed_controls(
    encoder: Option<&str>,
    preset: Option<&str>,
    deadline: Option<&str>,
    cpu_used: Option<i32>,
    speed_level: Option<u32>,
) -> Result<(), SpeedControlError> {
    super::ensure_init().ok();
    let Some(enc) = encoder else {
        return Ok(());
    };
    // The numeric knobs bind their owned strings first so `.as_deref()` can
    // borrow them; `preset`/`deadline` are already `&str` and pass through with
    // no allocation on the happy path.
    let cpu = cpu_used.map(|n| n.to_string());
    let spd = speed_level.map(|n| i32::try_from(n).unwrap_or(i32::MAX).to_string());
    check_knob(KnobField::Preset, preset, enc)?;
    check_knob(KnobField::Deadline, deadline, enc)?;
    check_knob(KnobField::CpuUsed, cpu.as_deref(), enc)?;
    check_knob(KnobField::SpeedLevel, spd.as_deref(), enc)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_rejected_on_x264() {
        let err =
            validate_speed_controls(Some("libx264"), None, Some("good"), None, None).unwrap_err();
        assert!(matches!(
            err,
            SpeedControlError::NotApplicable {
                knob: KnobField::Deadline,
                ..
            }
        ));
    }

    #[test]
    fn cpu_used_range_is_per_encoder() {
        // libvpx-vp9 range is -8..=8, so 9 is out of range
        assert!(validate_speed_controls(Some("libvpx-vp9"), None, None, Some(9), None).is_err());
        // libvpx (VP8) range is -16..=16, so 9 is in range
        assert!(validate_speed_controls(Some("libvpx"), None, None, Some(9), None).is_ok());
    }

    #[test]
    fn vvenc_preset_membership() {
        assert!(
            validate_speed_controls(Some("libvvenc"), Some("medium"), None, None, None).is_ok()
        );
        assert!(
            validate_speed_controls(Some("libvvenc"), Some("ultrafast"), None, None, None).is_err()
        );
    }

    #[test]
    fn svtav1_preset_is_numeric() {
        // 8 is within -2..=13
        assert!(validate_speed_controls(Some("libsvtav1"), Some("8"), None, None, None).is_ok());
        // 14 is outside -2..=13
        assert!(validate_speed_controls(Some("libsvtav1"), Some("14"), None, None, None).is_err());
        // non-integer string fails parse
        assert!(
            validate_speed_controls(Some("libsvtav1"), Some("fast"), None, None, None).is_err()
        );
    }

    #[test]
    fn none_values_and_unresolved_encoder_pass() {
        assert!(validate_speed_controls(Some("libx264"), None, None, None, None).is_ok());
        assert!(validate_speed_controls(None, Some("anything"), None, None, None).is_ok());
    }

    #[test]
    fn resolver_prefers_explicit_encoder() {
        super::super::ensure_init().ok();
        if resolve_encoder("libx265").is_some() {
            assert_eq!(
                resolve_recode_encoder(Some("libx265"), Some("mp4"), None),
                Some("libx265")
            );
        }
    }

    #[test]
    fn resolver_matches_recode_stage_defaults() {
        assert_eq!(default_codec_for_container("webm"), "vp9");
        assert_eq!(default_codec_for_container("ivf"), "vp9");
        assert_eq!(default_codec_for_container("ogg"), "theora");
        assert_eq!(default_codec_for_container("mpg"), "mpeg2");
        assert_eq!(default_codec_for_container("vob"), "mpeg2");
        assert_eq!(default_codec_for_container("mp4"), "h264");
        assert_eq!(default_codec_for_container("mkv"), "h264");
    }
}
