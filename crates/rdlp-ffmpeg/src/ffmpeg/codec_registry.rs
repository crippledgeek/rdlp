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
/// The trait carries only what the shared lookups need. `display_name` is
/// common to both row types too, but is read directly by each module's own
/// `list_available_*`, which construct different `*Info` shapes and therefore
/// remain per-media; audio's `supported_containers` and video's speed-control
/// derivation are genuinely media-specific and stay in the owning module.
pub trait CodecRow {
    /// Canonical codec name, e.g. `"aac"` / `"h264"`.
    fn codec(&self) -> &'static str;
    /// Ordered encoder preference list: `(encoder_name, display_name)`.
    fn encoders(&self) -> &'static [(&'static str, &'static str)];
}

/// Which media a registry serves. Also supplies the word used in the
/// selection log line.
#[derive(Debug, Clone, Copy)]
pub enum MediaKind {
    /// An audio codec/encoder registry.
    Audio,
    /// A video codec/encoder registry.
    Video,
}

impl std::fmt::Display for MediaKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Audio => "audio",
            Self::Video => "video",
        })
    }
}

/// A codec preference table with its own memoisation cache.
///
/// The cache is owned by the `Registry`, not borrowed from a module-level
/// `static`: there is no longer a second value that could be mismatched
/// against the table, so passing one registry's table with another
/// registry's cache is not a shape the type system can even express.
pub struct Registry<R: CodecRow + 'static> {
    table: &'static [R],
    cache: OnceLock<HashMap<&'static str, Option<&'static str>>>,
    kind: MediaKind,
}

impl<R: CodecRow + 'static> Registry<R> {
    /// Binds a preference table to a fresh cache and declares which media it serves.
    #[must_use]
    pub const fn new(table: &'static [R], kind: MediaKind) -> Self {
        Self {
            table,
            cache: OnceLock::new(),
            kind,
        }
    }

    /// Finds the row for a codec name, case-insensitively.
    #[must_use]
    pub fn find_row(&self, codec: &str) -> Option<&'static R> {
        self.table
            .iter()
            .find(|row| row.codec().eq_ignore_ascii_case(codec))
    }

    /// Best available encoder for a codec name, memoised in this registry's cache.
    ///
    /// Requires [`super::ensure_init`] to have been called first.
    #[must_use]
    pub fn preferred_encoder(&self, codec: &str) -> Option<&'static str> {
        self.lookup_preferred(&codec.to_ascii_lowercase())
    }

    /// As [`Self::preferred_encoder`], but `lower` must already be lowercase.
    ///
    /// Lets [`Self::resolve`] lowercase its input once and reuse the same
    /// lookup, instead of lowercasing again on the way in here.
    fn lookup_preferred(&self, lower: &str) -> Option<&'static str> {
        let map = self.cache.get_or_init(|| {
            let mut map = HashMap::new();
            for row in self.table {
                let selected = row
                    .encoders()
                    .iter()
                    .find(|(enc, _)| is_encoder_available(enc))
                    .map(|(enc, _)| *enc);

                if let Some(enc) = selected {
                    info!(
                        "Using {enc} as {codec} {kind} encoder",
                        codec = row.codec(),
                        kind = self.kind
                    );
                }

                map.insert(row.codec(), selected);
            }
            map
        });

        map.get(lower).copied().flatten()
    }

    /// Resolves either a codec name or a direct encoder name to an available encoder.
    ///
    /// Codec names go through [`Self::preferred_encoder`]; anything else is
    /// matched against the table's encoder names and then gated on availability.
    ///
    /// Requires [`super::ensure_init`] to have been called first.
    #[must_use]
    pub fn resolve(&self, input: &str) -> Option<&'static str> {
        let lower = input.to_ascii_lowercase();

        if let Some(enc) = self.lookup_preferred(&lower) {
            return Some(enc);
        }

        // Short-circuits on the first name match: duplicate encoder names occur
        // only across byte-identical codec-alias rows, so the verdict is unchanged.
        self.table
            .iter()
            .flat_map(CodecRow::encoders)
            .find(|(enc, _)| enc.eq_ignore_ascii_case(input))
            .and_then(|(enc, _)| is_encoder_available(enc).then_some(*enc))
    }
}

