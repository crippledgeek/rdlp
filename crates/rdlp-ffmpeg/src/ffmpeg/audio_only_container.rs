//! Which containers rdlp refuses to write a real video stream into, and the
//! container it points the user at instead (#577).
//!
//! rdlp's remux is a stream copy and `FFmpeg` enforces nothing here: the ASF
//! muxer is registered once for `asf,wmv,wma` with no extension-conditioned
//! codec checks, so `-c copy` into a `.wma` writes H.264 and exits 0. The
//! extension then claims audio-only while the bytes disagree — non-conformant
//! per Microsoft's own File Name Extension Guidelines, which give `.wma` to a
//! file with "no supported video streams" and send any video stream to
//! `.wmv`/`.asf`.
//!
//! **The decision is to refuse, not to drop the video stream.** Silently
//! discarding a user's video track is worse than telling them the container
//! is wrong for it, and refusing is what upstream does: yt-dlp's
//! `--remux-video` is a stream copy that hard-fails rather than falling back
//! to a re-encode, and `--recode-video` never emits `-c copy`. Neither verb
//! falls back, so a refusal is the idiomatic behaviour rather than an
//! invention.
//!
//! The module itself is private (`mod audio_only_container;`), so the `pub`
//! on the items below reaches no further than the crate — `pub(crate)` inside
//! a private module is what `clippy::redundant_pub_crate` rejects. Only
//! [`video_alternative_for`] is re-exported to the outside world.
//!
//! # One policy, every entry point
//!
//! Three mux entry points take a caller-chosen target container, and all
//! three ask this module rather than repeating the classification:
//! `remux_sync` (`--remux`, HLS auto-remux, thumbnail auto-remux, fixup),
//! `convert_video_sync` (`--recode-video`/`--recode-container`, whose
//! transcode branch would otherwise *encode* video into the artifact #577
//! forbids), and `merge_sync` (`postprocess.merge_output_format`).
//!
//! # `ATTACHED_PIC` is a correctness discriminator, not a trust boundary
//!
//! The cover-art carve-out reads `AV_DISPOSITION_ATTACHED_PIC`, which is
//! metadata carried by the *input file* and therefore attacker-influenced for
//! a downloaded source. Nothing here depends on it being honest: a source
//! that mislabels real video as an attached picture buys only its own
//! mislabelled `.wma`, which is the pre-#577 behaviour and harms nobody else.
//! The flag is the only discriminator `FFmpeg` offers, and this guard is a
//! correctness/UX policy, not a security control.

use std::path::Path;

use rdlp_types::ContainerFormat;

use crate::error::PostProcessError;

