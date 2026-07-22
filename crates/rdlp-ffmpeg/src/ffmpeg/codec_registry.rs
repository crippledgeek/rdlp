//! Shared core for the audio and video encoder preference registries.
//!
//! Both registries hold a static table of codecs, each with an ordered
//! encoder-preference list, and answer the same four questions against it.
//! Those answers live here once; the per-media modules keep only their table
//! and the `*Info` types they build, which genuinely differ.

use std::collections::HashMap;
use std::sync::OnceLock;

use log::info;
use rdlp_types::media_name::{CodecName, MediaName, NameKind};

/// The minimal view of a preference-table row the shared lookups need.
///
/// The trait carries only what the shared lookups need. `display_name` is
/// common to both row types too, but is read directly by each module's own
/// `list_available_*`, which construct different `*Info` shapes and therefore
/// remain per-media; audio's `supported_containers` and video's speed-control
/// derivation are genuinely media-specific and stay in the owning module.
pub trait CodecRow {
    /// Which encoder-name vocabulary this row's table resolves into —
    /// [`AudioEncoder`](rdlp_types::media_name::AudioEncoder) for the audio
    /// registry, [`VideoEncoder`](rdlp_types::media_name::VideoEncoder) for
    /// the video one. An associated type rather than a second generic
    /// parameter on [`Registry`] because every row of a given table shares
    /// exactly one vocabulary — there is nothing to parameterise per-row.
    type Encoder: NameKind;

    /// Canonical codec name, e.g. `"aac"` / `"h264"`.
    fn codec(&self) -> &CodecName;
    /// Ordered encoder preference list: `(encoder_name, display_name)`.
    fn encoders(&self) -> &'static [(MediaName<Self::Encoder>, &'static str)];
    /// Alternate names that should also resolve to this row, e.g. a
    /// pre-existing CLI vocabulary word that predates the row being keyed to
    /// its exact codec-ID name. Defaults to none so rows (and the video
    /// registry, which has no aliases at all) need not opt in.
    ///
    /// Must be lowercase — [`Registry::lookup_preferred`]'s memoised map is
    /// keyed on an already-lowercased needle (see [`Registry::preferred_encoder`]
    /// / [`Registry::resolve`]) and does an exact-key lookup, so an
    /// uppercase alias here would silently never match. [`Registry::find_row`]
    /// case-folds both sides instead and has no such restriction — that
    /// divergence is deliberate; don't "fix" `find_row` to match this map.
    fn aliases(&self) -> &'static [CodecName] {
        &[]
    }
}

