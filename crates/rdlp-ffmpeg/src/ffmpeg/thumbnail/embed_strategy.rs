//! How a thumbnail is carried into a given [`ContainerFormat`].
//!
//! `embed_thumbnail_sync` previously decided this with four independent
//! `container.eq_ignore_ascii_case(...)` checks (`is_mkv`, `is_mp3`,
//! `is_mp4_mov`, and the Ogg/Opus check added for #531) — the same decision
//! expressed piecemeal, entirely in raw strings. This module collapses it
//! into one typed decision made once, from [`ContainerFormat`] rather than a
//! bare `&str` (per the workspace convention: container formats are never raw
//! strings — see `CLAUDE.md`).

use rdlp_types::ContainerFormat;

/// The mechanism used to carry a thumbnail into a media container.
///
/// Constructed once per embed via [`Self::for_container`] and matched
/// exhaustively at each decision point in `embed_thumbnail_sync`, replacing
/// the four scattered string predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThumbnailEmbedStrategy {
    /// Native Matroska attachment (raw FFI), not a stream at all.
    MatroskaAttachment,
    /// `ATTACHED_PIC` video stream, only audio streams mapped from the
    /// source, `ID3v2` title/comment tags on the picture stream.
    Id3Apic,
    /// `ATTACHED_PIC` video stream; the muxer converts it to a native FLAC
    /// `PICTURE` block, which must be written to the picture stream BEFORE
    /// the header (`write_header`) is written.
    FlacAttachedPic,
    /// `ATTACHED_PIC` video stream with `movflags=+faststart` (moov atom
    /// first, for Windows Explorer thumbnail visibility); picture packet
    /// written after the header, same as any other stream.
    Mp4FamilyAttachedPic,
    /// No stream at all — the cover rides a base64 `METADATA_BLOCK_PICTURE`
    /// `VorbisComment` field set before the header is written (#531; see
    /// `super::vorbis_picture`). `FFmpeg`'s Ogg muxer hard-rejects any stream
    /// whose codec isn't Vorbis/Theora/Speex/FLAC/Opus/VP8, so an
    /// `ATTACHED_PIC` image stream is not an option here.
    VorbisComment,
}