/// The video-capable container to point a user at when their remux target is
/// one rdlp treats as audio-only — `None` for every container rdlp will
/// write a video stream into.
///
/// `Some(_)` for exactly the twelve containers `speed_controls::
/// video_default_for` classifies `Policy::NotATarget` for the video stream
/// kind. (Backticks rather than intra-doc links: `video_default_for` is
/// `pub(crate)` and `Policy::NotATarget` reaches only as far as the
/// `pub(crate) mod container_default`, so a link from this public item trips
/// `rustdoc::private_intra_doc_links` under the doc gate's `-D warnings`.
/// The enclosing `pub mod speed_controls` is public; the linked *items* are
/// not.) That function's doc comment is the policy record — including why
/// those twelve are three materially different situations, and why
/// [`ContainerFormat::Ogg`] is deliberately not among them (Theora is a real
/// video codec). This table does not re-derive the classification; it only
/// adds the remediation, and
/// `refusal_set_matches_the_video_default_policy` below fails if the two ever
/// disagree.
///
/// Exhaustive with no `_` arm, for the same reason `video_default_for` is: a
/// new [`ContainerFormat`] variant must be classified here or the build
/// fails, rather than silently inheriting "video is fine here".
///
/// The remediation is the nearest container a user plausibly meant, which for
/// three of the twelve is a same-family spelling — Microsoft's rule sends
/// `.wma`'s video to `.wmv`; `.m4a`→`.mp4` and `.mka`→`.mkv` are the
/// video-admitting spellings of the same underlying format. Note that "same
/// family" is NOT "same muxer registration": `.m4a` shares `ff_ipod_muxer`
/// with `.m4v`, and `.mp4` is a *separate* registration that merely declares
/// the same codec on this build (`video_default_for`, Item 14); `.mka`
/// likewise resolves to matroska's *audio* muxer, a different muxer from
/// `.mkv`'s under an identical name (`RecodeStage::rule_for` records the same
/// fact). The nine with no video-admitting sibling get
/// [`ContainerFormat::Mkv`], which carries essentially any codec pairing;
/// [`ContainerFormat::Mp4`] would be narrower advice, and for
/// [`ContainerFormat::Opus`] the *family* sibling (`.ogg`) would be actively
/// bad advice, since Ogg's video slot is Theora and the typical payload here
/// is H.264. `.mka` therefore shares an arm with those nine rather than
/// having its own: same answer, two different reasons (see the arm's
/// comment).
#[must_use]
pub const fn video_alternative_for(container: ContainerFormat) -> Option<ContainerFormat> {
    match container {
        // Microsoft's File Name Extension Guidelines, verbatim: `.wma` is for
        // a file with "no supported video streams"; a video stream belongs in
        // `.wmv` (WM codecs) or `.asf` (third-party codecs). `.wmv` is the
        // one to name — it is the sibling registration of the same `asf`
        // muxer, so the remux the user meant is a one-word edit.
        ContainerFormat::Wma => Some(ContainerFormat::Wmv),
        // rdlp declines video at `.m4a` purely by naming convention (see
        // `ContainerFormat`'s m4a/mp4 extension split), so `.mp4` is the
        // ISOBMFF spelling that admits it. Not the same muxer registration —
        // `.m4a` resolves to `ff_ipod_muxer` (extensions "m4v,m4a,m4b"), and
        // `.mp4`'s is separate; both are ISOBMFF and hold the same streams,
        // which is what makes this the right advice.
        ContainerFormat::M4a => Some(ContainerFormat::Mp4),
        // `Mkv` for two different reasons that happen to agree, so clippy's
        // `match_same_arms` (correctly) wants them in one arm: `.mka` is the
        // Matroska *audio* spelling of `.mkv` (a distinct muxer under an
        // identical name, which rejects every video codec), a same-family
        // sibling like the two above; the rest have no video-admitting
        // sibling at all and get Matroska as the general-purpose container
        // that carries essentially any codec pairing.
        ContainerFormat::Mka
        | ContainerFormat::Mp3
        | ContainerFormat::Wav
        | ContainerFormat::Flac
        | ContainerFormat::Opus
        | ContainerFormat::Aac
        | ContainerFormat::Aiff
        | ContainerFormat::Wv
        | ContainerFormat::Caf
        | ContainerFormat::Ac3 => Some(ContainerFormat::Mkv),
        ContainerFormat::Mp4
        | ContainerFormat::Mkv
        | ContainerFormat::WebM
        | ContainerFormat::Mov
        | ContainerFormat::M4v
        | ContainerFormat::Ts
        | ContainerFormat::Flv
        | ContainerFormat::Avi
        | ContainerFormat::ThreeGp
        | ContainerFormat::Mpg
        | ContainerFormat::F4v
        | ContainerFormat::Wmv
        | ContainerFormat::Asf
        | ContainerFormat::Mxf
        | ContainerFormat::Vob
        | ContainerFormat::Dv
        | ContainerFormat::Nut
        | ContainerFormat::Ivf
        | ContainerFormat::Ogg => None,
    }
}

/// The codec name of the first stream that is *real* video — a video-medium
/// stream WITHOUT the `ATTACHED_PIC` disposition — or `None` if `ictx` has
/// none.
///
/// The disposition check is the whole point. Cover art is carried as a
/// video-codec stream (mjpeg/png), and rdlp writes one into audio containers
/// on purpose — `ThumbnailEmbedStrategy`'s `Id3Apic` / `FlacAttachedPic` /
/// `Mp4FamilyAttachedPic` arms all do. `MediaInfo::has_video` cannot tell the
/// two apart (it counts an attached picture as video), so a guard built on it
/// would refuse every thumbnail-bearing audio file.
/// `AV_DISPOSITION_ATTACHED_PIC` is `FFmpeg`'s own flag for the distinction,
/// and the same one `ffi_helpers::set_attached_pic_disposition` sets on the
/// write side. See the module doc on why an input-controlled flag is
/// sufficient here.
fn first_real_video_codec(ictx: &ffmpeg_the_third::format::context::Input) -> Option<String> {
    ictx.streams()
        .find(|ist| {
            ist.parameters().medium() == ffmpeg_the_third::media::Type::Video
                && !ist
                    .disposition()
                    .contains(ffmpeg_the_third::format::stream::Disposition::ATTACHED_PIC)
        })
        .map(|ist| ist.parameters().id().name().to_string())
}