/// Which media a registry serves. Also supplies the word used in the
/// selection log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    cache: OnceLock<HashMap<CodecName, Option<MediaName<R::Encoder>>>>,
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

    /// Finds the row for a codec name, case-insensitively. Matches either the
    /// row's primary codec name or one of its [`CodecRow::aliases`].
    #[must_use]
    pub fn find_row(&self, codec: &str) -> Option<&'static R> {
        self.table.iter().find(|row| {
            row.codec().as_str().eq_ignore_ascii_case(codec)
                || row
                    .aliases()
                    .iter()
                    .any(|alias| alias.as_str().eq_ignore_ascii_case(codec))
        })
    }

    /// Best available encoder for a codec name, memoised in this registry's cache.
    ///
    /// Requires [`super::ensure_init`] to have been called first.
    #[must_use]
    pub fn preferred_encoder(&self, codec: &str) -> Option<MediaName<R::Encoder>> {
        self.lookup_preferred(&codec.to_ascii_lowercase())
    }

    /// As [`Self::preferred_encoder`], but `lower` must already be lowercase.
    ///
    /// Lets [`Self::resolve`] lowercase its input once and reuse the same
    /// lookup, instead of lowercasing again on the way in here.
    fn lookup_preferred(&self, lower: &str) -> Option<MediaName<R::Encoder>> {
        let map = self.cache.get_or_init(|| {
            let mut map = HashMap::new();
            for row in self.table {
                let selected = row
                    .encoders()
                    .iter()
                    .find(|(enc, _)| is_encoder_available(enc.as_str()))
                    .map(|(enc, _)| enc.clone());

                if let Some(enc) = &selected {
                    info!(
                        "Using {enc} as {codec} {kind} encoder",
                        codec = row.codec(),
                        kind = self.kind
                    );
                }

                map.insert(row.codec().clone(), selected.clone());

                // Aliases resolve through this same map, not just through
                // `find_row` — otherwise a codec reachable only by its alias
                // (e.g. the CLI vocabulary word "pcm") resolves via
                // `find_row`-based callers but returns `None` from
                // `preferred_encoder`/`resolve`. See `CodecRow::aliases`'s
                // doc comment for why this insert requires a lowercase
                // alias and why that's a different contract from `find_row`.
                //
                // The `debug_assert_eq!` below is compiled out in release, so
                // it is not what enforces that contract — the real guard is
                // `audio_encoder_registry`'s
                // `every_alias_resolves_to_its_rows_own_encoder`, which fails
                // in any profile because an uppercase alias is inserted
                // verbatim while the lookup needle arrives lowercased, so the
                // map misses. Do not delete that test believing this assert
                // covers release builds.
                for alias in row.aliases() {
                    debug_assert_eq!(
                        alias.as_str(),
                        alias.as_str().to_ascii_lowercase(),
                        "CodecRow::aliases must be declared lowercase: {alias:?}"
                    );
                    map.insert(alias.clone(), selected.clone());
                }
            }
            map
        });

        map.get(lower).cloned().flatten()
    }

    /// Resolves either a codec name or a direct encoder name to an available encoder.
    ///
    /// Codec names go through [`Self::preferred_encoder`]; anything else is
    /// matched against the table's encoder names and then gated on availability.
    ///
    /// Requires [`super::ensure_init`] to have been called first.
    #[must_use]
    pub fn resolve(&self, input: &str) -> Option<MediaName<R::Encoder>> {
        let lower = input.to_ascii_lowercase();

        if let Some(enc) = self.lookup_preferred(&lower) {
            return Some(enc);
        }

        // Short-circuits on the first name match: duplicate encoder names occur
        // only across byte-identical codec-alias rows, so the verdict is unchanged.
        self.table
            .iter()
            .flat_map(CodecRow::encoders)
            .find(|(enc, _)| enc.as_str().eq_ignore_ascii_case(input))
            .and_then(|(enc, _)| is_encoder_available(enc.as_str()).then_some(enc.clone()))
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
) -> impl Iterator<Item = (MediaName<R::Encoder>, &'static str)> {
    row.encoders()
        .iter()
        .filter(|(enc, _)| is_encoder_available(enc.as_str()))
        .map(|(enc, display)| (enc.clone(), *display))
}

#[cfg(test)]
mod tests {
    use rdlp_types::media_name::AudioEncoder;

    use super::*;

