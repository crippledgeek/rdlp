//! Audio format types for extraction and conversion

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Supported audio formats for extraction and conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// MPEG Audio Layer 3
    Mp3,
    /// Advanced Audio Coding
    Aac,
    /// MPEG-4 Audio (AAC in M4A container)
    M4a,
    /// Opus codec
    Opus,
    /// Vorbis codec
    Vorbis,
    /// Free Lossless Audio Codec
    Flac,
    /// Apple Lossless Audio Codec
    Alac,
    /// Waveform Audio File Format (PCM)
    Wav,
}

impl AudioFormat {
    /// File extension for this audio format.
    #[must_use]
    pub fn as_ext(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Aac => "aac",
            Self::M4a => "m4a",
            Self::Opus => "opus",
            Self::Vorbis => "ogg",
            Self::Flac => "flac",
            Self::Alac => "m4a",
            Self::Wav => "wav",
        }
    }

    /// Codec lookup name (matches AUDIO_CODECS keys in ffmpeg.rs).
    #[must_use]
    pub fn codec_name(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Aac => "aac",
            Self::M4a => "m4a",
            Self::Opus => "opus",
            Self::Vorbis => "vorbis",
            Self::Flac => "flac",
            Self::Alac => "alac",
            Self::Wav => "wav",
        }
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.codec_name())
    }
}

impl FromStr for AudioFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mp3" => Ok(Self::Mp3),
            "aac" => Ok(Self::Aac),
            "m4a" => Ok(Self::M4a),
            "opus" => Ok(Self::Opus),
            "vorbis" | "ogg" => Ok(Self::Vorbis),
            "flac" => Ok(Self::Flac),
            "alac" => Ok(Self::Alac),
            "wav" => Ok(Self::Wav),
            _ => Err(format!("unsupported audio format: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_roundtrip() {
        for fmt in [
            AudioFormat::Mp3,
            AudioFormat::Aac,
            AudioFormat::M4a,
            AudioFormat::Opus,
            AudioFormat::Vorbis,
            AudioFormat::Flac,
            AudioFormat::Alac,
            AudioFormat::Wav,
        ] {
            let s = fmt.to_string();
            let parsed: AudioFormat = s.parse().unwrap();
            assert_eq!(fmt, parsed);
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let fmt = AudioFormat::Mp3;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(json, "\"mp3\"");
        let parsed: AudioFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, parsed);
    }
}
