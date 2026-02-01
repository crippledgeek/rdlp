//! Container format types for video/audio files

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Supported container formats for video/audio files.
///
/// Used for merge output, remux targets, and video recode targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerFormat {
    /// MPEG-4 Part 14 - best compatibility, supports faststart
    Mp4,
    /// Matroska - supports all codecs, efficient cues index
    Mkv,
    /// Web-optimized, VP8/VP9/AV1 + Opus/Vorbis
    WebM,
    /// Apple QuickTime, good for editing
    Mov,
    /// MPEG Transport Stream, broadcast/streaming
    Ts,
    /// Flash Video, legacy format
    Flv,
    /// Audio Video Interleave, legacy format
    Avi,
    /// Ogg container
    Ogg,
    /// MPEG-4 Audio (audio-only container)
    M4a,
}

impl ContainerFormat {
    /// File extension for this container format.
    #[must_use]
    pub fn as_ext(&self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::WebM => "webm",
            Self::Mov => "mov",
            Self::Ts => "ts",
            Self::Flv => "flv",
            Self::Avi => "avi",
            Self::Ogg => "ogg",
            Self::M4a => "m4a",
        }
    }

    /// Whether this container supports faststart (moov atom at beginning).
    #[must_use]
    pub fn supports_faststart(&self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov)
    }
}

impl fmt::Display for ContainerFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ext())
    }
}

impl FromStr for ContainerFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mp4" => Ok(Self::Mp4),
            "mkv" | "matroska" => Ok(Self::Mkv),
            "webm" => Ok(Self::WebM),
            "mov" | "quicktime" => Ok(Self::Mov),
            "ts" | "mpegts" => Ok(Self::Ts),
            "flv" => Ok(Self::Flv),
            "avi" => Ok(Self::Avi),
            "ogg" => Ok(Self::Ogg),
            "m4a" => Ok(Self::M4a),
            _ => Err(format!("unsupported container format: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_roundtrip() {
        for fmt in [
            ContainerFormat::Mp4,
            ContainerFormat::Mkv,
            ContainerFormat::WebM,
            ContainerFormat::Mov,
            ContainerFormat::Ts,
            ContainerFormat::Flv,
            ContainerFormat::Avi,
            ContainerFormat::Ogg,
            ContainerFormat::M4a,
        ] {
            let s = fmt.to_string();
            let parsed: ContainerFormat = s.parse().unwrap();
            assert_eq!(fmt, parsed);
        }
    }

    #[test]
    fn test_case_insensitive_parse() {
        assert_eq!(
            "MP4".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Mp4
        );
        assert_eq!(
            "MKV".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Mkv
        );
        assert_eq!(
            "Matroska".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Mkv
        );
    }

    #[test]
    fn test_serde_roundtrip() {
        let fmt = ContainerFormat::Mp4;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(json, "\"mp4\"");
        let parsed: ContainerFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, parsed);
    }

    #[test]
    fn test_faststart() {
        assert!(ContainerFormat::Mp4.supports_faststart());
        assert!(ContainerFormat::Mov.supports_faststart());
        assert!(!ContainerFormat::Mkv.supports_faststart());
    }
}
