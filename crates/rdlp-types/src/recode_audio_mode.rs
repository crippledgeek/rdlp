//! Audio handling mode for video recode operations.

use serde::{Deserialize, Serialize};

/// How to handle audio during video recode.
///
/// Collapses the audio copy/encoder decision into a single discriminated type,
/// eliminating the two-field interaction matrix.
///
/// # Examples
///
/// ```rust
/// use rdlp_types::RecodeAudioMode;
/// use serde_json;
///
/// // Default is Copy
/// let mode = RecodeAudioMode::default();
/// assert_eq!(mode, RecodeAudioMode::Copy);
///
/// // Serde roundtrip
/// let json = serde_json::to_string(&RecodeAudioMode::Copy).unwrap();
/// assert_eq!(json, r#"{"mode":"copy"}"#);
///
/// let encoder = RecodeAudioMode::Encoder { name: "libopus".to_string() };
/// let json = serde_json::to_string(&encoder).unwrap();
/// assert_eq!(json, r#"{"mode":"encoder","name":"libopus"}"#);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum RecodeAudioMode {
    /// Stream copy audio unchanged (default).
    #[default]
    Copy,
    /// Auto-select best encoder for the output container.
    Auto,
    /// Use a specific encoder (e.g., "`libfdk_aac`", "libopus").
    Encoder {
        /// `FFmpeg` encoder name (e.g., "`libfdk_aac`", "libopus").
        name: String,
    },
}

impl From<&str> for RecodeAudioMode {
    /// Parses the `--recode-audio` vocabulary: `copy`, `auto`, or an `FFmpeg`
    /// encoder name.
    ///
    /// `From` rather than `FromStr` because the conversion cannot fail — any
    /// string that is not a mode keyword *is* an encoder name, and whether that
    /// encoder exists is `FFmpeg`'s question to answer, not this type's.
    ///
    /// The two mode keywords are matched case-insensitively, matching every
    /// other format vocabulary in this crate (which gets it from
    /// `#[strum(ascii_case_insensitive)]`). The encoder name is preserved
    /// verbatim — `FFmpeg` matches encoder names exactly, so `libOpus` must
    /// survive unchanged.
    fn from(value: &str) -> Self {
        if value.eq_ignore_ascii_case("copy") {
            Self::Copy
        } else if value.eq_ignore_ascii_case("auto") {
            Self::Auto
        } else {
            Self::Encoder {
                name: value.to_owned(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_maps_the_mode_keywords_case_insensitively() {
        for spelling in ["copy", "COPY", "Copy", "cOpY"] {
            assert_eq!(RecodeAudioMode::from(spelling), RecodeAudioMode::Copy);
        }
        for spelling in ["auto", "AUTO", "Auto"] {
            assert_eq!(RecodeAudioMode::from(spelling), RecodeAudioMode::Auto);
        }
    }

    #[test]
    fn from_preserves_encoder_name_case() {
        assert_eq!(
            RecodeAudioMode::from("libOpus"),
            RecodeAudioMode::Encoder {
                name: "libOpus".to_string()
            },
            "FFmpeg matches encoder names exactly; case must not be folded"
        );
    }

    /// A keyword with surrounding content is an encoder name, not a mode —
    /// the match is on the whole value, never a substring.
    #[test]
    fn from_does_not_match_keywords_as_substrings() {
        for name in ["copycat", "autotune", "libcopy", "copy2"] {
            assert_eq!(
                RecodeAudioMode::from(name),
                RecodeAudioMode::Encoder {
                    name: name.to_string()
                },
                "{name} must be treated as an encoder name"
            );
        }
    }

    #[test]
    fn default_is_copy() {
        assert_eq!(RecodeAudioMode::default(), RecodeAudioMode::Copy);
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

    #[test]
    fn serde_encoder_roundtrip() {
        let mode = RecodeAudioMode::Encoder {
            name: "libfdk_aac".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#"{"mode":"encoder","name":"libfdk_aac"}"#);
        let parsed: RecodeAudioMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, parsed);
    }

    #[test]
    fn serde_encoder_libopus_roundtrip() {
        let mode = RecodeAudioMode::Encoder {
            name: "libopus".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: RecodeAudioMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, parsed);
    }
}
