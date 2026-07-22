//! [`AudioEncoderName`] / [`VideoEncoderName`] — an encoder to *invoke*.

use super::define_media_name;

define_media_name! {
    /// The name of an audio encoder to invoke — `"aac"`, `"libfdk_aac"`,
    /// `"libopus"`, `"libmp3lame"`.
    ///
    /// This is what `encoder::find_by_name` resolves, and what the audio
    /// preference registry selects. Whether a given name exists depends on how
    /// the linked `FFmpeg` build was configured, which is why availability is a
    /// runtime lookup rather than a property of this type.
    ///
    /// Kept separate from [`CodecName`](super::CodecName) because the two sets
    /// overlap without being equal: `aac` names both a codec and an encoder,
    /// but `libfdk_aac` names only an encoder. The audio codec table's doc
    /// comment already had to explain in prose that its entries mean the
    /// *literal* encoder rather than the preference-resolved one — a
    /// distinction this type carries instead.
    AudioEncoderName
}

define_media_name! {
    /// The name of a video encoder to invoke — `"libx264"`, `"libx265"`,
    /// `"libsvtav1"`, `"libvvenc"`.
    ///
    /// The video counterpart of [`AudioEncoderName`]. Separate from it because
    /// the two registries are separate and an audio encoder is never a valid
    /// answer to a video encoder question; `speed_controls_def` and
    /// `default_bitrate_for_encoder` are keyed to one media kind each.
    VideoEncoderName
}
