//! Audio format types for extraction and conversion

use serde::Serialize;

use crate::parse_error::ParseEnumError;
use serde_with::DeserializeFromStr;
use strum_macros::{Display, EnumIter, EnumString};
/// Builds the `FromStr` error for [`AudioFormat`].
///
/// Named by `#[strum(parse_err_fn = ...)]`. Replaces strum's default
/// `ParseError::VariantNotFound`, whose `Display` is the fixed string
/// "Matching variant not found" — that told a user editing `config.toml`
/// neither which value was rejected nor which field it came from (#540).
fn audio_format_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError {
        type_name: "audio format",
        input: input.to_owned(),
    }
}

/// Supported audio formats for extraction and conversion.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    DeserializeFromStr,
    Display,
    EnumString,
    EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = audio_format_parse_err)]
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
    use crate::enum_test_support::{
        assert_all_parse_to, assert_display_matches, assert_display_roundtrips,
        assert_serde_spellings_are_parseable, assert_toml_accepts_every_from_str_spelling,
        assert_toml_rejects_unknown_spelling,
    };

    #[test]
    fn test_display_roundtrip() {
        assert_display_roundtrips::<AudioFormat>();
    }

    /// Every alias still parses — including the spellings displaced from
    /// `Display` when `eac3`/`dts` were promoted to `to_string` (#580). strum
    /// unions `to_string` into the `FromStr` set rather than replacing the
    /// `serialize` list, so nothing should be lost; this pins that.
    ///
    /// Case folding of each spelling is asserted by the helper.
    #[test]
    fn test_alias_parsing() {
        assert_all_parse_to(&[
            ("ogg", AudioFormat::Vorbis),
            ("wv", AudioFormat::WavPack),
            ("eac3", AudioFormat::Eac3),
            ("e-ac-3", AudioFormat::Eac3),
            ("e-ac3", AudioFormat::Eac3),
            ("dts", AudioFormat::Dts),
            ("dca", AudioFormat::Dts),
        ]);
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
        assert_display_matches::<AudioFormat>(|fmt| fmt.codec_name(), "codec_name()");
    }

    /// Precondition for #540's `Deserialize` -> `FromStr` delegation: no
    /// variant may have a serde spelling that `FromStr` rejects.
    #[test]
    fn test_serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<AudioFormat>();
    }

    /// An unknown spelling must still be an error, and the message must name it.
    #[test]
    fn test_toml_rejects_unknown_spelling() {
        assert_toml_rejects_unknown_spelling::<AudioFormat>(
            "mp3x",
            "unsupported audio format: mp3x",
        );
    }

    /// The config file must accept every spelling the CLI accepts (#540).
    #[test]
    fn test_toml_accepts_every_cli_spelling() {
        assert_toml_accepts_every_from_str_spelling::<AudioFormat>(&[
            "mp3", "aac", "m4a", "opus", "flac", "alac", "wav", "ac3", "mp2", "tta",
            // aliases the config file used to reject outright
            "vorbis", "ogg", "wavpack", "wv", "eac3", "e-ac-3", "e-ac3", "dts", "dca",
            // case-insensitivity, which serde's rename_all never honoured
            "MP3", "Ogg", "E-AC-3",
        ]);
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