/// Returns `true` if the named encoder is present in the linked `FFmpeg` build.
///
/// Identical for audio and video — this is the single definition. Requires
/// [`super::ensure_init`] to have been called first.
#[must_use]
pub fn is_encoder_available(encoder: &str) -> bool {
    ffmpeg_the_third::codec::encoder::find_by_name(encoder).is_some()
}

/// The row's encoders that are present in this build, in preference order.
pub fn available_encoders<R: CodecRow>(
    row: &'static R,
) -> impl Iterator<Item = (&'static str, &'static str)> {
    row.encoders()
        .iter()
        .filter(|(enc, _)| is_encoder_available(enc))
        .map(|(enc, display)| (*enc, *display))
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

    static REGISTRY_FAKE: Registry<FakeRow> = Registry::new(FAKE_TABLE, MediaKind::Audio);

    #[test]
    fn is_encoder_available_false_for_nonsense_name() {
        assert!(!is_encoder_available("nonexistent_encoder_xyz"));
    }

    #[test]
    fn is_encoder_available_true_for_builtin_aac() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert!(is_encoder_available("aac"));
    }

    /// Every encoder listed for `"fakecodec"` is absent from any real build,
    /// so the registry must resolve to `None`, not silently fall through to
    /// a permissive default. Guards the `.find(available)` +
    /// `.copied().flatten()` chain in `lookup_preferred` against a mutation
    /// that would return the first (unavailable) entry regardless.
    #[test]
    fn preferred_encoder_none_when_no_encoder_is_available() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_FAKE.preferred_encoder("fakecodec"), None);
    }

    static TABLE_A: &[FakeRow] = &[FakeRow {
        codec: "shared_name",
        encoders: &[("aac", "AAC")],
    }];
    static TABLE_C: &[FakeRow] = &[FakeRow {
        codec: "ordered",
        encoders: &[
            ("nonexistent_encoder_first", "Missing 1"),
            ("aac", "AAC"),
            ("pcm_s16le", "PCM"),
        ],
    }];

    static REGISTRY_A: Registry<FakeRow> = Registry::new(TABLE_A, MediaKind::Audio);
    static REGISTRY_C: Registry<FakeRow> = Registry::new(TABLE_C, MediaKind::Audio);

    #[test]
    fn preferred_encoder_returns_first_available() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_A.preferred_encoder("shared_name"), Some("aac"));
    }

    #[test]
    fn preferred_encoder_unknown_codec_is_none() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_A.preferred_encoder("no_such_codec"), None);
    }

    /// Preference order matters: the first entry is an unavailable encoder,
    /// so the registry must skip it and land on `aac` — not `pcm_s16le`,
    /// which is also available but listed later. A one-encoder fixture
    /// (`TABLE_A`) cannot distinguish `.find(available)` from `.first()` or
    /// `.last()`; this table can.
    #[test]
    fn preferred_encoder_skips_unavailable_and_respects_order() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_C.preferred_encoder("ordered"), Some("aac"));
    }

    /// Pins both the availability filter AND the preservation of preference
    /// order in `available_encoders`.
    #[test]
    fn available_encoders_filters_and_preserves_order() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let got: Vec<_> = available_encoders(TABLE_C.first().expect("row")).collect();
        assert_eq!(got, vec![("aac", "AAC"), ("pcm_s16le", "PCM")]);
    }

    #[test]
    fn find_row_matches_case_insensitively() {
        assert!(REGISTRY_A.find_row("SHARED_NAME").is_some());
        assert!(REGISTRY_A.find_row("nope").is_none());
    }

    #[test]
    fn resolve_accepts_a_codec_name() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_A.resolve("shared_name"), Some("aac"));
    }

    #[test]
    fn resolve_accepts_a_direct_encoder_name() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_A.resolve("aac"), Some("aac"));
    }

    #[test]
    fn resolve_rejects_an_unknown_name() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_A.resolve("no_such_thing"), None);
    }

    #[test]
    fn available_encoders_filters_unavailable() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        // FAKE_TABLE's only encoder does not exist in any build.
        assert!(
            available_encoders(FAKE_TABLE.first().expect("row"))
                .next()
                .is_none(),
            "unavailable encoder must be filtered out"
        );

        let got: Vec<_> = available_encoders(TABLE_A.first().expect("row")).collect();
        assert_eq!(got, vec![("aac", "AAC")]);
    }
}
