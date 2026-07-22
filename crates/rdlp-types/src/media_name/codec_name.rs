//! [`CodecName`] — what a decoded stream *is*.

use super::define_media_name;

define_media_name! {
    /// An `FFmpeg` codec descriptor name, as `probe` reports it — `"h264"`,
    /// `"hevc"`, `"vp9"`, `"aac"`, `"pcm_s16le"`.
    ///
    /// This is the name `avcodec_get_name` produces and
    /// `avcodec_descriptor_get_by_name` round-trips, and it is what
    /// `muxer_can_represent` asks about. Lookup against `FFmpeg`'s descriptor
    /// table is **exact and case-sensitive**, so this type deliberately does
    /// not case-fold: `"H264"` and `"h264"` are different values, matching the
    /// behaviour the muxer predicate documents.
    ///
    /// Distinct from [`AudioEncoderName`](super::AudioEncoderName) /
    /// [`VideoEncoderName`](super::VideoEncoderName): those name an *encoder to
    /// invoke*, and the sets overlap without being equal — `aac` is both a
    /// codec and an encoder, `libfdk_aac` is only an encoder.
    CodecName
}
