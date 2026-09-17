//! Audio handling mode for video recode operations.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::media_name::{AudioCodecOrEncoderName, InvalidMediaName};

/// How to handle audio during video recode, when the operator said.
///
/// Collapses the audio copy/encoder decision into a single discriminated type,
/// eliminating the two-field interaction matrix.
///
/// There is deliberately no `Default`: the field that carries this is
/// `Option<RecodeAudioMode>`, where `None` means *not specified* — rdlp then
/// copies the audio when the target container carries its codec and re-encodes
/// otherwise. An explicit [`Copy`](Self::Copy) is a demand, and an impossible
/// demand is refused rather than quietly re-encoded (#645). A `Default` of
/// `Copy` on a plain field made the two indistinguishable.
///
/// # Examples
///
/// ```rust
/// use rdlp_types::RecodeAudioMode;
/// use serde_json;
///
/// // Serde roundtrip
/// let json = serde_json::to_string(&RecodeAudioMode::Copy).unwrap();
/// assert_eq!(json, r#"{"mode":"copy"}"#);
///
/// let encoder: RecodeAudioMode = "libopus".parse().unwrap();
/// let json = serde_json::to_string(&encoder).unwrap();
/// assert_eq!(json, r#"{"mode":"encoder","name":"libopus"}"#);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum RecodeAudioMode {
    /// Stream copy audio unchanged; refuse if the target cannot carry it.
    Copy,
    /// Auto-select best encoder for the output container.
    Auto,
    /// Use a specific codec or encoder (e.g., "`libfdk_aac`", "libopus", "aac").
    Encoder {
        /// The codec or encoder name; which one is resolved against the
        /// linked `FFmpeg` build. Kept exactly as typed — `FFmpeg` matches
        /// encoder names exactly, so `libOpus` must survive unchanged.
        name: AudioCodecOrEncoderName,
    },
}

impl FromStr for RecodeAudioMode {
    type Err = InvalidMediaName;

    /// Parses the `--recode-audio` vocabulary: `copy`, `auto`, or a codec /
    /// encoder name.
    ///
    /// The two mode keywords are matched case-insensitively, matching every
    /// other format vocabulary in this crate (which gets it from
    /// `#[strum(ascii_case_insensitive)]`). Anything else is a name, validated
    /// for shape here (#649) — whether that name exists is `FFmpeg`'s question
    /// to answer, at the boundary that can ask it.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("copy") {
            Ok(Self::Copy)
        } else if value.eq_ignore_ascii_case("auto") {
            Ok(Self::Auto)
        } else {
            Ok(Self::Encoder {
                name: AudioCodecOrEncoderName::new(value)?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoder(name: &str) -> RecodeAudioMode {
        RecodeAudioMode::Encoder {
            name: AudioCodecOrEncoderName::new(name).unwrap(),
        }
    }

    #[test]
    fn parse_maps_the_mode_keywords_case_insensitively() {
        for spelling in ["copy", "COPY", "Copy", "cOpY"] {
            assert_eq!(spelling.parse(), Ok(RecodeAudioMode::Copy));
        }
        for spelling in ["auto", "AUTO", "Auto"] {
            assert_eq!(spelling.parse(), Ok(RecodeAudioMode::Auto));
        }
    }

    #[test]
    fn parse_preserves_encoder_name_case() {
        assert_eq!(
            "libOpus".parse(),
            Ok(encoder("libOpus")),
            "FFmpeg matches encoder names exactly; case must not be folded"
        );
    }

    /// A keyword with surrounding content is an encoder name, not a mode —
    /// the match is on the whole value, never a substring.
    #[test]
    fn parse_does_not_match_keywords_as_substrings() {
        for name in ["copycat", "autotune", "libcopy", "copy2"] {
            assert_eq!(
                name.parse(),
                Ok(encoder(name)),
                "{name} must be treated as an encoder name"
            );
        }
    }

    /// #649: a malformed value fails at the boundary with the name-shape
    /// error, not deep inside `FFmpeg`.
    #[test]
    fn parse_rejects_a_malformed_name() {
        for bad in ["", " ", "lib opus", "aac;rm -rf"] {
            assert!(
                bad.parse::<RecodeAudioMode>().is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    #[test]
    fn serde_copy_roundtrip() {
        let mode = RecodeAudioMode::Copy;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#"{"mode":"copy"}"#);
        let parsed: RecodeAudioMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, parsed);
    }

    #[test]
    fn serde_auto_roundtrip() {
        let mode = RecodeAudioMode::Auto;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#"{"mode":"auto"}"#);
        let parsed: RecodeAudioMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, parsed);
    }

    /// The wire form is unchanged by typing the name (#649).
    #[test]
    fn serde_encoder_roundtrip() {
        let mode = encoder("libfdk_aac");
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#"{"mode":"encoder","name":"libfdk_aac"}"#);
        let parsed: RecodeAudioMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, parsed);
    }

    #[test]
    fn serde_encoder_rejects_a_malformed_name_on_the_wire() {
        assert!(
            serde_json::from_str::<RecodeAudioMode>(r#"{"mode":"encoder","name":""}"#).is_err()
        );
    }
}
