//! Which subtitle codec a given [`ContainerFormat`] can carry.
//!
//! `SubtitleStage` previously decided this with two raw-string lists and three
//! inline `eq_ignore_ascii_case` arrays (`SUBTITLE_CONTAINERS`, plus
//! `["mp4","m4a","m4v","mov"]` / `["mkv","mka"]` / `"webm"` inside
//! `subtitle_codec_for_container`) — the same decision expressed twice, in
//! strings, with an `else` arm that defaulted to `"srt"` for anything
//! unrecognised. This module collapses both into one typed decision made from
//! [`ContainerFormat`] (per the workspace convention: container formats are
//! never raw strings — see `CLAUDE.md`), modelled directly on
//! `rdlp_ffmpeg`'s `embed_strategy::ThumbnailEmbedStrategy` (#533/#537).
//!
//! The `"srt"` default is gone: a container either resolves to a codec or is
//! not a subtitle target at all. Support and codec choice can no longer
//! disagree, because both now come from the single match below.

use rdlp_types::ContainerFormat;

/// The `FFmpeg` subtitle codec used to embed into a given container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubtitleEmbedCodec {
    /// `MPEG-4` timed text — the MP4/`QuickTime` family's native subtitle codec.
    MovText,
    /// `SubRip` — Matroska's default text subtitle codec.
    Srt,
    /// `WebVTT` — the only text subtitle codec `WebM` accepts.
    WebVtt,
}

impl SubtitleEmbedCodec {
    /// Resolve the subtitle codec for `format`, or `None` when `format` cannot
    /// carry embedded subtitles at all.
    ///
    /// Matched exhaustively over every [`ContainerFormat`] variant (no
    /// catch-all arm) so that a newly-added container format fails to compile
    /// here, forcing an explicit decision instead of silently inheriting the
    /// old `"srt"` fallback.
    #[must_use]
    pub(super) const fn for_container(format: ContainerFormat) -> Option<Self> {
        match format {
            ContainerFormat::Mp4
            | ContainerFormat::Mov
            | ContainerFormat::M4v
            | ContainerFormat::M4a => Some(Self::MovText),
            ContainerFormat::Mkv | ContainerFormat::Mka => Some(Self::Srt),
            ContainerFormat::WebM => Some(Self::WebVtt),
            ContainerFormat::Ts
            | ContainerFormat::Flv
            | ContainerFormat::Avi
            | ContainerFormat::ThreeGp
            | ContainerFormat::Mpg
            | ContainerFormat::F4v
            | ContainerFormat::Wmv
            | ContainerFormat::Wma
            | ContainerFormat::Asf
            | ContainerFormat::Mxf
            | ContainerFormat::Vob
            | ContainerFormat::Dv
            | ContainerFormat::Nut
            | ContainerFormat::Ivf
            | ContainerFormat::Ogg
            | ContainerFormat::Opus
            | ContainerFormat::Mp3
            | ContainerFormat::Wav
            | ContainerFormat::Flac
            | ContainerFormat::Aac
            | ContainerFormat::Aiff
            | ContainerFormat::Wv
            | ContainerFormat::Caf
            | ContainerFormat::Ac3 => None,
        }
    }

    /// The encoder name passed to `FFmpeg`.
    #[must_use]
    pub(super) const fn as_ffmpeg_name(self) -> &'static str {
        match self {
            Self::MovText => "mov_text",
            Self::Srt => "srt",
            Self::WebVtt => "webvtt",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::ContainerFormat;

    /// Behaviour parity: every container supported before the conversion keeps
    /// the exact codec it had, so this refactor changes nothing for anyone.
    #[test]
    fn supported_containers_keep_their_codec() {
        for (ext, want) in [
            ("mp4", "mov_text"),
            ("m4a", "mov_text"),
            ("m4v", "mov_text"),
            ("mov", "mov_text"),
            ("mkv", "srt"),
            ("mka", "srt"),
            ("webm", "webvtt"),
        ] {
            let format: ContainerFormat = ext.parse().expect("known container");
            let codec = SubtitleEmbedCodec::for_container(format)
                .unwrap_or_else(|| panic!("{ext} must still support subtitle embedding"));
            assert_eq!(
                codec.as_ffmpeg_name(),
                want,
                "codec for {ext} must not change"
            );
        }
    }

    /// The negative half, swept rather than sampled: EVERY variant outside the
    /// supported set must resolve to `None`, so nothing falls through to the
    /// old silent `"srt"` default.
    ///
    /// Iterating `ContainerFormat` pins the whole mapping. The exhaustive match
    /// only makes an *unclassified* variant fail to compile; this catches a
    /// *misclassified* one, which the compiler cannot see.
    #[test]
    fn every_variant_matches_the_pinned_support_set() {
        use strum::IntoEnumIterator as _;

        const SUPPORTED: &[ContainerFormat] = &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::M4v,
            ContainerFormat::M4a,
            ContainerFormat::Mkv,
            ContainerFormat::Mka,
            ContainerFormat::WebM,
        ];

        for format in ContainerFormat::iter() {
            assert_eq!(
                SubtitleEmbedCodec::for_container(format).is_some(),
                SUPPORTED.contains(&format),
                "{format:?} support must match the pinned set"
            );
        }
    }

    /// The widening this conversion accepts, pinned exactly as #537 pinned the
    /// thumbnail one: the typed path recognises `ContainerFormat`'s aliases, so
    /// a file named `.matroska` / `.quicktime` is now accepted where the raw
    /// string list rejected it. Deliberate, and locked down here.
    #[test]
    fn strum_aliases_are_accepted_and_map_to_the_canonical_codec() {
        let mkv: ContainerFormat = "matroska".parse().expect("alias");
        let mov: ContainerFormat = "quicktime".parse().expect("alias");

        assert_eq!(mkv, ContainerFormat::Mkv);
        assert_eq!(mov, ContainerFormat::Mov);
        assert_eq!(
            SubtitleEmbedCodec::for_container(mkv).map(SubtitleEmbedCodec::as_ffmpeg_name),
            Some("srt")
        );
        assert_eq!(
            SubtitleEmbedCodec::for_container(mov).map(SubtitleEmbedCodec::as_ffmpeg_name),
            Some("mov_text")
        );
    }

    /// Case-insensitivity came free with the string compare
    /// (`eq_ignore_ascii_case`); it must survive the move to `FromStr`.
    #[test]
    fn extension_matching_stays_case_insensitive() {
        for ext in ["MP4", "Mkv", "WEBM"] {
            let format: ContainerFormat = ext.parse().expect("case-insensitive parse");
            assert!(
                SubtitleEmbedCodec::for_container(format).is_some(),
                "{ext} must still be recognised"
            );
        }
    }
}
