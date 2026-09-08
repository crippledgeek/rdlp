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

use rdlp_types::ContainerFormat;

/// The video-capable container to point a user at when their remux target is
/// one rdlp treats as audio-only — `None` for every container rdlp will
/// write a video stream into.
///
/// `Some(_)` for exactly the twelve containers `speed_controls::
/// video_default_for` classifies `Policy::NotATarget` for the video stream
/// kind. (Backticks rather than intra-doc links: both are crate-private, and
/// `#![warn(missing_docs)]` + `-D warnings` makes a public item linking to a
/// private one a build error.) That function's doc comment is the policy
/// record —
/// including why those twelve are three materially different situations, and
/// why [`ContainerFormat::Ogg`] is deliberately not among them (Theora is a
/// real video codec). This table does not re-derive the classification; it
/// only adds the remediation, and
/// `refusal_set_matches_the_video_default_policy` below fails if the two ever
/// disagree.
///
/// Exhaustive with no `_` arm, for the same reason `video_default_for` is: a
/// new [`ContainerFormat`] variant must be classified here or the build
/// fails, rather than silently inheriting "video is fine here".
///
/// The remediation is a family sibling where one exists — Microsoft's rule
/// sends `.wma`'s video to `.wmv`; `.m4a`/`.mka` are the audio spellings of
/// `.mp4`/`.mkv` and share their muxer registration. The nine with no video
/// sibling get [`ContainerFormat::Mkv`], which carries essentially any codec
/// pairing; [`ContainerFormat::Mp4`] would be narrower advice, and for
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
        // Shares the `ipod` muxer registration with `M4v`/`Mp4`; rdlp
        // declines video at `.m4a` purely by naming convention (see
        // `ContainerFormat`'s m4a/mp4 extension split), so `.mp4` is the same
        // bytes under the extension that admits them.
        ContainerFormat::M4a => Some(ContainerFormat::Mp4),
        // `Mkv` for two different reasons that happen to agree, so clippy's
        // `match_same_arms` (correctly) wants them in one arm: `.mka` is the
        // Matroska *audio* spelling of `.mkv`, a family sibling like the two
        // above; the rest have no video sibling at all and get Matroska as
        // the general-purpose container that carries essentially any codec
        // pairing.
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
