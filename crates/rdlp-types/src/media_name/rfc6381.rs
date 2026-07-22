//! [`Rfc6381Codec`] — the codec vocabulary of HLS and DASH manifests.

use super::define_media_name;

define_media_name! {
    /// A codec identifier as it appears in an HLS `CODECS=` attribute or a DASH
    /// `codecs=` parameter — `"avc1.640028"`, `"mp4a.40.2"`, `"hev1.1.6.L93.B0"`.
    ///
    /// # A different alphabet, not a different spelling
    ///
    /// These are RFC 6381 identifiers, **not** `FFmpeg` descriptor names.
    /// `"avc1.640028"` describes the same stream as [`CodecName`](super::CodecName)
    /// `"h264"`, but no `FFmpeg` lookup will resolve it — so handing one to
    /// `muxer_can_represent` returns `false` for the wrong reason: it reads as
    /// "the container refuses this codec" when it actually means "this was
    /// never a name the descriptor table could answer about".
    ///
    /// Both vocabularies were `Option<String>` before #642, so nothing stopped
    /// an extractor's manifest value from reaching a muxer predicate. The
    /// separate type makes that mistake a compile error.
    ///
    /// Converting between the two is a real operation (parsing the profile
    /// bytes out of `avc1.640028`), not a cast — hence no `From` impl in either
    /// direction.
    Rfc6381Codec
}
