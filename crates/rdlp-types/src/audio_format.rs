//! Audio format types for extraction and conversion

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::parse_error::ParseEnumError;

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
    /// Dolby Digital (AC-3)
    Ac3,
    /// Dolby Digital Plus (Enhanced AC-3)
    Eac3,
    /// DTS Coherent Acoustics
    Dts,
    /// MPEG Audio Layer 2
    Mp2,
    /// WavPack lossless
    WavPack,
    /// True Audio lossless
    Tta,
}

impl AudioFormat {
    /// File extension for this audio format.
    #[inline]
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
            Self::Ac3 => "ac3",
            Self::Eac3 => "eac3",
            Self::Dts => "dts",
            Self::Mp2 => "mp2",
            Self::WavPack => "wv",
            Self::Tta => "tta",
        }
    }

    /// Codec lookup name (matches AUDIO_CODECS keys in ffmpeg.rs).
    #[inline]
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
            Self::Ac3 => "ac3",
            Self::Eac3 => "eac3",
            Self::Dts => "dts",
            Self::Mp2 => "mp2",
            Self::WavPack => "wavpack",
            Self::Tta => "tta",
        }
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.codec_name())
    }
}

impl FromStr for AudioFormat {
    type Err = ParseEnumError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("mp3") {
            Ok(Self::Mp3)
        } else if s.eq_ignore_ascii_case("aac") {
            Ok(Self::Aac)
        } else if s.eq_ignore_ascii_case("m4a") {
            Ok(Self::M4a)
        } else if s.eq_ignore_ascii_case("opus") {
            Ok(Self::Opus)
        } else if s.eq_ignore_ascii_case("vorbis") || s.eq_ignore_ascii_case("ogg") {
            Ok(Self::Vorbis)
        } else if s.eq_ignore_ascii_case("flac") {
            Ok(Self::Flac)
        } else if s.eq_ignore_ascii_case("alac") {
            Ok(Self::Alac)
        } else if s.eq_ignore_ascii_case("wav") {
            Ok(Self::Wav)
        } else if s.eq_ignore_ascii_case("ac3") {
            Ok(Self::Ac3)
        } else if s.eq_ignore_ascii_case("eac3")
            || s.eq_ignore_ascii_case("e-ac-3")
            || s.eq_ignore_ascii_case("e-ac3")
        {
            Ok(Self::Eac3)
        } else if s.eq_ignore_ascii_case("dts") || s.eq_ignore_ascii_case("dca") {
            Ok(Self::Dts)
        } else if s.eq_ignore_ascii_case("mp2") {
            Ok(Self::Mp2)
        } else if s.eq_ignore_ascii_case("wavpack") || s.eq_ignore_ascii_case("wv") {
            Ok(Self::WavPack)
        } else if s.eq_ignore_ascii_case("tta") {
            Ok(Self::Tta)
        } else {
            Err(ParseEnumError {
                type_name: "AudioFormat",
                input: s.to_string(),
            })
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
            AudioFormat::Ac3,
            AudioFormat::Eac3,
            AudioFormat::Dts,
            AudioFormat::Mp2,
            AudioFormat::WavPack,
            AudioFormat::Tta,
        ] {
            let s = fmt.to_string();
            let parsed: AudioFormat = s.parse().unwrap();
            assert_eq!(fmt, parsed, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn test_alias_parsing() {
        assert_eq!("ogg".parse::<AudioFormat>().unwrap(), AudioFormat::Vorbis);
        assert_eq!("e-ac-3".parse::<AudioFormat>().unwrap(), AudioFormat::Eac3);
        assert_eq!("e-ac3".parse::<AudioFormat>().unwrap(), AudioFormat::Eac3);
        assert_eq!("dca".parse::<AudioFormat>().unwrap(), AudioFormat::Dts);
        assert_eq!("wv".parse::<AudioFormat>().unwrap(), AudioFormat::WavPack);
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