impl ThumbnailEmbedStrategy {
    /// Resolve the embed strategy for `format`, or `None` when `format`
    /// cannot carry a thumbnail at all.
    ///
    /// Matched exhaustively over every [`ContainerFormat`] variant (no
    /// catch-all arm) so that a newly-added container format fails to
    /// compile here, forcing an explicit decision about its thumbnail
    /// strategy — the same exhaustive-match discipline `vorbis_picture::
    /// mime_type` uses for [`rdlp_types::ThumbnailFormat`] (#525).
    #[must_use]
    pub(super) const fn for_container(format: ContainerFormat) -> Option<Self> {
        match format {
            ContainerFormat::Mkv | ContainerFormat::Mka => Some(Self::MatroskaAttachment),
            ContainerFormat::Mp3 => Some(Self::Id3Apic),
            ContainerFormat::Flac => Some(Self::FlacAttachedPic),
            ContainerFormat::Mp4
            | ContainerFormat::Mov
            | ContainerFormat::M4v
            | ContainerFormat::M4a => Some(Self::Mp4FamilyAttachedPic),
            ContainerFormat::Ogg | ContainerFormat::Opus => Some(Self::VorbisComment),
            ContainerFormat::WebM
            | ContainerFormat::Ts
            | ContainerFormat::Flv
            | ContainerFormat::Avi
            | ContainerFormat::ThreeGp
            | ContainerFormat::Mpg
            | ContainerFormat::F4v
            | ContainerFormat::Asf
            | ContainerFormat::Mxf
            | ContainerFormat::Vob
            | ContainerFormat::Dv
            | ContainerFormat::Nut
            | ContainerFormat::Ivf
            | ContainerFormat::Wav
            | ContainerFormat::Aac
            | ContainerFormat::Aiff
            | ContainerFormat::Wv
            | ContainerFormat::Caf
            | ContainerFormat::Ac3 => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::ThumbnailEmbedStrategy;
    use rdlp_types::ContainerFormat;

    /// Mirrors `rdlp-postprocess`'s `SUPPORTED_CONTAINERS`
    /// (`crates/rdlp-postprocess/src/pipeline/stages/thumbnail.rs`) — the two
    /// lists are independent sources of truth for "can this container carry a
    /// thumbnail" today (full consolidation tracked by #533). The two tests
    /// below are the guardrail that keeps them from silently drifting apart
    /// until that consolidation lands.
    const MIRRORED_SUPPORTED_CONTAINERS: &[&str] = &[
        "mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac", "ogg", "opus",
    ];

    /// Every `ContainerFormat` variant. Kept in step, by hand, with
    /// `for_container`'s own exhaustive match (no catch-all, above) — that
    /// match already fails to compile if a variant is added without a
    /// decision, so a stale list here is a soft test gap, not a silently
    /// wrong answer.
    fn all_container_formats() -> Vec<ContainerFormat> {
        vec![
            ContainerFormat::Mp4,
            ContainerFormat::Mkv,
            ContainerFormat::WebM,
            ContainerFormat::Mov,
            ContainerFormat::M4v,
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
        ]
    }

    /// Forward direction: every string `rdlp-postprocess` advertises as
    /// thumbnail-capable must actually resolve to an embed strategy here.
    #[test]
    fn supported_containers_all_resolve_to_a_strategy() {
        for container in MIRRORED_SUPPORTED_CONTAINERS {
            let format = ContainerFormat::from_str(container)
                .unwrap_or_else(|_| panic!("'{container}' must parse as a ContainerFormat"));
            assert!(
                ThumbnailEmbedStrategy::for_container(format).is_some(),
                "'{container}' is listed in SUPPORTED_CONTAINERS but for_container returned None"
            );
        }
    }

    /// Reverse direction: every container this module resolves to a strategy
    /// for must be advertised by `rdlp-postprocess` — otherwise
    /// `ThumbnailStage` would never reach this code for that container at
    /// all, and the two lists have quietly diverged.
    #[test]
    fn every_strategy_supported_format_is_in_supported_containers() {
        for format in all_container_formats() {
            if ThumbnailEmbedStrategy::for_container(format).is_some() {
                // `as_ext()` (not `Display`/`to_string()`) is the canonical
                // extension string: `ContainerFormat`'s `Display` impl does
                // not necessarily print the same alias `SUPPORTED_CONTAINERS`
                // uses (e.g. `Mkv`'s `Display` is "matroska", not "mkv").
                let name = format.as_ext();
                assert!(
                    MIRRORED_SUPPORTED_CONTAINERS.contains(&name),
                    "'{name}' resolves to a thumbnail strategy but is missing from SUPPORTED_CONTAINERS"
                );
            }
        }
    }

    #[test]
    fn matroska_containers_use_native_attachment() {
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Mkv),
            Some(ThumbnailEmbedStrategy::MatroskaAttachment)
        );
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Mka),
            Some(ThumbnailEmbedStrategy::MatroskaAttachment)
        );
    }

    #[test]
    fn mp3_uses_id3_apic() {
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Mp3),
            Some(ThumbnailEmbedStrategy::Id3Apic)
        );
    }

    #[test]
    fn flac_uses_attached_pic_before_header() {
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Flac),
            Some(ThumbnailEmbedStrategy::FlacAttachedPic)
        );
    }

    /// The regression this module exists to prevent: Ogg and Opus must NOT
    /// resolve to any `ATTACHED_PIC`-stream strategy (#531).
    #[test]
    fn ogg_and_opus_use_vorbis_comment_not_a_stream() {
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Ogg),
            Some(ThumbnailEmbedStrategy::VorbisComment)
        );
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Opus),
            Some(ThumbnailEmbedStrategy::VorbisComment)
        );
    }

    #[test]
    fn mp4_family_uses_attached_pic_with_faststart() {
        for format in [
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::M4v,
            ContainerFormat::M4a,
        ] {
            assert_eq!(
                ThumbnailEmbedStrategy::for_container(format),
                Some(ThumbnailEmbedStrategy::Mp4FamilyAttachedPic)
            );
        }
    }

    /// Negative: a container `rdlp` never advertises as thumbnail-capable
    /// (`SUPPORTED_CONTAINERS` in `rdlp-postprocess`) must resolve to `None`,
    /// not silently fall through to a stream-based strategy that would fail
    /// at mux time.
    #[test]
    fn unsupported_container_resolves_to_none() {
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::WebM),
            None
        );
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Wav),
            None
        );
        assert_eq!(
            ThumbnailEmbedStrategy::for_container(ContainerFormat::Avi),
            None
        );
    }
}
