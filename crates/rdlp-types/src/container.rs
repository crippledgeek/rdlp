//! Container format types for video/audio files

use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter, EnumString};

/// Supported container formats for video/audio files.
///
/// Used for merge output, remux targets, and video recode targets.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(ascii_case_insensitive)]
pub enum ContainerFormat {
    // === Video containers ===
    /// MPEG-4 Part 14 — best compatibility, supports faststart
    #[strum(serialize = "mp4")]
    Mp4,
    /// Matroska — supports all codecs, efficient cues index
    #[strum(to_string = "mkv", serialize = "matroska")]
    Mkv,
    /// Web-optimized, VP8/VP9/AV1 + Opus/Vorbis
    #[strum(serialize = "webm")]
    WebM,
    /// Apple `QuickTime`, good for editing
    #[strum(to_string = "mov", serialize = "quicktime")]
    Mov,
    /// MPEG-4 Part 14, video variant (iTunes video) — audio-only sibling is [`Self::M4a`]
    #[strum(serialize = "m4v")]
    M4v,
    /// MPEG Transport Stream, broadcast/streaming
    #[strum(to_string = "ts", serialize = "mpegts")]
    Ts,
    /// Flash Video, legacy format
    #[strum(serialize = "flv")]
    Flv,
    /// Audio Video Interleave, legacy format
    #[strum(serialize = "avi")]
    Avi,
    /// 3GPP mobile video
    #[strum(to_string = "3gp", serialize = "3gpp")]
    ThreeGp,
    /// MPEG-1/2 program stream
    #[strum(to_string = "mpg", serialize = "mpeg")]
    Mpg,
    /// Flash Video (MP4 variant)
    #[strum(serialize = "f4v")]
    F4v,
    /// Advanced Streaming Format / Windows Media
    #[strum(to_string = "asf", serialize = "wmv", serialize = "wma")]
    Asf,
    /// Material eXchange Format (broadcast/professional)
    #[strum(serialize = "mxf")]
    Mxf,
    /// DVD Video Object
    #[strum(serialize = "vob")]
    Vob,
    /// Digital Video
    #[strum(serialize = "dv")]
    Dv,
    /// NUT (`FFmpeg` native container)
    #[strum(serialize = "nut")]
    Nut,
    /// On2 IVF (VP8/VP9/AV1 raw)
    #[strum(serialize = "ivf")]
    Ivf,

    // === Audio containers ===
    /// Ogg container
    #[strum(serialize = "ogg")]
    Ogg,
    /// MPEG-4 Audio (audio-only container)
    #[strum(serialize = "m4a")]
    M4a,
    /// MPEG Audio Layer 3
    #[strum(serialize = "mp3")]
    Mp3,
    /// Waveform Audio (PCM)
    #[strum(to_string = "wav", serialize = "wave")]
    Wav,
    /// Free Lossless Audio Codec
    #[strum(serialize = "flac")]
    Flac,
    /// Ogg Opus
    #[strum(serialize = "opus")]
    Opus,
    /// Raw ADTS AAC
    #[strum(to_string = "aac", serialize = "adts")]
    Aac,
    /// Audio Interchange File Format (Apple)
    #[strum(serialize = "aiff", serialize = "aif")]
    Aiff,
    /// Matroska Audio
    #[strum(serialize = "mka")]
    Mka,
    /// `WavPack` lossless
    #[strum(to_string = "wv", serialize = "wavpack")]
    Wv,
    /// Core Audio Format (Apple)
    #[strum(serialize = "caf")]
    Caf,
    /// Dolby AC-3
    #[strum(serialize = "ac3")]
    Ac3,
}

impl ContainerFormat {
    /// File extension for this container format.
    #[inline]
    #[must_use]
    pub const fn as_ext(&self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::WebM => "webm",
            Self::Mov => "mov",
            Self::M4v => "m4v",
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
    #[inline]
    #[must_use]
    pub const fn supports_faststart(&self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov | Self::M4v | Self::F4v)
    }

