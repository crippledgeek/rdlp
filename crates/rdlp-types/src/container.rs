//! Container format types for video/audio files

use serde::Serialize;

use crate::parse_error::ParseEnumError;
use serde_with::DeserializeFromStr;
use std::path::Path;

use strum_macros::{Display, EnumIter, EnumString};

/// Builds the `FromStr` error for [`ContainerFormat`].
///
/// Named by `#[strum(parse_err_fn = ...)]`. Replaces strum's default
/// `ParseError::VariantNotFound`, whose `Display` is the fixed string
/// "Matching variant not found" — that told a user editing `config.toml`
/// neither which value was rejected nor which field it came from (#540).
fn container_format_parse_err(input: &str) -> ParseEnumError {
    ParseEnumError::new("container format", input)
}

/// Supported container formats for video/audio files.
///
/// Used for merge output, remux targets, and video recode targets.
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
#[strum(parse_err_ty = ParseEnumError, parse_err_fn = container_format_parse_err)]
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
    ///
    /// `threegp` is not a real-world spelling — it is the lowercased *variant
    /// identifier* that `#[serde(rename_all = "lowercase")]` accepted before
    /// #540 delegated `Deserialize` to `FromStr`. It is kept as a parse-only
    /// alias so configs persisted under the old vocabulary keep loading.
    #[strum(to_string = "3gp", serialize = "3gpp", serialize = "threegp")]
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

    /// Derive the container from a filesystem path's extension.
    ///
    /// The single place a path becomes a `ContainerFormat`. Honours everything
    /// [`FromStr`](std::str::FromStr) does — the canonical extension, the
    /// aliases (`matroska`, `quicktime`, ...), and ASCII case — so callers stop
    /// hand-rolling `path.extension().and_then(str::parse)` chains that each
    /// pick their own subset of that vocabulary. Answers `None` for a missing,
    /// non-UTF-8, or unrecognised extension; deciding what `None` means is the
    /// caller's job.
    ///
    /// Takes only the path's extension — no filesystem access, so this stays
    /// within the crate's pure-data remit.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(|e| e.parse().ok())
    }

    /// Whether this container supports faststart (moov atom at beginning).
    #[inline]
    #[must_use]
    pub const fn supports_faststart(&self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov | Self::M4v | Self::F4v)
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
    use strum::IntoEnumIterator as _;

    #[test]
    fn test_display_roundtrip() {
        assert_display_roundtrips::<ContainerFormat>();
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
        assert_display_matches::<ContainerFormat>(|fmt| fmt.as_ext(), "as_ext()");
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

        // Case-insensitivity across the alias set is asserted by the helper.
        assert_all_parse_to(ALIASES);
    }

    /// Every spelling the *current* serde representation accepts must also be
    /// in the `FromStr` table.
    ///
    /// This is the back-compat precondition for #540, which delegates
    /// `Deserialize` to `FromStr` so the config file and the CLI stop accepting
    /// different vocabularies. Until that lands, `#[serde(rename_all =
    /// "lowercase")]` accepts the lowercased *variant identifier*, which for
    /// `ThreeGp` is `"threegp"` — a spelling `FromStr` rejects, because strum's
    /// table holds only `3gp`/`3gpp`. Delegating without first adding `threegp`
    /// as an alias would silently break every persisted config that wrote it.
    ///
    /// Derived from the serialized form rather than a hand-listed set, so it
    /// cannot drift as variants are added.
    #[test]
    fn test_serde_spellings_are_all_parseable() {
        assert_serde_spellings_are_parseable::<ContainerFormat>();
    }

    /// #540 changed only `Deserialize`. The serialized form is a Tauri IPC
    /// contract, mirrored by a TypeScript union and gated by
    /// `scripts/check-ts-enum-drift.sh`, so it must not move.
    ///
    /// `ThreeGp` is where a regression would surface: `Serialize` still emits
    /// the `rename_all` spelling `"threegp"`, while `Display`/`as_ext` render
    /// `"3gp"`. That asymmetry is deliberate — the wire keeps the old value,
    /// the filesystem gets the real extension, and `FromStr` now accepts both.
    #[test]
    fn test_serialize_still_emits_the_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&ContainerFormat::ThreeGp).expect("serialize"),
            "\"threegp\"",
            "the IPC wire value must not change; only Deserialize was widened"
        );
        assert_eq!(ContainerFormat::ThreeGp.to_string(), "3gp");
        assert_eq!(ContainerFormat::ThreeGp.as_ext(), "3gp");
    }

    /// An unknown spelling must still be an error, and the message must name it.
    #[test]
    fn test_toml_rejects_unknown_spelling() {
        assert_toml_rejects_unknown_spelling::<ContainerFormat>(
            "mkvv",
            "unsupported container format: mkvv",
        );
    }

    /// The config file must accept every spelling the CLI accepts (#540).
    #[test]
    fn test_toml_accepts_every_cli_spelling() {
        assert_toml_accepts_every_from_str_spelling::<ContainerFormat>(&[
            // canonical extensions
            "mp4",
            "mkv",
            "3gp",
            "wav",
            "aac",
            "wv",
            "mov",
            "ts",
            "mpg",
            // strum aliases the config file used to reject outright
            "matroska",
            "quicktime",
            "mpegts",
            "3gpp",
            "mpeg",
            "wave",
            "adts",
            "aif",
            "wavpack",
            // the pre-#540 serde-only spelling, kept for persisted configs
            "threegp",
            // case-insensitivity, which serde's rename_all never honoured
            "MP4",
            "Mkv",
            "MATROSKA",
        ]);
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

    /// `from_path` is the one place a filesystem path becomes a container, so
    /// it must honour everything `FromStr` honours: the canonical extension,
    /// the strum aliases, and ASCII case-insensitivity.
    #[test]
    fn from_path_accepts_canonical_aliases_and_any_case() {
        for (path, want) in [
            ("/tmp/Title.mkv", ContainerFormat::Mkv),
            ("/tmp/Title.matroska", ContainerFormat::Mkv),
            ("/tmp/Title.MKV", ContainerFormat::Mkv),
            ("/tmp/Title.quicktime", ContainerFormat::Mov),
            ("Title.mp4", ContainerFormat::Mp4),
            ("/tmp/a.b.c/Title.with.dots.webm", ContainerFormat::WebM),
        ] {
            assert_eq!(
                ContainerFormat::from_path(std::path::Path::new(path)),
                Some(want),
                "{path}"
            );
        }
    }

    /// The negative space: anything that is not a recognised container answers
    /// `None` rather than guessing a default.
    #[test]
    fn from_path_rejects_missing_unknown_and_non_container_extensions() {
        for path in [
            "/tmp/Title",
            "/tmp/Title.",
            "/tmp/.hidden",
            "/tmp/Title.rdlp-part",
            "/tmp/Title.srt",
            "/tmp/Title.xyz",
            "",
        ] {
            assert_eq!(
                ContainerFormat::from_path(std::path::Path::new(path)),
                None,
                "{path:?} must not resolve to a container"
            );
        }
    }
}
