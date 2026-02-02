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
    // === Video containers ===
    /// MPEG-4 Part 14 — best compatibility, supports faststart
    Mp4,
    /// Matroska — supports all codecs, efficient cues index
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
    /// 3GPP mobile video
    ThreeGp,
    /// MPEG-1/2 program stream
    Mpg,
    /// Flash Video (MP4 variant)
    F4v,
    /// Advanced Streaming Format / Windows Media
    Asf,
    /// Material eXchange Format (broadcast/professional)
    Mxf,
    /// DVD Video Object
    Vob,
    /// Digital Video
    Dv,
    /// NUT (FFmpeg native container)
    Nut,
    /// On2 IVF (VP8/VP9/AV1 raw)
    Ivf,

    // === Audio containers ===
    /// Ogg container
    Ogg,
    /// MPEG-4 Audio (audio-only container)
    M4a,
    /// MPEG Audio Layer 3
    Mp3,
    /// Waveform Audio (PCM)
    Wav,
    /// Free Lossless Audio Codec
    Flac,
    /// Ogg Opus
    Opus,
    /// Raw ADTS AAC
    Aac,
    /// Audio Interchange File Format (Apple)
    Aiff,
    /// Matroska Audio
    Mka,
    /// WavPack lossless
    Wv,
    /// Core Audio Format (Apple)
    Caf,
    /// Dolby AC-3
    Ac3,
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
            Self::ThreeGp => "3gp",
            Self::Mpg => "mpg",
            Self::F4v => "f4v",
            Self::Asf => "asf",
            Self::Mxf => "mxf",
            Self::Vob => "vob",
            Self::Dv => "dv",
            Self::Nut => "nut",
            Self::Ivf => "ivf",
            Self::Ogg => "ogg",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Opus => "opus",
            Self::Aac => "aac",
            Self::Aiff => "aiff",
            Self::Mka => "mka",
            Self::Wv => "wv",
            Self::Caf => "caf",
            Self::Ac3 => "ac3",
        }
    }

    /// Whether this container supports faststart (moov atom at beginning).
    #[must_use]
    pub fn supports_faststart(&self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov | Self::F4v)
    }

    /// Whether this is an audio-only container format.
    #[must_use]
    pub fn is_audio_only(&self) -> bool {
        matches!(
            self,
            Self::Ogg
                | Self::M4a
                | Self::Mp3
                | Self::Wav
                | Self::Flac
                | Self::Opus
                | Self::Aac
                | Self::Aiff
                | Self::Mka
                | Self::Wv
                | Self::Caf
                | Self::Ac3
        )
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
            "3gp" | "3gpp" => Ok(Self::ThreeGp),
            "mpg" | "mpeg" => Ok(Self::Mpg),
            "f4v" => Ok(Self::F4v),
            "asf" | "wmv" | "wma" => Ok(Self::Asf),
            "mxf" => Ok(Self::Mxf),
            "vob" => Ok(Self::Vob),
            "dv" => Ok(Self::Dv),
            "nut" => Ok(Self::Nut),
            "ivf" => Ok(Self::Ivf),
            "ogg" => Ok(Self::Ogg),
            "m4a" => Ok(Self::M4a),
            "mp3" => Ok(Self::Mp3),
            "wav" | "wave" => Ok(Self::Wav),
            "flac" => Ok(Self::Flac),
            "opus" => Ok(Self::Opus),
            "aac" | "adts" => Ok(Self::Aac),
            "aiff" | "aif" => Ok(Self::Aiff),
            "mka" => Ok(Self::Mka),
            "wv" | "wavpack" => Ok(Self::Wv),
            "caf" => Ok(Self::Caf),
            "ac3" => Ok(Self::Ac3),
            _ => Err(format!("unsupported container format: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All variants for exhaustive testing.
    const ALL_FORMATS: [ContainerFormat; 28] = [
        ContainerFormat::Mp4,
        ContainerFormat::Mkv,
        ContainerFormat::WebM,
        ContainerFormat::Mov,
        ContainerFormat::Ts,
        ContainerFormat::Flv,
        ContainerFormat::Avi,
        ContainerFormat::ThreeGp,
        ContainerFormat::Mpg,
        ContainerFormat::F4v,
        ContainerFormat::Asf,
        ContainerFormat::Mxf,
        ContainerFormat::Vob,
        ContainerFormat::Dv,
        ContainerFormat::Nut,
        ContainerFormat::Ivf,
        ContainerFormat::Ogg,
        ContainerFormat::M4a,
        ContainerFormat::Mp3,
        ContainerFormat::Wav,
        ContainerFormat::Flac,
        ContainerFormat::Opus,
        ContainerFormat::Aac,
        ContainerFormat::Aiff,
        ContainerFormat::Mka,
        ContainerFormat::Wv,
        ContainerFormat::Caf,
        ContainerFormat::Ac3,
    ];

    #[test]
    fn test_display_roundtrip() {
        for fmt in ALL_FORMATS {
            let s = fmt.to_string();
            let parsed: ContainerFormat = s.parse().unwrap();
            assert_eq!(fmt, parsed, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn test_alias_parsing() {
        assert_eq!(
            "matroska".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Mkv
        );
        assert_eq!(
            "quicktime".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Mov
        );
        assert_eq!(
            "mpegts".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Ts
        );
        assert_eq!(
            "3gpp".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::ThreeGp
        );
        assert_eq!(
            "mpeg".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Mpg
        );
        assert_eq!(
            "wmv".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Asf
        );
        assert_eq!(
            "wma".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Asf
        );
        assert_eq!(
            "wave".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Wav
        );
        assert_eq!(
            "adts".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Aac
        );
        assert_eq!(
            "aif".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Aiff
        );
        assert_eq!(
            "wavpack".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Wv
        );
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
            "WMV".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Asf
        );
        assert_eq!(
            "FLAC".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::Flac
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
        assert!(ContainerFormat::F4v.supports_faststart());
        assert!(!ContainerFormat::Mkv.supports_faststart());
        assert!(!ContainerFormat::Avi.supports_faststart());
    }

    #[test]
    fn test_is_audio_only() {
        // Audio containers
        assert!(ContainerFormat::Mp3.is_audio_only());
        assert!(ContainerFormat::Wav.is_audio_only());
        assert!(ContainerFormat::Flac.is_audio_only());
        assert!(ContainerFormat::Opus.is_audio_only());
        assert!(ContainerFormat::Aac.is_audio_only());
        assert!(ContainerFormat::M4a.is_audio_only());
        assert!(ContainerFormat::Ogg.is_audio_only());
        assert!(ContainerFormat::Mka.is_audio_only());
        assert!(ContainerFormat::Ac3.is_audio_only());

        // Video containers
        assert!(!ContainerFormat::Mp4.is_audio_only());
        assert!(!ContainerFormat::Mkv.is_audio_only());
        assert!(!ContainerFormat::Avi.is_audio_only());
    }
}