    /// Whether this is an audio-only container format.
    #[inline]
    #[must_use]
    pub const fn is_audio_only(&self) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator as _;

    #[test]
    fn test_display_roundtrip() {
        for fmt in ContainerFormat::iter() {
            let s = fmt.to_string();
            let parsed: ContainerFormat = s.parse().unwrap();
            assert_eq!(fmt, parsed, "roundtrip failed for {s}");
        }
    }

    /// `Display` must render the file extension, not an alias.
    ///
    /// strum picks the longest `serialize` value when no `to_string` is set, so
    /// without an explicit `to_string` this silently returns `"matroska"` for
    /// `Mkv`, `"quicktime"` for `Mov`, and so on (#545). Anything that formats a
    /// `ContainerFormat` into a filename or an `FFmpeg` extension probe then gets a
    /// string no muxer lookup recognises.
    #[test]
    fn test_display_is_the_file_extension() {
        for fmt in ContainerFormat::iter() {
            assert_eq!(
                fmt.to_string(),
                fmt.as_ext(),
                "Display for {fmt:?} must equal as_ext()"
            );
        }
    }

    /// Every alias must still parse after `to_string` is added.
    ///
    /// strum unions `to_string` into the `FromStr` set rather than replacing the
    /// `serialize` list, so this holds — but it is the exact way the #545 fix
    /// could have silently narrowed the accepted CLI vocabulary, so it is pinned.
    #[test]
    fn test_every_alias_still_parses() {
        const ALIASES: &[(&str, ContainerFormat)] = &[
            ("matroska", ContainerFormat::Mkv),
            ("quicktime", ContainerFormat::Mov),
            ("mpegts", ContainerFormat::Ts),
            ("3gpp", ContainerFormat::ThreeGp),
            ("mpeg", ContainerFormat::Mpg),
            ("wmv", ContainerFormat::Asf),
            ("wma", ContainerFormat::Asf),
            ("wave", ContainerFormat::Wav),
            ("adts", ContainerFormat::Aac),
            ("aif", ContainerFormat::Aiff),
            ("wavpack", ContainerFormat::Wv),
        ];

        for (alias, expected) in ALIASES {
            assert_eq!(
                alias.parse::<ContainerFormat>().ok(),
                Some(*expected),
                "alias '{alias}' must keep parsing"
            );
            // Case-insensitivity must cover the alias set too.
            assert_eq!(
                alias.to_uppercase().parse::<ContainerFormat>().ok(),
                Some(*expected),
                "alias '{alias}' must keep parsing case-insensitively"
            );
        }
    }

    /// Each variant's own extension parses back to that variant.
    #[test]
    fn test_as_ext_roundtrips() {
        for fmt in ContainerFormat::iter() {
            let ext = fmt.as_ext();
            assert_eq!(
                ext.parse::<ContainerFormat>().ok(),
                Some(fmt),
                "as_ext() '{ext}' must parse back to {fmt:?}"
            );
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
        assert!(ContainerFormat::M4v.supports_faststart());
        assert!(ContainerFormat::F4v.supports_faststart());
        assert!(!ContainerFormat::Mkv.supports_faststart());
        assert!(!ContainerFormat::Avi.supports_faststart());
    }

    /// `m4v` (MPEG-4 video) parses to its own variant rather than silently
    /// aliasing `Mp4`/`Mov` — it is a distinct extension already used by the
    /// MP4-family thumbnail-embed strategy (`rdlp-postprocess`,
    /// `rdlp-ffmpeg`) before this variant existed.
    #[test]
    fn test_m4v_parses_to_its_own_variant() {
        assert_eq!(
            "m4v".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::M4v
        );
        assert_eq!(
            "M4V".parse::<ContainerFormat>().unwrap(),
            ContainerFormat::M4v
        );
        assert_eq!(ContainerFormat::M4v.as_ext(), "m4v");
        assert!(!ContainerFormat::M4v.is_audio_only());
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
