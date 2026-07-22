//! What the linked `FFmpeg` build's muxers declare as their default codecs.
//!
//! `AVOutputFormat` carries `audio_codec` / `video_codec` fields — the muxer
//! author's own answer to "what belongs in this container by default". Reading
//! them here means rdlp does not keep a second copy of `FFmpeg`'s table that can
//! drift from whichever build is actually linked.

use std::ffi::{CStr, CString};

use ffmpeg_the_third::ffi::{
    AVCodecID, AVMediaType, av_guess_codec, av_guess_format, avcodec_get_name,
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
/// The probe filename is built from [`ContainerFormat::as_ext`] because that is
/// the projection this code intends to probe with — the actual file extension,
/// not an arbitrary alias. `Display` happens to equal `as_ext()` for every
/// variant today (`rdlp-types` pins `to_string` to the extension as of #545,
/// verified by `test_display_is_the_file_extension`), but that equality is an
/// `rdlp-types` invariant, not one this module owns; keep using `as_ext()` so
/// this code stays correct even if that invariant is ever relaxed.
///
/// `av_guess_format` scores every registered muxer against the probe name and
/// returns the first strict-maximum match, so when more than one muxer claims
/// the same extension the winner is decided by `FFmpeg`'s internal registration
/// order, not by this code — worth knowing if a declared default ever looks
/// surprising.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn declared_codec(container: ContainerFormat, kind: MediaKind) -> Option<&'static str> {
    let probe = CString::new(format!("x.{}", container.as_ext())).ok()?;

    // SAFETY: `av_guess_format` and `av_guess_codec` are pure lookups over
    // FFmpeg's static muxer tables; `probe` outlives both calls. `ofmt` (when
    // non-null) points into FFmpeg's static muxer registry, never freed, and
    // is null-checked below before `av_guess_codec` dereferences `fmt->name`
    // internally. The returned name pointer is a 'static string literal owned
    // by FFmpeg.
    unsafe {
        let ofmt = av_guess_format(std::ptr::null(), probe.as_ptr(), std::ptr::null());
        if ofmt.is_null() {
            return None;
        }

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
                "{container} muxer declared {kind} codec id {}, but the \
                 linked libavcodec has no name for it (descriptor table is \
                 behind the muxer table) — reporting no declared {kind} codec",
                id as u32
            );
            return None;
        }
        Some(name)
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
