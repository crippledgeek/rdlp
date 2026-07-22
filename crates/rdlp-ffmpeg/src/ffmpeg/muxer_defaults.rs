//! What the linked `FFmpeg` build's muxers declare as their default codecs.
//!
//! `AVOutputFormat` carries `audio_codec` / `video_codec` fields — the muxer
//! author's own answer to "what belongs in this container by default". Reading
//! them here means rdlp does not keep a second copy of `FFmpeg`'s table that can
//! drift from whichever build is actually linked.

use std::ffi::{CStr, CString};

use ffmpeg_the_third::ffi::{
    AVCodecID, AVMediaType, av_guess_codec, av_guess_format, avcodec_descriptor_get_by_name,
    avcodec_get_name,
};
use rdlp_types::ContainerFormat;

use super::codec_registry::MediaKind;

impl MediaKind {
    const fn as_av(self) -> AVMediaType {
        match self {
            Self::Audio => AVMediaType::AVMEDIA_TYPE_AUDIO,
            Self::Video => AVMediaType::AVMEDIA_TYPE_VIDEO,
        }
    }
}

/// The muxer the linked build picks for `container`'s extension, together with
/// the probe filename it was picked with (`av_guess_codec` needs the same name
/// to score against).
///
/// The probe filename is built from [`ContainerFormat::as_ext`] because that is
/// the projection this module intends to probe with — the actual file
/// extension, not an arbitrary alias. `Display` happens to equal `as_ext()` for
/// every variant today (`rdlp-types` pins `to_string` to the extension as of
/// #545, verified by `test_display_is_the_file_extension`), but that equality is
/// an `rdlp-types` invariant, not one this module owns; keep using `as_ext()` so
/// this code stays correct even if that invariant is ever relaxed.
///
/// `av_guess_format` scores every registered muxer against the probe name and
/// returns the first strict-maximum match, so when more than one muxer claims
/// the same extension the winner is decided by `FFmpeg`'s internal registration
/// order, not by this code — worth knowing if an answer here ever looks
/// surprising. `.mka` is the live example: it resolves to matroska's *audio*
/// muxer, which shares the video muxer's `name` ("matroska") but declares no
/// video codec and rejects every video codec id.
///
/// Returns `None` when the extension embeds a NUL or no muxer claims it. The
/// returned pointer is into `FFmpeg`'s static muxer registry (never freed) and is
/// guaranteed non-null.
fn guess_muxer(
    container: ContainerFormat,
) -> Option<(*const ffmpeg_the_third::ffi::AVOutputFormat, CString)> {
    let probe = CString::new(format!("x.{}", container.as_ext())).ok()?;

    // SAFETY: `av_guess_format` is a pure lookup over FFmpeg's static muxer
    // registry; `probe` outlives the call.
    let ofmt = unsafe { av_guess_format(std::ptr::null(), probe.as_ptr(), std::ptr::null()) };

    (!ofmt.is_null()).then_some((ofmt, probe))
}

