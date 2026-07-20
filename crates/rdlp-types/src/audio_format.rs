//! Audio format types for extraction and conversion

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter, EnumString};

/// Supported audio formats for extraction and conversion.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
pub enum AudioFormat {
    /// MPEG Audio Layer 3
    #[strum(serialize = "mp3")]
    Mp3,
    /// Advanced Audio Coding
    #[strum(serialize = "aac")]
    Aac,
    /// MPEG-4 Audio (AAC in M4A container)
    #[strum(serialize = "m4a")]
    M4a,
    /// Opus codec
    #[strum(serialize = "opus")]
    Opus,
    /// Vorbis codec
    #[strum(serialize = "vorbis", serialize = "ogg")]
    Vorbis,
    /// Free Lossless Audio Codec
    #[strum(serialize = "flac")]
    Flac,
    /// Apple Lossless Audio Codec
    #[strum(serialize = "alac")]
    Alac,
    /// Waveform Audio File Format (PCM)
    #[strum(serialize = "wav")]
    Wav,
    /// Dolby Digital (AC-3)
    #[strum(serialize = "ac3")]
    Ac3,
    /// Dolby Digital Plus (Enhanced AC-3)
    #[strum(to_string = "eac3", serialize = "e-ac-3", serialize = "e-ac3")]
    Eac3,
    /// DTS Coherent Acoustics
    #[strum(to_string = "dts", serialize = "dca")]
    Dts,
    /// MPEG Audio Layer 2
    #[strum(serialize = "mp2")]
    Mp2,
    /// `WavPack` lossless
    #[strum(serialize = "wavpack", serialize = "wv")]
    WavPack,
    /// True Audio lossless
    #[strum(serialize = "tta")]
    Tta,
}

impl AudioFormat {
    /// File extension for this audio format.
    #[inline]
    #[must_use]
    pub const fn as_ext(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Aac => "aac",
            Self::M4a | Self::Alac => "m4a",
            Self::Opus => "opus",
            Self::Vorbis => "ogg",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Ac3 => "ac3",
            Self::Eac3 => "eac3",
            Self::Dts => "dts",
            Self::Mp2 => "mp2",
            Self::WavPack => "wv",
            Self::Tta => "tta",
        }
    }

    /// Codec lookup name (matches `AUDIO_CODECS` keys in ffmpeg.rs).
    #[inline]
    #[must_use]
    pub const fn codec_name(&self) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

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

    /// Promoting a spelling from `serialize` to `to_string` must not drop any
    /// other spelling: strum builds the `FromStr` table from `serialize` **plus**
    /// `to_string`, so every pre-#580 input stays accepted. The canonical
    /// spellings are asserted alongside the aliases because `to_string` is what
    /// moved.
    #[test]
    fn test_to_string_promotion_preserved_every_spelling() {
        for (input, expected) in [
            ("eac3", AudioFormat::Eac3),
            ("e-ac-3", AudioFormat::Eac3),
            ("e-ac3", AudioFormat::Eac3),
            ("dts", AudioFormat::Dts),
            ("dca", AudioFormat::Dts),
        ] {
            assert_eq!(
                input.parse::<AudioFormat>().unwrap(),
                expected,
                "{input} must still parse after the to_string promotion"
            );
        }
    }

    /// `ascii_case_insensitive` is a container-level default applied to every
    /// spelling strum accepts — including `to_string` values, not just
    /// `serialize` ones.
    #[test]
    fn test_promoted_spellings_stay_case_insensitive() {
        for input in ["EAC3", "E-AC-3", "DTS", "Dca"] {
            assert!(
                input.parse::<AudioFormat>().is_ok(),
                "{input} must parse case-insensitively"
            );
        }
    }

    /// The codec-vs-container split is intended, not a bug to "fix".
    ///
    /// These three variants are the reason this enum's `Display` guard is
    /// pinned to `codec_name()` rather than `as_ext()` — a future reviewer
    /// mirroring #545's `ContainerFormat` rule onto this enum would otherwise
    /// make `Vorbis` render `ogg`. See #580.
    #[test]
    fn test_codec_name_and_ext_intentionally_differ() {
        assert_eq!(AudioFormat::Vorbis.codec_name(), "vorbis");
        assert_eq!(AudioFormat::Vorbis.as_ext(), "ogg");
        assert_eq!(AudioFormat::Alac.codec_name(), "alac");
        assert_eq!(AudioFormat::Alac.as_ext(), "m4a");
        assert_eq!(AudioFormat::WavPack.codec_name(), "wavpack");
        assert_eq!(AudioFormat::WavPack.as_ext(), "wv");
    }

    /// `Display` must render the codec name, not whichever strum alias happens
    /// to be longest.
    ///
    /// `AudioFormat` is a *codec* enum: [`AudioFormat::as_ext`] deliberately
    /// returns the **container** extension the codec is carried in (`Vorbis` →
    /// `ogg`, `Alac` → `m4a`), so the projection `Display` must agree with is
    /// [`AudioFormat::codec_name`], not `as_ext`. See #580; the
    /// `ContainerFormat` sibling of this guard is pinned to `as_ext` instead
    /// (#545) because that enum names containers.
    #[test]
    fn test_display_equals_codec_name() {
        for fmt in AudioFormat::iter() {
            assert_eq!(
                fmt.to_string(),
                fmt.codec_name(),
                "Display for {fmt:?} must equal codec_name()"
            );
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
