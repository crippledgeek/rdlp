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
    /// Windows Media Video — ASF container carrying a video stream.
    ///
    /// Shares `FFmpeg`'s `asf` muxer with [`Self::Wma`] and [`Self::Asf`]
    /// (the muxer declares `Common extensions: asf,wmv,wma`), but is a
    /// distinct variant so the extension survives to the output filename —
    /// the same shape as [`Self::Mkv`]/[`Self::Mka`] over the matroska muxer.
    /// See [`Self::Asf`] for why the spelling is load-bearing (#538).
    #[strum(serialize = "wmv")]
    Wmv,
    /// Windows Media Audio — ASF container carrying only audio.
    ///
    /// Audio-only sibling of [`Self::Wmv`], mirroring [`Self::Mka`] to
    /// [`Self::Mkv`].
    #[strum(serialize = "wma")]
    Wma,
    /// Advanced Systems Format — the *fallback* spelling of the ASF family.
    ///
    /// Microsoft's [File Name Extension Guidelines] make this the name for ASF
    /// content carrying third-party or otherwise unsupported streams: a file is
    /// `.wmv` when it holds video, `.wma` when it holds only audio, and `.asf`
    /// only otherwise. Because `.asf` is the exception rather than the family
    /// name, `wmv`/`wma` are separate variants instead of aliases that
    /// canonicalize here — folding them in made `--remux=wmv` write
    /// `Title.asf`, inverting Microsoft's rule (#538).
    ///
    /// [File Name Extension Guidelines]: https://learn.microsoft.com/en-us/windows/win32/wmformat/file-name-extension-guidelines
    #[strum(serialize = "asf")]
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
            Self::Wmv => "wmv",
            Self::Wma => "wma",
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
                // Microsoft's File Name Extension Guidelines are explicit that
                // `.wma` names ASF content with "no supported video streams" —
                // a video stream of any codec belongs in `.wmv` or `.asf`. This
                // predicate records that classification; it does NOT yet drop
                // video streams from a `--remux=wma` (see #538 follow-up).
                | Self::Wma
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
            // `wmv`/`wma` were aliases of `Asf` until #538 gave them their own
            // variants; they are covered by `test_wmv_wma_keep_their_own_extension`.
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
            ContainerFormat::Wmv
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

    /// `wmv` and `wma` keep their own extension instead of canonicalizing to
    /// `asf` (#538).
    ///
    /// Microsoft's File Name Extension Guidelines make `.asf` the *fallback*
    /// spelling — reserved for ASF content carrying third-party or unsupported
    /// streams — while `.wmv` and `.wma` are the expected names for Windows
    /// Media video and audio. Folding all three onto one variant whose
    /// `as_ext()` is `"asf"` inverted that rule: `--remux=wmv` wrote
    /// `Title.asf`. They are three variants over one `FFmpeg` muxer (`asf`
    /// declares `Common extensions: asf,wmv,wma`), mirroring how `Mkv`/`Mka`
    /// and `Mp4`/`M4a` already share a muxer while keeping distinct extensions.
    #[test]
    fn test_wmv_wma_keep_their_own_extension() {
        const SPELLINGS: &[(&str, ContainerFormat, &str)] = &[
            ("wmv", ContainerFormat::Wmv, "wmv"),
            ("wma", ContainerFormat::Wma, "wma"),
            ("asf", ContainerFormat::Asf, "asf"),
        ];

        for (spelling, expected, ext) in SPELLINGS {
            let parsed: ContainerFormat = spelling
                .parse()
                .unwrap_or_else(|_| panic!("'{spelling}' must parse"));
            assert_eq!(parsed, *expected, "'{spelling}' must parse to {expected:?}");
            assert_eq!(
                parsed.as_ext(),
                *ext,
                "'{spelling}' must keep its own extension, not canonicalize"
            );
        }

        // The three are distinct containers, not aliases of one another.
        assert_ne!(ContainerFormat::Wmv, ContainerFormat::Asf);
        assert_ne!(ContainerFormat::Wma, ContainerFormat::Asf);
        assert_ne!(ContainerFormat::Wmv, ContainerFormat::Wma);
    }

    /// The counter-direction guard for #538: aliases that are format *names*
    /// rather than file extensions must keep canonicalizing.
    ///
    /// This is the regression this fix could most easily cause. A general
    /// "preserve whatever the user typed" rule would fix `wmv` and
    /// simultaneously start writing `Title.matroska` / `Title.quicktime` —
    /// spellings no muxer lookup recognises and no player expects. Matroska's
    /// own documentation uses only `.mkv`; Apple's uses `.mov`; `FFmpeg`'s mpegts
    /// muxer lists `ts,m2t,m2ts,mts` and never `mpegts`.
    #[test]
    fn test_format_name_aliases_still_canonicalize() {
        const CANONICALIZING: &[(&str, &str)] = &[
            ("matroska", "mkv"),
            ("quicktime", "mov"),
            ("mpegts", "ts"),
            ("3gpp", "3gp"),
            ("mpeg", "mpg"),
            ("wave", "wav"),
            ("adts", "aac"),
            ("aif", "aiff"),
            ("wavpack", "wv"),
        ];

        for (alias, canonical_ext) in CANONICALIZING {
            let parsed: ContainerFormat = alias
                .parse()
                .unwrap_or_else(|_| panic!("alias '{alias}' must parse"));
            assert_eq!(
                parsed.as_ext(),
                *canonical_ext,
                "alias '{alias}' is a format name, not an extension — it must \
                 canonicalize to '{canonical_ext}', never be preserved verbatim"
            );
        }
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

        // ASF family (#538). The negative pair carries more weight than the
        // positive: `is_audio_only` is a `matches!`, so it can never be made
        // exhaustive — a misclassified `Wmv`/`Asf` would be silent by
        // construction, and dropping `| Self::Wma` from the predicate must not
        // leave the suite green.
        assert!(
            ContainerFormat::Wma.is_audio_only(),
            "wma names ASF content with no supported video streams"
        );
        assert!(
            !ContainerFormat::Wmv.is_audio_only(),
            "wmv is the ASF spelling that carries video"
        );
        assert!(
            !ContainerFormat::Asf.is_audio_only(),
            "asf is the general fallback and may carry video"
        );
    }
}