/// The codec the linked build's muxer declares as its default for `container`.
///
/// Returns `None` when the container carries no stream of that kind at all
/// (e.g. `Ivf` has no audio), when no muxer claims the extension, or when the
/// muxer declares a codec id the linked `libavcodec` cannot name (its
/// descriptor table is older than the id the muxer produced — an ABI-skew
/// case, logged as a warning when it happens).
///
/// `None` is therefore not proof the container carries no stream of that
/// kind: the third cause above means a container that genuinely has a codec
/// of that kind can still resolve to `None` here. Callers that need to
/// distinguish "no slot of this kind" from "slot present but unrepresentable
/// in this build" must not treat `None` as equivalent to the first two
/// causes.
///
/// Which muxer answers for `container` (and why that can surprise you) is
/// [`guess_muxer`]'s contract.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn declared_codec(container: ContainerFormat, kind: MediaKind) -> Option<&'static str> {
    let (ofmt, probe) = guess_muxer(container)?;

    // SAFETY: `av_guess_codec` is a pure lookup over FFmpeg's static muxer
    // tables; `probe` outlives the call, and `ofmt` was null-checked by
    // `guess_muxer` before being returned. The name pointer `avcodec_get_name`
    // returns is a 'static string literal owned by FFmpeg.
    unsafe {
        let id = av_guess_codec(
            ofmt,
            std::ptr::null(),
            probe.as_ptr(),
            std::ptr::null(),
            kind.as_av(),
        );

        if id == AVCodecID::AV_CODEC_ID_NONE {
            return None;
        }

        // `avcodec_get_name` is documented to return a static, NUL-terminated
        // string and never NULL — it falls back to the literal
        // "unknown_codec" for any id absent from its descriptor table (see
        // `FFmpegRunner::unrepresentable_codec_error` in
        // `ffi_helpers/mod.rs` for the same guarantee cited against the same
        // C API). The null check below is belt-and-braces; the sentinel is
        // what we actually reject here.
        let name = avcodec_get_name(id);
        if name.is_null() {
            return None;
        }
        let name = CStr::from_ptr(name).to_str().ok()?;
        if name == "unknown_codec" {
            log::warn!(
                "{} muxer declared {kind} codec id {}, but the \
                 linked libavcodec has no name for it (descriptor table is \
                 behind the muxer table) — reporting no declared {kind} codec",
                container.as_ext(),
                id as u32
            );
            return None;
        }
        Some(name)
    }
}

