//! Shared core for the audio and video encoder preference registries.
//!
//! Both registries hold a static table of codecs, each with an ordered
//! encoder-preference list, and answer the same four questions against it.
//! Those answers live here once; the per-media modules keep only their table
//! and the `*Info` types they build, which genuinely differ.

use std::collections::HashMap;
use std::sync::OnceLock;

use log::info;

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

/// Finds the row for a codec name, case-insensitively.
#[must_use]
pub fn find_row<R: CodecRow>(table: &'static [R], codec: &str) -> Option<&'static R> {
    table
        .iter()
        .find(|row| row.codec().eq_ignore_ascii_case(codec))
}

/// Best available encoder for a codec name, memoised in `cache`.
///
/// `cache` is a parameter, not a `static` inside this function: a static in a
/// generic scope is NOT monomorphized, so an inner static would be shared by
/// every registry that calls this. `label` distinguishes the registries in the
/// log line (e.g. `"audio"` / `"video"`).
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn preferred_encoder<R: CodecRow>(
    table: &'static [R],
    cache: &'static OnceLock<HashMap<&'static str, Option<&'static str>>>,
    codec: &str,
    label: &str,
) -> Option<&'static str> {
    let map = cache.get_or_init(|| {
        let mut map = HashMap::new();
        for row in table {
            let selected = row
                .encoders()
                .iter()
                .find(|(enc, _)| is_encoder_available(enc))
                .map(|(enc, _)| *enc);

            if let Some(enc) = selected {
                info!(
                    "Using {enc} as {codec} {label} encoder",
                    codec = row.codec()
                );
            }

            map.insert(row.codec(), selected);
        }
        map
    });

    map.get(codec.to_ascii_lowercase().as_str())
        .copied()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::OnceLock;

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
        let encoder = row.encoders().first().expect("encoders has one entry");
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

    static FAKE_CACHE_A: OnceLock<HashMap<&'static str, Option<&'static str>>> = OnceLock::new();
    static FAKE_CACHE_B: OnceLock<HashMap<&'static str, Option<&'static str>>> = OnceLock::new();

    static TABLE_A: &[FakeRow] = &[FakeRow {
        codec: "shared_name",
        encoders: &[("aac", "AAC")],
    }];
    static TABLE_B: &[FakeRow] = &[FakeRow {
        codec: "shared_name",
        encoders: &[("pcm_s16le", "PCM")],
    }];

    #[test]
    fn preferred_encoder_returns_first_available() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            preferred_encoder(TABLE_A, &FAKE_CACHE_A, "shared_name", "test-a"),
            Some("aac")
        );
    }

    #[test]
    fn preferred_encoder_unknown_codec_is_none() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            preferred_encoder(TABLE_A, &FAKE_CACHE_A, "no_such_codec", "test-a"),
            None
        );
    }

    /// REGRESSION GUARD for the generic-static trap. Two tables share a codec
    /// name but have different encoders. With one shared cache, whichever ran
    /// first would win for both. Distinct caches must give distinct answers.
    #[test]
    fn distinct_caches_do_not_share_entries() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let a = preferred_encoder(TABLE_A, &FAKE_CACHE_A, "shared_name", "test-a");
        let b = preferred_encoder(TABLE_B, &FAKE_CACHE_B, "shared_name", "test-b");
        assert_eq!(a, Some("aac"), "table A must resolve from its own table");
        assert_eq!(b, Some("pcm_s16le"), "table B must NOT inherit A's cache");
        assert_ne!(a, b, "caches leaked across registries");
    }

    #[test]
    fn find_row_matches_case_insensitively() {
        assert!(find_row(TABLE_A, "SHARED_NAME").is_some());
        assert!(find_row(TABLE_A, "nope").is_none());
    }
}
