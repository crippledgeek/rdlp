//! The vocabulary markers that distinguish one [`MediaName`](super::MediaName)
//! from another.
//!
//! Each is a zero-sized type carrying no data — its whole job is to make
//! `MediaName<Codec>` and `MediaName<VideoEncoder>` different types to the
//! compiler. The documentation on each marker is the authority on what its
//! vocabulary means and why it is not interchangeable with the others.

use super::{NameKind, sealed::Sealed};

/// Declares a vocabulary marker: a zero-sized type plus its `NameKind` impl.
///
/// This generates only the marker boilerplate — the sealed impl and the
/// concept string. The actual name behaviour lives once on the generic
/// [`MediaName`](super::MediaName), so nothing behavioural is duplicated here.
macro_rules! declare_kinds {
    ($($(#[$meta:meta])* $name:ident => $concept:literal),+ $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct $name;

            impl Sealed for $name {}

            impl NameKind for $name {
                const CONCEPT: &'static str = $concept;
            }
        )+
    };
}

declare_kinds! {
    /// An `FFmpeg` codec descriptor name, as `probe` reports it — `"h264"`,
    /// `"hevc"`, `"vp9"`, `"aac"`, `"pcm_s16le"`.
    ///
    /// This is the name `avcodec_get_name` produces and
    /// `avcodec_descriptor_get_by_name` round-trips, and it is what
    /// `muxer_can_represent` asks about. Lookup against `FFmpeg`'s descriptor
    /// table is **exact and case-sensitive**, so this kind deliberately does
    /// not case-fold: `"H264"` and `"h264"` are different values, matching the
    /// behaviour the muxer predicate documents.
    ///
    /// Distinct from [`AudioEncoder`] / [`VideoEncoder`]: those name an
    /// *encoder to invoke*, and the sets overlap without being equal — `aac` is
    /// both a codec and an encoder, `libfdk_aac` is only an encoder.
    Codec => "codec name",

    /// The name of an audio encoder to invoke — `"aac"`, `"libfdk_aac"`,
    /// `"libopus"`, `"libmp3lame"`.
    ///
    /// This is what `encoder::find_by_name` resolves, and what the audio
    /// preference registry selects. Whether a given name exists depends on how
    /// the linked `FFmpeg` build was configured, which is why availability is a
    /// runtime lookup rather than a property of this kind.
    ///
    /// Kept separate from [`Codec`] because the two sets overlap without being
    /// equal: `aac` names both a codec and an encoder, but `libfdk_aac` names
    /// only an encoder. The audio codec table's doc comment already had to
    /// explain in prose that its entries mean the *literal* encoder rather than
    /// the preference-resolved one — a distinction this kind carries instead.
    AudioEncoder => "audio encoder name",

    /// The name of a video encoder to invoke — `"libx264"`, `"libx265"`,
    /// `"libsvtav1"`, `"libvvenc"`.
    ///
    /// The video counterpart of [`AudioEncoder`]. Separate from it because the
    /// two registries are separate and an audio encoder is never a valid answer
    /// to a video encoder question; `speed_controls_def` and
    /// `default_bitrate_for_encoder` are keyed to one media kind each.
    VideoEncoder => "video encoder name",

    /// A codec identifier as it appears in an HLS `CODECS=` attribute or a DASH
    /// `codecs=` parameter — `"avc1.640028"`, `"mp4a.40.2"`, `"hev1.1.6.L93.B0"`.
    ///
    /// # A different alphabet, not a different spelling
    ///
    /// These are RFC 6381 identifiers, **not** `FFmpeg` descriptor names.
    /// `"avc1.640028"` describes the same stream as [`Codec`] `"h264"`, but no
    /// `FFmpeg` lookup will resolve it — so handing one to
    /// `muxer_can_represent` returns `false` for the wrong reason: it reads as
    /// "the container refuses this codec" when it actually means "this was
    /// never a name the descriptor table could answer about".
    ///
    /// Both vocabularies were `Option<String>` before #642, so nothing stopped
    /// an extractor's manifest value from reaching a muxer predicate. The
    /// separate kind makes that mistake a compile error.
    ///
    /// Converting between the two is a real operation (parsing the profile
    /// bytes out of `avc1.640028`), not a cast — hence no `From` impl in either
    /// direction.
    Rfc6381 => "RFC 6381 codec",
}