    /// Test fixtures reuse the `AudioEncoder` vocabulary marker regardless of
    /// what the fixture data represents — these tables are synthetic and
    /// never touch a real audio/video registry, so which real kind stands in
    /// is arbitrary; only that it is a single, consistent kind matters.
    struct FakeRow {
        codec: CodecName,
        encoders: &'static [(MediaName<AudioEncoder>, &'static str)],
    }

    impl CodecRow for FakeRow {
        type Encoder = AudioEncoder;
        fn codec(&self) -> &CodecName {
            &self.codec
        }
        fn encoders(&self) -> &'static [(MediaName<AudioEncoder>, &'static str)] {
            self.encoders
        }
    }

    static FAKE_TABLE: &[FakeRow] = &[FakeRow {
        codec: CodecName::from_static("fakecodec"),
        encoders: &[(MediaName::from_static("nonexistent_encoder_xyz"), "Fake")],
    }];

    static REGISTRY_FAKE: Registry<FakeRow> = Registry::new(FAKE_TABLE, MediaKind::Audio);

    struct AliasedRow {
        codec: CodecName,
        aliases: &'static [CodecName],
        encoders: &'static [(MediaName<AudioEncoder>, &'static str)],
    }

    impl CodecRow for AliasedRow {
        type Encoder = AudioEncoder;
        fn codec(&self) -> &CodecName {
            &self.codec
        }
        fn encoders(&self) -> &'static [(MediaName<AudioEncoder>, &'static str)] {
            self.encoders
        }
        fn aliases(&self) -> &'static [CodecName] {
            self.aliases
        }
    }

    static ALIASED_TABLE: &[AliasedRow] = &[AliasedRow {
        codec: CodecName::from_static("pcm_s16le"),
        aliases: &[CodecName::from_static("pcm")],
        encoders: &[(MediaName::from_static("pcm_s16le"), "PCM")],
    }];

    static REGISTRY_ALIASED: Registry<AliasedRow> = Registry::new(ALIASED_TABLE, MediaKind::Audio);

    /// `find_row` must match a row by its declared alias, not just its
    /// primary `codec()` name — the mechanism Important-4 restores.
    #[test]
    fn find_row_matches_by_alias() {
        assert!(REGISTRY_ALIASED.find_row("pcm").is_some());
        assert!(
            REGISTRY_ALIASED.find_row("PCM").is_some(),
            "case-insensitive"
        );
        assert_eq!(
            REGISTRY_ALIASED.find_row("pcm").unwrap().codec().as_str(),
            "pcm_s16le"
        );
    }

    /// Negative control: an alias-shaped string that was never declared must
    /// not accidentally match via the default `&[]` aliases.
    #[test]
    fn find_row_does_not_match_undeclared_alias() {
        assert!(REGISTRY_ALIASED.find_row("pcm_s24le").is_none());
        assert!(REGISTRY_FAKE.find_row("pcm").is_none());
    }

    /// `preferred_encoder` must resolve an alias too, not just `find_row` —
    /// the CRITICAL-8 gap: aliases reached `find_row`-based callers
    /// (`container_supports_audio_codec`) but not `lookup_preferred`-based
    /// ones (`preferred_audio_encoder`, `resolve_audio_encoder`), so
    /// `--recode-audio=pcm` silently fell through to a container default.
    #[test]
    fn preferred_encoder_resolves_via_alias() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            REGISTRY_ALIASED
                .preferred_encoder("pcm")
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("pcm_s16le")
        );
    }

    /// Same gap through the `resolve` entry point (what
    /// `resolve_audio_encoder` — and therefore the CLI/config `pcm` value —
    /// actually calls).
    #[test]
    fn resolve_resolves_via_alias() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            REGISTRY_ALIASED
                .resolve("pcm")
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("pcm_s16le")
        );
    }

    /// Negative control: an undeclared alias-shaped string must still miss
    /// through the memoised-map path, same as it does through `find_row`.
    #[test]
    fn preferred_encoder_undeclared_alias_is_none() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(REGISTRY_ALIASED.preferred_encoder("pcm_s24le"), None);
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
        codec: CodecName::from_static("shared_name"),
        encoders: &[(MediaName::from_static("aac"), "AAC")],
    }];
    static TABLE_C: &[FakeRow] = &[FakeRow {
        codec: CodecName::from_static("ordered"),
        encoders: &[
            (
                MediaName::from_static("nonexistent_encoder_first"),
                "Missing 1",
            ),
            (MediaName::from_static("aac"), "AAC"),
            (MediaName::from_static("pcm_s16le"), "PCM"),
        ],
    }];

    static REGISTRY_A: Registry<FakeRow> = Registry::new(TABLE_A, MediaKind::Audio);
    static REGISTRY_C: Registry<FakeRow> = Registry::new(TABLE_C, MediaKind::Audio);

    #[test]
    fn preferred_encoder_returns_first_available() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            REGISTRY_A
                .preferred_encoder("shared_name")
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("aac")
        );
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
        assert_eq!(
            REGISTRY_C
                .preferred_encoder("ordered")
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("aac")
        );
    }

    /// Pins both the availability filter AND the preservation of preference
    /// order in `available_encoders`.
    #[test]
    fn available_encoders_filters_and_preserves_order() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let got: Vec<_> = available_encoders(TABLE_C.first().expect("row"))
            .map(|(enc, display)| (enc.as_str().to_owned(), display))
            .collect();
        assert_eq!(
            got,
            vec![("aac".to_string(), "AAC"), ("pcm_s16le".to_string(), "PCM")]
        );
    }

    #[test]
    fn find_row_matches_case_insensitively() {
        assert!(REGISTRY_A.find_row("SHARED_NAME").is_some());
        assert!(REGISTRY_A.find_row("nope").is_none());
    }

    #[test]
    fn resolve_accepts_a_codec_name() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            REGISTRY_A
                .resolve("shared_name")
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("aac")
        );
    }

    #[test]
    fn resolve_accepts_a_direct_encoder_name() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        assert_eq!(
            REGISTRY_A
                .resolve("aac")
                .as_ref()
                .map(rdlp_types::media_name::MediaName::as_str),
            Some("aac")
        );
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

        let got: Vec<_> = available_encoders(TABLE_A.first().expect("row"))
            .map(|(enc, display)| (enc.as_str().to_owned(), display))
            .collect();
        assert_eq!(got, vec![("aac".to_string(), "AAC")]);
    }
}
