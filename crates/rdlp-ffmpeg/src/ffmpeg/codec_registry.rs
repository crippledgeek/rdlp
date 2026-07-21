//! Shared core for the audio and video encoder preference registries.
//!
//! Both registries hold a static table of codecs, each with an ordered
//! encoder-preference list, and answer the same four questions against it.
//! Those answers live here once; the per-media modules keep only their table
//! and the `*Info` types they build, which genuinely differ.

/// The minimal view of a preference-table row the shared lookups need.
///
/// Deliberately two methods: everything else about a row (audio's
/// `supported_containers`, video's speed-control derivation) is media-specific
/// and stays in the owning module.
pub trait CodecRow {
    /// Canonical codec name, e.g. `"aac"` / `"h264"`.
    fn codec(&self) -> &'static str;
    /// Ordered encoder preference list: `(encoder_name, display_name)`.
    fn encoders(&self) -> &'static [(&'static str, &'static str)];
}

/// Returns `true` if the named encoder is present in the linked `FFmpeg` build.
///
/// Identical for audio and video — this is the single definition. Requires
/// [`super::ensure_init`] to have been called first.
#[must_use]
pub fn is_encoder_available(encoder: &str) -> bool {
    ffmpeg_the_third::codec::encoder::find_by_name(encoder).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRow {
        codec: &'static str,
        encoders: &'static [(&'static str, &'static str)],
    }

    impl CodecRow for FakeRow {
        fn codec(&self) -> &'static str {
            self.codec
        }
        fn encoders(&self) -> &'static [(&'static str, &'static str)] {
            self.encoders
        }
    }

    static FAKE_TABLE: &[FakeRow] = &[FakeRow {
        codec: "fakecodec",
        encoders: &[("nonexistent_encoder_xyz", "Fake")],
    }];

    #[test]
    fn codec_row_exposes_codec_and_encoders() {
        let row = FAKE_TABLE.first().expect("fake table has one row");
        assert_eq!(row.codec(), "fakecodec");
        assert_eq!(row.encoders().len(), 1);
        let encoder = row
            .encoders()
            .first()
            .expect("encoders has one entry");
        assert_eq!(encoder.0, "nonexistent_encoder_xyz");
    }

    #[test]
    fn is_encoder_available_false_for_nonsense_name() {
        assert!(!is_encoder_available("nonexistent_encoder_xyz"));
    }

    #[test]
    fn is_encoder_available_true_for_builtin_aac() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(is_encoder_available("aac"));
    }
}