/// The I/O-free half of the guard: `Some((target, alternative))` when
/// `output`'s extension names a container rdlp refuses video for.
///
/// Split out so every caller can answer the cheap question first and open an
/// input only on the path that is about to be refused. `None` for an output
/// whose extension parses to no known container — this guard has no opinion
/// there, and the muxer's own errors remain the backstop.
fn refused_target(output: &Path) -> Option<(ContainerFormat, ContainerFormat)> {
    let target = ContainerFormat::from_path(output)?;
    video_alternative_for(target).map(|alternative| (target, alternative))
}

/// Refuse when `output` is an audio-only target and `ictx` carries a real
/// (non-cover-art) video stream. For a caller that already has its input
/// open.
///
/// # Errors
///
/// [`PostProcessError::AudioOnlyContainerRejectsVideo`], naming the container
/// asked for and the one to use instead.
pub fn reject_video_into_audio_only(
    ictx: &ffmpeg_the_third::format::context::Input,
    output: &Path,
) -> Result<(), PostProcessError> {
    let Some((container, alternative)) = refused_target(output) else {
        return Ok(());
    };
    let Some(codec) = first_real_video_codec(ictx) else {
        return Ok(());
    };
    Err(PostProcessError::AudioOnlyContainerRejectsVideo {
        container,
        codec,
        alternative,
    })
}

/// Same guard, for a caller that has not opened its input yet — the recode
/// path, which must refuse *before* it starts encoding.
///
/// Opens `video_input` only when the target is one of the twelve, so the
/// ordinary path costs no I/O at all. A source that cannot be opened is not
/// this guard's error to report: it returns `Ok(())` and lets the caller's
/// own open produce the real, contextualised failure.
///
/// # Errors
///
/// [`PostProcessError::AudioOnlyContainerRejectsVideo`], as above.
pub fn reject_video_source_into_audio_only(
    video_input: &Path,
    output: &Path,
) -> Result<(), PostProcessError> {
    if refused_target(output).is_none() {
        return Ok(());
    }
    let Ok(ictx) = ffmpeg_the_third::format::input(video_input) else {
        return Ok(());
    };
    reject_video_into_audio_only(&ictx, output)
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use super::{ContainerFormat, video_alternative_for};
    use crate::ffmpeg::container_default::Policy;
    use crate::ffmpeg::speed_controls::video_default_for;

    /// The drift guard this module's doc comment promises: the refusal set is
    /// *the* `Policy::NotATarget` set, not a second opinion about it. Both
    /// matches are exhaustive, so a new `ContainerFormat` variant cannot
    /// reach either without a decision — but nothing except this test stops
    /// the two decisions from disagreeing.
    #[test]
    fn refusal_set_matches_the_video_default_policy() {
        let mut checked = 0_usize;
        for container in ContainerFormat::iter() {
            let refused = video_alternative_for(container).is_some();
            let not_a_target = matches!(video_default_for(container).policy(), Policy::NotATarget);
            assert_eq!(
                refused, not_a_target,
                "{container}: refusal set and video_default_for disagree \
                 (refused={refused}, NotATarget={not_a_target})"
            );
            checked += 1;
        }
        assert_eq!(
            checked,
            ContainerFormat::iter().count(),
            "the loop must visit every ContainerFormat variant"
        );
    }

    /// The count is pinned separately from the equivalence above so that
    /// reclassifying a container in *both* tables at once — which the
    /// equivalence test would wave through — still trips something.
    #[test]
    fn exactly_twelve_containers_are_refused() {
        let refused: Vec<ContainerFormat> = ContainerFormat::iter()
            .filter(|c| video_alternative_for(*c).is_some())
            .collect();
        assert_eq!(refused.len(), 12, "refused set changed: {refused:?}");
    }

    /// The issue's own example: `--remux=wma` on a video source points at
    /// `wmv`, per Microsoft's extension guidelines.
    #[test]
    fn wma_points_at_wmv() {
        assert_eq!(
            video_alternative_for(ContainerFormat::Wma),
            Some(ContainerFormat::Wmv)
        );
    }

    /// `Ogg` is dual-purpose (Vorbis audio, Theora video) and must NOT be
    /// refused — the classification `is_audio_only()` drifted into before it
    /// was deleted (#618).
    #[test]
    fn ogg_is_not_refused() {
        assert_eq!(video_alternative_for(ContainerFormat::Ogg), None);
    }

    /// Every suggestion must itself accept video, or the error sends the user
    /// straight into the same refusal.
    #[test]
    fn every_suggestion_is_itself_video_capable() {
        for container in ContainerFormat::iter() {
            if let Some(alternative) = video_alternative_for(container) {
                assert_eq!(
                    video_alternative_for(alternative),
                    None,
                    "{container} points at {alternative}, which is itself refused"
                );
            }
        }
    }
}
