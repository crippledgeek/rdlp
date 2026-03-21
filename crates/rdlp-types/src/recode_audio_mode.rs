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
    /// Use a specific encoder (e.g., "libfdk_aac", "libopus").
    Encoder {
        /// FFmpeg encoder name (e.g., "libfdk_aac", "libopus").
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

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