/// Whether `container`'s muxer can represent a stream of `codec` — i.e.
/// whether a stream *copy* into that container is a question `FFmpeg` answers
/// yes to, rather than one it refuses mid-mux.
///
/// `codec` is a codec name as [`crate::ffmpeg::probe`] reports it (`FFmpeg`'s own
/// descriptor name — `"h264"`, `"hevc"`, `"vp9"`), which is what
/// `avcodec_descriptor_get_by_name` round-trips: probe derives it from
/// `avcodec_get_name`, the inverse of this lookup. A name no descriptor claims
/// answers `false` — nothing is proven, and an unproven stream copy is exactly
/// what #630 was about. Lookup is exact and case-sensitive (`"H264"` resolves
/// to nothing).
///
/// `kind` is not redundant with `codec`, and dropping it is a live bug, not a
/// tidiness question. Codec *aliases* are not descriptor names, and one of
/// them collides across media: `"avc"` — the alias the old hand-written
/// `can_remux` list used for H.264 — is a real descriptor, but it is
/// `AV_CODEC_ID_ON2AVC`, an **audio** codec. Without the `kind` check,
/// `muxer_can_represent(Mkv, "avc")` answers `true` because Matroska carries
/// On2 Audio — a video routing decision settled by an audio codec's
/// representability. The descriptor's own media type must match what the
/// caller is asking about.
///
/// The decision rule itself is [`crate::ffmpeg::ffi_helpers::oformat_can_represent`],
/// shared with the enforcement point inside the mux so routing and enforcement
/// cannot drift apart. See that function for why the answer is tri-state and
/// why "unknown" counts as "no".
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn muxer_can_represent(container: ContainerFormat, codec: &str, kind: MediaKind) -> bool {
    let Some((ofmt, _probe)) = guess_muxer(container) else {
        return false;
    };
    let Ok(codec_name) = CString::new(codec) else {
        return false;
    };

    // SAFETY: `avcodec_descriptor_get_by_name` is a pure lookup over FFmpeg's
    // static descriptor table and `codec_name` outlives the call; the returned
    // pointer is null-checked before its fields are read. `ofmt` is non-null by
    // `guess_muxer`'s contract.
    unsafe {
        let desc = avcodec_descriptor_get_by_name(codec_name.as_ptr());
        if desc.is_null() || (*desc).type_ != kind.as_av() {
            return false;
        }

        super::ffi_helpers::oformat_can_represent(ofmt, (*desc).id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp3_container_declares_mp3_not_aac() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            declared_codec(ContainerFormat::Mp3, MediaKind::Audio),
            Some("mp3")
        );
    }

    /// `muxer_can_represent` answers from a codec-tag-table hit.
    #[test]
    fn represents_via_the_codec_tag_table() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(muxer_can_represent(
            ContainerFormat::Avi,
            "h264",
            MediaKind::Video
        ));
        assert!(muxer_can_represent(
            ContainerFormat::Nut,
            "hevc",
            MediaKind::Video
        ));
    }

    /// …and from a positive `query_codec` answer with no tag-table entry.
    /// `matroskaenc` defines `mkv_query_codec`, which reports `hevc`
    /// representable even though matroska's tag table (keyed off `CodecID`
    /// strings, not fourccs) has no entry for it — so a tag-table-only
    /// predicate would wrongly refuse the single most common remux rdlp does.
    #[test]
    fn represents_via_query_codec_without_a_tag_entry() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(muxer_can_represent(
            ContainerFormat::Mkv,
            "hevc",
            MediaKind::Video
        ));
        assert!(muxer_can_represent(
            ContainerFormat::WebM,
            "vp9",
            MediaKind::Video
        ));
    }

    /// An explicit `0` from the tag-table fallback: `avienc` defines no
    /// `query_codec` and AVI's table has no `hevc` entry.
    #[test]
    fn refuses_what_the_muxer_reports_unsupported() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(!muxer_can_represent(
            ContainerFormat::Avi,
            "hevc",
            MediaKind::Video
        ));
        assert!(!muxer_can_represent(
            ContainerFormat::WebM,
            "h264",
            MediaKind::Video
        ));
    }

    /// `avformat_query_codec`'s *third* branch, which the first version of
    /// this file's docs missed: with no `query_codec` callback and no tag
    /// table, a muxer still answers `1` for the codecs it **declares**
    /// (`mux_utils.c`: `codec_id == ofmt->video_codec || audio_codec ||
    /// subtitle_codec`). `ff_mxf_muxer` declares `mpeg2video` and
    /// `pcm_s16le`, so mxf is not the uniform "always transcode" container
    /// the docs claimed — and this is a genuinely distinct arm from the
    /// tag-table and `query_codec` paths the other tests pin.
    #[test]
    fn a_muxers_own_declared_codec_answers_yes() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(muxer_can_represent(
            ContainerFormat::Mxf,
            "mpeg2video",
            MediaKind::Video
        ));
        assert!(muxer_can_represent(
            ContainerFormat::Mxf,
            "pcm_s16le",
            MediaKind::Audio
        ));
    }

    /// The negative (`AVERROR_PATCHWELCOME`) arm — `mxfenc` has neither a
    /// `query_codec` callback nor a `codec_tag` table, so every codec *other
    /// than the ones it declares* answers "unknown", and unknown must not be
    /// read as yes.
    #[test]
    fn treats_unknown_representability_as_no() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(!muxer_can_represent(
            ContainerFormat::Mxf,
            "h264",
            MediaKind::Video
        ));
        assert!(!muxer_can_represent(
            ContainerFormat::Mxf,
            "hevc",
            MediaKind::Video
        ));
    }

    /// A name no descriptor claims proves nothing.
    #[test]
    fn refuses_a_codec_name_no_descriptor_claims() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(!muxer_can_represent(
            ContainerFormat::Mkv,
            "",
            MediaKind::Video
        ));
        assert!(!muxer_can_represent(
            ContainerFormat::Mkv,
            "not_a_codec",
            MediaKind::Video
        ));
        // Exact, case-sensitive lookup: FFmpeg's descriptor names are lowercase.
        assert!(!muxer_can_represent(
            ContainerFormat::Mkv,
            "H264",
            MediaKind::Video
        ));
    }

    /// The cross-medium alias trap: `"avc"` is a real descriptor, but it is
    /// `AV_CODEC_ID_ON2AVC` (audio), not H.264. Asked as a *video* question it
    /// must answer `false`; asked as the audio codec it actually is, Matroska
    /// does carry it — which is what makes the `kind` check load-bearing
    /// rather than decorative.
    #[test]
    fn a_codec_name_of_the_wrong_medium_answers_no() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(!muxer_can_represent(
            ContainerFormat::Mkv,
            "avc",
            MediaKind::Video
        ));
        assert!(muxer_can_represent(
            ContainerFormat::Mkv,
            "avc",
            MediaKind::Audio
        ));
    }

    /// `.mka` resolves to matroska's *audio* muxer — same `name` as the video
    /// one, different muxer, no video codec accepted. This is why `Mka` keeps
    /// its unconditional `can_remux` arm pending #577 rather than being
    /// re-routed to a transcode that would happily write video into it.
    #[test]
    fn mka_resolves_to_the_audio_muxer_which_takes_no_video() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(!muxer_can_represent(
            ContainerFormat::Mka,
            "h264",
            MediaKind::Video
        ));
        assert!(muxer_can_represent(
            ContainerFormat::Mkv,
            "h264",
            MediaKind::Video
        ));
    }

    #[test]
    fn flac_declares_flac_and_wav_declares_pcm() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            declared_codec(ContainerFormat::Flac, MediaKind::Audio),
            Some("flac")
        );
        assert_eq!(
            declared_codec(ContainerFormat::Wav, MediaKind::Audio),
            Some("pcm_s16le")
        );
    }

    #[test]
    fn aiff_declares_big_endian_pcm() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            declared_codec(ContainerFormat::Aiff, MediaKind::Audio),
            Some("pcm_s16be"),
            "AIFF is big-endian; pcm_s16le would be wrong"
        );
    }

    #[test]
    fn ivf_declares_no_audio_codec() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            declared_codec(ContainerFormat::Ivf, MediaKind::Audio),
            None,
            "ivf is a raw video elementary stream and carries no audio"
        );
    }

    /// Pins matroska's declared default audio codec. `libavformat/matroskaenc.c`
    /// picks between vorbis and ac3 depending on whether the linked build
    /// enables `CONFIG_LIBVORBIS_ENCODER`, so both are valid outcomes here —
    /// this module intentionally does not keep a second, build-specific copy
    /// of `FFmpeg`'s table (see the module doc).
    #[test]
    fn mkv_declares_vorbis_or_ac3_depending_on_build() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(
            matches!(
                declared_codec(ContainerFormat::Mkv, MediaKind::Audio),
                Some("vorbis" | "ac3")
            ),
            "matroska declares vorbis when CONFIG_LIBVORBIS_ENCODER is on, else ac3"
        );
    }

    /// Only observes which containers resolve NO audio codec at all — either
    /// because no muxer claims the extension, or because the muxer that does
    /// claim it declares no default audio codec. It does not pin the *value*
    /// each container declares; that per-container table is a later task's to
    /// pin (see the individual exact-pin tests above for the containers that
    /// already have one).
    ///
    /// This is a deliberate pin on a full-featured `FFmpeg` build: it asserts
    /// the EXACT set of audio-less containers, so it fails on any build with
    /// muxers disabled (`--disable-muxer=caf`, a `--disable-everything`
    /// distro build, a minimal vendored build) where a container this test
    /// expects to have audio no longer does. If it fires for that reason,
    /// the fix is widening the expected set to match the linked build, not
    /// treating it as a real regression.
    #[test]
    fn every_container_resolves_and_only_ivf_lacks_audio() {
        use strum::IntoEnumIterator;
        crate::ffmpeg::ensure_init().expect("ffmpeg init");

        let without_audio: Vec<ContainerFormat> = ContainerFormat::iter()
            .filter(|c| declared_codec(*c, MediaKind::Audio).is_none())
            .collect();

        assert_eq!(
            without_audio,
            vec![ContainerFormat::Ivf],
            "unexpected set of containers with no declared audio codec"
        );
    }

    #[test]
    fn video_kind_answers_separately() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            declared_codec(ContainerFormat::Dv, MediaKind::Video),
            Some("dvvideo")
        );
        assert_eq!(
            declared_codec(ContainerFormat::Ivf, MediaKind::Video),
            Some("vp8")
        );
    }
}
