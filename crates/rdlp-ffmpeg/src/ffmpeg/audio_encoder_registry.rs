//! Audio encoder registry.
//!
//! Provides a static preference table mapping audio codec names to encoder
//! preference lists and container compatibility information. Runtime detection
//! via `ffmpeg_the_third::codec::encoder::find_by_name()` determines which
//! encoders are actually available in the linked `FFmpeg` build.
//!
//! Mirrors the pattern of `video_codecs.rs`.
//!
//! # Key Functions
//!
//! - [`preferred_audio_encoder`] — best available encoder for a codec name
//! - [`resolve_audio_encoder`] — accepts codec or encoder name
//! - [`list_available_audio_codecs`] — all codecs with at least one available encoder
//! - [`audio_codecs_for_container`] — filtered by container compatibility
//! - [`container_supports_audio_codec`] — point query for validation
//! - [`select_audio_encoder_for_container`] — best default encoder for a container

use rdlp_types::ContainerFormat;
use serde::{Deserialize, Serialize};

use crate::ffmpeg::{codec_registry, muxer_defaults};

/// Information about a specific audio encoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEncoderInfo {
    /// `FFmpeg` encoder name (e.g., "`libfdk_aac`", "libopus").
    pub encoder_name: String,
    /// Human-readable display name (e.g., "FDK AAC", "Opus (libopus)").
    pub display_name: String,
}

/// Information about an audio codec and its available encoders.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCodecInfo {
    /// Canonical codec name (e.g., "aac", "opus").
    pub codec: String,
    /// Human-readable display name (e.g., "AAC", "Opus").
    pub display_name: String,
    /// Encoders available in the current `FFmpeg` build, in preference order.
    pub encoders: Vec<AudioEncoderInfo>,
    /// Containers this codec is compatible with.
    pub supported_containers: Vec<ContainerFormat>,
}

/// Entry in the audio codec preference table.
struct AudioCodecEntry {
    /// Canonical codec name.
    codec: &'static str,
    /// Human-readable display name.
    display_name: &'static str,
    /// Ordered encoder preference list: (`encoder_name`, `display_name`).
    encoders: &'static [(&'static str, &'static str)],
    /// Container formats this codec is compatible with.
    supported_containers: &'static [ContainerFormat],
    /// Alternate names that should also resolve to this row. Declared next
    /// to the row it describes rather than in a separate `match self.codec`
    /// keyed by string — that indirection let a rename of `codec` silently
    /// drop the alias through the `CodecRow::aliases` trait default. Empty
    /// for every row but `pcm_s16le`.
    aliases: &'static [&'static str],
}

impl codec_registry::CodecRow for AudioCodecEntry {
    fn codec(&self) -> &'static str {
        self.codec
    }
    fn encoders(&self) -> &'static [(&'static str, &'static str)] {
        self.encoders
    }
    fn aliases(&self) -> &'static [&'static str] {
        self.aliases
    }
}

/// Static preference table for audio codecs.
///
/// For each codec, encoders are listed in preference order — the first
/// available encoder wins. Container compatibility is authoritative for
/// the `container_supports_audio_codec` gate and the frontend greying logic.
static AUDIO_CODEC_PREFERENCES: &[AudioCodecEntry] = &[
    AudioCodecEntry {
        codec: "aac",
        display_name: "AAC",
        encoders: &[
            ("libfdk_aac", "FDK AAC (libfdk_aac)"),
            ("aac", "AAC (built-in)"),
        ],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Avi,
            ContainerFormat::Ts,
            ContainerFormat::Aac,
            ContainerFormat::M4a,
            // ThreeGp: rdlp overrides the muxer's own `amr_nb` default to
            // aac (see `audio_default_for`); the compatibility matrix must
            // accept the codec it actually produces. M4v/F4v: their muxers
            // declare aac as the default (verified against the linked
            // FFmpeg build).
            ContainerFormat::ThreeGp,
            ContainerFormat::M4v,
            ContainerFormat::F4v,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "mp3",
        display_name: "MP3",
        encoders: &[("libmp3lame", "MP3 (libmp3lame)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Avi,
            ContainerFormat::Ts,
            ContainerFormat::Mp3,
            // Flv's muxer declares mp3 as its default audio codec.
            ContainerFormat::Flv,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "opus",
        display_name: "Opus",
        encoders: &[("libopus", "Opus (libopus)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mkv,
            // Mka: rdlp overrides matroska's own default to opus (see
            // `audio_default_for`); the matrix must accept it.
            ContainerFormat::Mka,
            ContainerFormat::WebM,
            ContainerFormat::Ogg,
            ContainerFormat::Opus,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "vorbis",
        display_name: "Vorbis",
        encoders: &[("libvorbis", "Vorbis (libvorbis)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::WebM,
            ContainerFormat::Ogg,
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "flac",
        display_name: "FLAC",
        encoders: &[("flac", "FLAC (built-in)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::Ogg,
            ContainerFormat::Flac,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "alac",
        display_name: "ALAC",
        encoders: &[("alac", "ALAC (built-in)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "ac3",
        display_name: "AC-3 (Dolby Digital)",
        encoders: &[("ac3", "AC-3 (built-in)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Avi,
            ContainerFormat::Ts,
            ContainerFormat::Ac3,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "eac3",
        display_name: "E-AC-3 (Dolby Digital Plus)",
        encoders: &[("eac3", "E-AC-3 (built-in)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Ts,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "dts",
        display_name: "DTS",
        encoders: &[("dca", "DTS (built-in)")],
        supported_containers: &[ContainerFormat::Mkv],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "mp2",
        display_name: "MP2",
        encoders: &[("mp2", "MP2 (built-in)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::Ts,
            // Mpg/Vob's muxers both declare mp2 as their default audio codec.
            ContainerFormat::Mpg,
            ContainerFormat::Vob,
        ],
        aliases: &[],
    },
    // Keyed to the exact FFmpeg codec-ID name (rather than the historical
    // "pcm" vocabulary word) so it matches `muxer_defaults::declared_codec`
    // and stays in tier 1 — see `resolve_declared_codec`'s tiers. "pcm"
    // still resolves via `CodecRow::aliases` below, for the CLI vocabulary a
    // user may type from muscle memory.
    //
    // Aiff/Caf are listed here too, alongside the separate `pcm_s16be` row:
    // this is a *compatibility* fact ("pcm_s16le can be muxed into AIFF/CAF",
    // verified against the linked FFmpeg build), not a claim about either
    // container's *default* (which is `pcm_s16be` — see the row below). The
    // two facts coexist; neither row should be read as claiming the other.
    AudioCodecEntry {
        codec: "pcm_s16le",
        display_name: "PCM 16-bit little-endian",
        encoders: &[("pcm_s16le", "PCM 16-bit (built-in)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::Avi,
            ContainerFormat::Wav,
            ContainerFormat::Aiff,
            ContainerFormat::Caf,
            // Mxf and Dv's muxers both declare pcm_s16le as their default
            // audio codec (caught by the sweep test, not the review's table).
            ContainerFormat::Mxf,
            ContainerFormat::Dv,
        ],
        aliases: &["pcm"],
    },
    // Big-endian PCM is a distinct codec-ID from the little-endian
    // `pcm_s16le` row above (AIFF/CAF declare `pcm_s16be`; WAV/AVI/MKV
    // declare `pcm_s16le`). Keyed to the exact FFmpeg codec-ID name so it
    // matches `muxer_defaults::declared_codec` and stays in tier 1 rather
    // than silently falling through to tier 2's literal-encoder-name
    // fallback in `resolve_declared_codec`.
    //
    // Behavioural neutrality of this row being in tier 1 rather than tier 2
    // depends on `encoders` staying the singleton `[("pcm_s16be", _)]` where
    // the encoder name equals the codec key — see `resolve_declared_codec`'s
    // doc comment for the structural argument. Adding a second encoder here
    // would need that argument re-checked.
    AudioCodecEntry {
        codec: "pcm_s16be",
        display_name: "PCM 16-bit big-endian",
        encoders: &[("pcm_s16be", "PCM 16-bit big-endian (built-in)")],
        supported_containers: &[ContainerFormat::Aiff, ContainerFormat::Caf],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "wavpack",
        display_name: "WavPack",
        encoders: &[("wavpack", "WavPack (built-in)")],
        supported_containers: &[ContainerFormat::Mkv, ContainerFormat::Wv],
        aliases: &[],
    },
    // WMA had no compatibility-matrix row at all; `.wma`/`.wmv`/`.asf` all
    // declare `wmav2` as their default audio codec (see `asf_family_gets_wmav2`).
    // Keyed to the exact FFmpeg codec-ID name for the same reason as
    // `pcm_s16be` above.
    //
    // Same singleton dependency as `pcm_s16be`: `encoders` must stay
    // `[("wmav2", _)]` (key == encoder name) for tier 1 to be behaviourally
    // identical to tier 2's fallback — see `resolve_declared_codec`.
    AudioCodecEntry {
        codec: "wmav2",
        display_name: "WMA v2",
        encoders: &[("wmav2", "Windows Media Audio 2 (built-in)")],
        supported_containers: &[
            ContainerFormat::Wma,
            ContainerFormat::Wmv,
            ContainerFormat::Asf,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: "tta",
        display_name: "TTA",
        encoders: &[("tta", "TTA (built-in)")],
        supported_containers: &[ContainerFormat::Mkv],
        aliases: &[],
    },
];

/// The audio codec registry, owning its own cache over [`AUDIO_CODEC_PREFERENCES`].
static AUDIO_REGISTRY: codec_registry::Registry<AudioCodecEntry> =
    codec_registry::Registry::new(AUDIO_CODEC_PREFERENCES, codec_registry::MediaKind::Audio);

/// Returns `true` if the named audio encoder is available in the current `FFmpeg` build.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn is_audio_encoder_available(encoder: &str) -> bool {
    codec_registry::is_encoder_available(encoder)
}

/// Returns the best available audio encoder for a given codec name.
///
/// Results are cached via a `OnceLock<HashMap>` so detection only runs once.
/// Returns `None` if no encoder is available or the codec is unknown.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn preferred_audio_encoder(codec: &str) -> Option<&'static str> {
    AUDIO_REGISTRY.preferred_encoder(codec)
}

/// Resolves an audio encoder name.
///
/// Accepts either a codec name (e.g., "aac") or a direct encoder name
/// (e.g., "`libfdk_aac`"). For codec names, returns the best available encoder
/// via [`preferred_audio_encoder`]. For encoder names, checks availability
/// directly and returns the static str if available.
///
/// Returns `None` if no encoder can be resolved or is available.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn resolve_audio_encoder(input: &str) -> Option<&'static str> {
    AUDIO_REGISTRY.resolve(input)
}

/// Returns all available encoders for a given audio codec name, in preference order.
///
/// Returns an empty `Vec` if the codec is unknown or has no available encoders.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn available_audio_encoders_for_codec(codec: &str) -> Vec<AudioEncoderInfo> {
    AUDIO_REGISTRY
        .find_row(codec)
        .map(|row| {
            codec_registry::available_encoders(row)
                .map(|(enc, display)| AudioEncoderInfo {
                    encoder_name: enc.to_string(),
                    display_name: display.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lists all audio codecs that have at least one available encoder in the
/// current `FFmpeg` build.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn list_available_audio_codecs() -> Vec<AudioCodecInfo> {
    AUDIO_CODEC_PREFERENCES
        .iter()
        .filter_map(|entry| {
            let encoders: Vec<AudioEncoderInfo> = codec_registry::available_encoders(entry)
                .map(|(enc, display)| AudioEncoderInfo {
                    encoder_name: enc.to_string(),
                    display_name: display.to_string(),
                })
                .collect();

            if encoders.is_empty() {
                None
            } else {
                Some(AudioCodecInfo {
                    codec: entry.codec.to_string(),
                    display_name: entry.display_name.to_string(),
                    encoders,
                    supported_containers: entry.supported_containers.to_vec(),
                })
            }
        })
        .collect()
}

/// Returns audio codecs compatible with the given container format.
///
/// Filters `list_available_audio_codecs()` to only those codecs whose
/// `supported_containers` list includes `container`.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn audio_codecs_for_container(container: ContainerFormat) -> Vec<AudioCodecInfo> {
    list_available_audio_codecs()
        .into_iter()
        .filter(|codec| codec.supported_containers.contains(&container))
        .collect()
}

/// Returns `true` if `codec` is compatible with `container`.
///
/// Uses the static compatibility matrix — does NOT check runtime availability.
/// The `codec` argument accepts canonical codec names (e.g., "aac", "opus").
///
/// # Examples
///
/// ```no_run
/// use rdlp_ffmpeg::ffmpeg::audio_encoder_registry::container_supports_audio_codec;
/// use rdlp_types::ContainerFormat;
///
/// assert!(container_supports_audio_codec(ContainerFormat::Mp4, "aac"));
/// assert!(!container_supports_audio_codec(ContainerFormat::Mp4, "vorbis"));
/// assert!(container_supports_audio_codec(ContainerFormat::WebM, "opus"));
/// assert!(!container_supports_audio_codec(ContainerFormat::Mov, "opus"));
/// ```
#[must_use]
pub fn container_supports_audio_codec(container: ContainerFormat, codec: &str) -> bool {
    AUDIO_REGISTRY
        .find_row(codec)
        .is_some_and(|entry| entry.supported_containers.contains(&container))
}

/// What decides a container's default audio encoder.
enum AudioDefault {
    /// Defer to whatever the linked `FFmpeg` muxer declares.
    FromMuxer,
    /// rdlp overrides the muxer's declaration; the reason is on the arm.
    Override(&'static str),
    /// The container carries no audio stream at all.
    NoAudio,
}

/// Policy for each container. Exhaustive with no `_` arm: a new
/// [`ContainerFormat`] variant must be classified here or the build fails.
const fn audio_default_for(container: ContainerFormat) -> AudioDefault {
    match container {
        // Mkv/Mka: Opus over the matroska muxer's historical `vorbis`
        // declaration — better quality per bit, and Matroska carries Opus
        // without caveat.
        //
        // Ogg: preserved as-is pending #623. `.ogg` conventionally means Ogg
        // Vorbis (Xiph; RFC 7845 §9 recommends `.opus` for Opus), so this
        // override is probably wrong for Ogg specifically — but changing it
        // is a separate user-visible change.
        ContainerFormat::Mkv | ContainerFormat::Mka | ContainerFormat::Ogg => {
            AudioDefault::Override("opus")
        }

        // FFmpeg declares `amr_nb`: 8 kHz speech-only, and the native encoder
        // is absent from most builds. 3GPP TS 26.244 permits AMR, AMR-WB and
        // AAC, so AAC is spec-correct rather than a workaround.
        ContainerFormat::ThreeGp => AudioDefault::Override("aac"),

        // Raw VP8/VP9/AV1 elementary stream; the muxer declares no audio codec
        // and refuses any audio stream outright.
        ContainerFormat::Ivf => AudioDefault::NoAudio,

        ContainerFormat::Mp4
        | ContainerFormat::WebM
        | ContainerFormat::Mov
        | ContainerFormat::M4v
        | ContainerFormat::Ts
        | ContainerFormat::Flv
        | ContainerFormat::Avi
        | ContainerFormat::Mpg
        | ContainerFormat::F4v
        | ContainerFormat::Wmv
        | ContainerFormat::Wma
        | ContainerFormat::Asf
        | ContainerFormat::Mxf
        | ContainerFormat::Vob
        | ContainerFormat::Dv
        | ContainerFormat::Nut
        | ContainerFormat::M4a
        | ContainerFormat::Mp3
        | ContainerFormat::Wav
        | ContainerFormat::Flac
        | ContainerFormat::Opus
        | ContainerFormat::Aac
        | ContainerFormat::Aiff
        | ContainerFormat::Wv
        | ContainerFormat::Caf
        | ContainerFormat::Ac3 => AudioDefault::FromMuxer,
    }
}

/// Returns the best default audio encoder for `container`.
///
/// Returns `None` when no audio encoder could be determined for it: the
/// container genuinely carries no audio stream, or the muxer/codec tables
/// yielded no usable default (see `muxer_defaults::declared_codec` for the
/// causes).
///
/// Most containers defer to what the linked `FFmpeg` muxer declares, so rdlp
/// keeps no second copy of `FFmpeg`'s table. The handful of deliberate
/// deviations are named in `audio_default_for` with their reasons.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn select_audio_encoder_for_container(container: ContainerFormat) -> Option<&'static str> {
    let codec = match audio_default_for(container) {
        AudioDefault::NoAudio => return None,
        AudioDefault::Override(codec) => codec,
        AudioDefault::FromMuxer => {
            let Some(codec) =
                muxer_defaults::declared_codec(container, codec_registry::MediaKind::Audio)
            else {
                // Every other `None` path logs (the ABI-skew warn inside
                // `declared_codec`, the tier-3 AAC-fallback warn below) or is
                // a declared policy (`NoAudio`). "No muxer claims this
                // extension" is the one case that would otherwise pass
                // through silently — the case an operator would most want to
                // see (#618).
                log::warn!(
                    "no muxer declares an audio codec for container {}; \
                     no default audio encoder available",
                    container.as_ext()
                );
                return None;
            };
            codec
        }
    };

    resolve_declared_codec(codec, container)
}

/// Resolves a declared codec-ID name to an available encoder, in three tiers:
///
/// 1. **Preference-table match** — `codec` is an exact key in
///    `AUDIO_CODEC_PREFERENCES` (or one of its aliases); use the best
///    available encoder for that row (e.g. `libfdk_aac` over `aac`).
/// 2. **Literal encoder-name fallback** — `codec` has no table row (native
///    single-encoder codecs like PCM/WMA have none: the codec-ID name IS the
///    encoder name), so accept it directly if it's available in this build.
/// 3. **AAC-with-a-warning fallback** — neither tier resolved anything;
///    substitute AAC, but only when `container` can actually carry it (see
///    Important-2), and always log so this never becomes a silent
///    catch-all.
///
/// `codec` is `FFmpeg`'s codec-ID name (e.g. "`pcm_s16le`", "`wmav2`", "vorbis"),
/// not necessarily a key in `AUDIO_CODEC_PREFERENCES` — see tier 2 above.
///
/// Split out of [`select_audio_encoder_for_container`] so tier 3 is
/// testable with an arbitrary/bogus codec name, independent of what any
/// particular linked `FFmpeg` build actually declares.
///
/// Caveat: `is_audio_encoder_available` is a bare `find_by_name(..).is_some()`
/// — it does not check `AV_CODEC_CAP_EXPERIMENTAL`. `FFmpeg`'s native
/// `vorbis`/`opus` encoders both carry that flag and fail to open without
/// `strict_std_compliance = experimental`, so on a build lacking
/// `libvorbis`/`libopus` tier 2 can resolve a codec-ID name (`vorbis`)
/// that "is available" by this check yet won't actually open. Not filtered
/// here: none of rdlp's transcode call sites set `strict_std_compliance`, so
/// this is a real gap. It is deferred, not because filtering would require
/// major plumbing — `ffmpeg-the-third` already exposes
/// `Codec::capabilities()` / `Capabilities::EXPERIMENTAL`, so a self-filter
/// here would be a few lines with no new plumbing — but because the failure
/// mode this leaves in place is a loud encoder-open error at transcode time,
/// not a silent wrong-codec substitution (the #618 bug class). Tracked as
/// #625.
fn resolve_declared_codec(codec: &'static str, container: ContainerFormat) -> Option<&'static str> {
    preferred_audio_encoder(codec)
        .or_else(|| is_audio_encoder_available(codec).then_some(codec))
        .or_else(|| {
            // Neither the preference table nor a direct name match resolved
            // an available encoder. AAC is only a sane fallback when the
            // container can actually carry it — substituting it into a
            // container that provably cannot (e.g. `.mp3` on a build without
            // libmp3lame) would recreate the #618 failure class in reduced
            // form. Refuse truthfully instead; the caller already turns a
            // `None` here into `RecodeStage`'s/normalize's honest refusal.
            if !container_supports_audio_codec(container, "aac") {
                log::warn!(
                    "no encoder available for {codec} (default for {}), and that \
                     container cannot carry AAC either; refusing rather than \
                     producing a container-incompatible file",
                    container.as_ext()
                );
                return None;
            }
            // Fall back to AAC, but SAY SO — a silent fallback would
            // recreate the invisible `_ => aac` this change exists to remove.
            log::warn!(
                "no encoder available for {codec} (default for {}); falling back to AAC",
                container.as_ext()
            );
            preferred_audio_encoder("aac")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_audio_encoder_aac_returns_valid() {
        let enc = preferred_audio_encoder("aac");
        assert!(enc.is_some(), "aac encoder should be available");
        let name = enc.unwrap();
        assert!(
            name == "aac" || name == "libfdk_aac",
            "unexpected aac encoder: {name}"
        );
    }

    #[test]
    fn resolve_encoder_by_codec_name() {
        let enc = resolve_audio_encoder("aac");
        assert!(enc.is_some(), "should resolve aac codec");
    }

    /// CRITICAL-8 regression guard: `resolve_audio_encoder("pcm")` must
    /// resolve via the `pcm_s16le` row's alias, not fall through to `None`.
    /// `pcm_s16le` is a built-in `FFmpeg` encoder present in every build (the
    /// same assumption `audio_only_containers_get_their_own_codec_not_aac`
    /// already makes unguarded), so this is safe to assert unconditionally.
    #[test]
    fn resolve_encoder_by_alias_pcm() {
        ensure_init_for_test();
        assert_eq!(resolve_audio_encoder("pcm"), Some("pcm_s16le"));
        assert_eq!(preferred_audio_encoder("pcm"), Some("pcm_s16le"));
    }

    #[test]
    fn resolve_encoder_by_encoder_name() {
        // "libmp3lame" is an ENCODER name (the sole encoder under codec key
        // "mp3"), not itself a codec key, so this exercises the
        // direct-encoder-name branch of `resolve` rather than short-circuiting
        // through `preferred_encoder` the way `resolve_encoder_by_codec_name`
        // (which passes "aac", a codec key) does. Gated on availability so a
        // build without libmp3lame still passes — either branch outcome
        // proves the direct-name path was taken, since "libmp3lame" never
        // matches a codec key.
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        if is_audio_encoder_available("libmp3lame") {
            assert_eq!(resolve_audio_encoder("libmp3lame"), Some("libmp3lame"));
        } else {
            assert_eq!(resolve_audio_encoder("libmp3lame"), None);
        }
    }

    #[test]
    fn list_available_audio_codecs_nonempty() {
        let codecs = list_available_audio_codecs();
        assert!(
            !codecs.is_empty(),
            "at least built-in codecs should be available"
        );
    }

    #[test]
    fn audio_codecs_for_webm_only_opus_vorbis() {
        let codecs = audio_codecs_for_container(ContainerFormat::WebM);
        let codec_names: Vec<&str> = codecs.iter().map(|c| c.codec.as_str()).collect();
        // WebM only supports Opus and Vorbis
        for name in &codec_names {
            assert!(
                *name == "opus" || *name == "vorbis",
                "unexpected codec for webm: {name}"
            );
        }
    }

    #[test]
    fn audio_codecs_for_mkv_includes_all() {
        let codecs = audio_codecs_for_container(ContainerFormat::Mkv);
        // MKV supports everything; at minimum aac should be present
        assert!(
            codecs.iter().any(|c| c.codec.as_str() == "aac"),
            "mkv should support aac"
        );
    }

    #[test]
    fn container_supports_mp4_opus() {
        assert!(container_supports_audio_codec(ContainerFormat::Mp4, "opus"));
    }

    #[test]
    fn container_does_not_support_mov_opus() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mov,
            "opus"
        ));
    }

    #[test]
    fn select_encoder_for_mp4_returns_aac() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let enc = select_audio_encoder_for_container(ContainerFormat::Mp4);
        assert!(
            enc == Some("aac") || enc == Some("libfdk_aac"),
            "expected aac encoder for mp4, got {enc:?}"
        );
    }

    #[test]
    fn compatibility_accepts_opus_mkv() {
        assert!(container_supports_audio_codec(ContainerFormat::Mkv, "opus"));
    }

    #[test]
    fn unknown_codec_returns_false() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            "nonexistent_codec"
        ));
    }

    #[test]
    fn unknown_codec_resolve_returns_none() {
        assert!(resolve_audio_encoder("nonexistent_codec_xyz").is_none());
    }

    #[test]
    fn registered_but_unavailable_encoder_resolves_none() {
        // `libfdk_aac` is registered in AUDIO_CODEC_PREFERENCES but is a nonfree
        // encoder usually absent from a stock ffmpeg build. When present it
        // resolves to itself; when absent the name matches yet the availability
        // gate rejects it → None. Pins the `.and_then(is_available.then_some(..))`
        // unavailable branch of the iterator chain.
        if is_audio_encoder_available("libfdk_aac") {
            assert_eq!(resolve_audio_encoder("libfdk_aac"), Some("libfdk_aac"));
        } else {
            assert_eq!(resolve_audio_encoder("libfdk_aac"), None);
        }
    }

    /// Guarded against a thin build lacking `libmp3lame`: on a build with it,
    /// this fails under the old `_ => "aac"` catch-all (mutation-verified,
    /// see the module report). On a build WITHOUT it, tier 3's AAC-fallback
    /// is correct by design, so a bare "must not be aac" would itself be a
    /// false failure — the honest form only asserts the exact
    /// codec when it's actually available.
    #[test]
    fn audio_only_containers_get_their_own_codec_not_aac() {
        ensure_init_for_test();
        let mp3 = select_audio_encoder_for_container(ContainerFormat::Mp3);
        assert!(
            mp3 == Some("libmp3lame") || !is_audio_encoder_available("libmp3lame"),
            "expected libmp3lame for .mp3 when available, got {mp3:?}"
        );
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Flac),
            Some("flac")
        );
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Wav),
            Some("pcm_s16le")
        );
    }

    #[test]
    fn asf_family_gets_wmav2() {
        ensure_init_for_test();
        for c in [
            ContainerFormat::Wmv,
            ContainerFormat::Wma,
            ContainerFormat::Asf,
        ] {
            let enc = select_audio_encoder_for_container(c);
            assert!(
                enc == Some("wmav2") || !is_audio_encoder_available("wmav2"),
                "{c:?}: expected wmav2 when available, got {enc:?}"
            );
        }
    }

    #[test]
    fn ivf_carries_no_audio() {
        ensure_init_for_test();
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Ivf),
            None
        );
    }

    /// The four overrides, asserted against literal encoder names — NOT
    /// against `declared_codec(..)`, which would be a tautology that
    /// survives mutating the override away. Guarded the same honest way as
    /// `asf_family_gets_wmav2`: only asserted when the encoder is
    /// actually available, since tier 3's AAC fallback is correct by design
    /// otherwise.
    #[test]
    fn overrides_are_exactly_these_four() {
        use strum::IntoEnumIterator;

        ensure_init_for_test();
        for c in [
            ContainerFormat::Mkv,
            ContainerFormat::Mka,
            ContainerFormat::Ogg,
        ] {
            let enc = select_audio_encoder_for_container(c);
            assert!(
                enc == Some("libopus") || !is_audio_encoder_available("libopus"),
                "{c:?}: expected libopus when available, got {enc:?}"
            );
        }

        let threegp = select_audio_encoder_for_container(ContainerFormat::ThreeGp);
        assert!(
            threegp == Some("aac") || threegp == Some("libfdk_aac"),
            "expected an aac-family encoder for 3gp, got {threegp:?}"
        );

        // The "exactly" half: no OTHER container may be classified
        // `AudioDefault::Override`. Without this, adding e.g.
        // `Avi => Override("opus")` to `audio_default_for` would leave the
        // assertions above green.
        let known_overrides = [
            ContainerFormat::Mkv,
            ContainerFormat::Mka,
            ContainerFormat::Ogg,
            ContainerFormat::ThreeGp,
        ];
        for container in ContainerFormat::iter() {
            let is_override = matches!(audio_default_for(container), AudioDefault::Override(_));
            assert_eq!(
                is_override,
                known_overrides.contains(&container),
                "{container:?}: AudioDefault::Override classification does not \
                 match the known set of four overrides"
            );
        }
    }

    /// Containers that defer to the muxer must NOT all be AAC — that was the
    /// old catch-all's signature failure. Guarded the same honest way as
    /// `asf_family_gets_wmav2`: on a thin build missing
    /// `mp2`/`libvorbis`, tier 3's AAC fallback is correct by design, so
    /// these only assert the exact codec when it's actually available.
    #[test]
    fn from_muxer_containers_are_not_uniformly_aac() {
        ensure_init_for_test();

        let ts = select_audio_encoder_for_container(ContainerFormat::Ts);
        assert!(
            ts == Some("mp2") || !is_audio_encoder_available("mp2"),
            "expected mp2 for .ts when available, got {ts:?}"
        );

        let nut = select_audio_encoder_for_container(ContainerFormat::Nut);
        assert!(
            nut == Some("libvorbis") || !is_audio_encoder_available("libvorbis"),
            "expected libvorbis for .nut when available, got {nut:?}"
        );

        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Ac3),
            Some("ac3")
        );
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Aiff),
            Some("pcm_s16be")
        );
    }

    #[test]
    fn mp4_family_still_gets_aac() {
        ensure_init_for_test();
        for c in [
            ContainerFormat::Mp4,
            ContainerFormat::M4a,
            ContainerFormat::M4v,
            ContainerFormat::Mov,
            ContainerFormat::F4v,
        ] {
            let enc = select_audio_encoder_for_container(c);
            assert!(
                enc == Some("aac") || enc == Some("libfdk_aac"),
                "expected an aac-family encoder for {c:?}, got {enc:?}"
            );
        }
    }

    /// Tier 3 (the AAC-with-warning fallback the whole design hinges on
    /// being VISIBLE) had no test — tiers 1-2 were covered, tier 3 wasn't.
    /// A bogus codec name skips the preference table (tier 1) and the
    /// direct-name check (tier 2) unconditionally, landing on tier 3
    /// regardless of what any particular linked `FFmpeg` build declares.
    #[test]
    fn tier_three_falls_back_to_aac_for_an_unresolvable_codec() {
        ensure_init_for_test();
        let enc = resolve_declared_codec("definitely_not_a_real_codec_xyz", ContainerFormat::Mp4);
        assert!(
            enc == Some("aac") || enc == Some("libfdk_aac"),
            "expected tier 3 to fall back to an aac-family encoder, got {enc:?}"
        );
    }

    /// Important-2: tier 3 must NOT hand back AAC when the target container
    /// provably cannot carry it — that would recreate the #618 failure class
    /// in reduced form (a thin build without libmp3lame would still resolve
    /// AAC for `.mp3`, which the mp3 muxer rejects). `Wav` is not in the
    /// `aac` row's `supported_containers`, so this is a genuine mismatch, not
    /// a build-availability accident.
    #[test]
    fn tier_three_refuses_rather_than_returning_aac_for_a_container_that_cannot_carry_it() {
        ensure_init_for_test();
        assert!(
            !container_supports_audio_codec(ContainerFormat::Wav, "aac"),
            "test premise: wav must not accept aac"
        );
        let enc = resolve_declared_codec("definitely_not_a_real_codec_xyz", ContainerFormat::Wav);
        assert_eq!(
            enc, None,
            "wav cannot carry aac; tier 3 must refuse rather than substitute it"
        );
    }

    fn ensure_init_for_test() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
    }

    /// Each codec whose own container exists as a variant must list it. The
    /// registry not knowing that FLAC belongs in .flac made
    /// `--recode-audio=flac` into .flac emit a bogus incompatibility warning.
    #[test]
    fn codecs_list_their_own_container() {
        assert!(container_supports_audio_codec(
            ContainerFormat::Flac,
            "flac"
        ));
        assert!(container_supports_audio_codec(ContainerFormat::Wav, "pcm"));
        assert!(container_supports_audio_codec(ContainerFormat::Mp3, "mp3"));
        assert!(container_supports_audio_codec(
            ContainerFormat::Opus,
            "opus"
        ));
        assert!(container_supports_audio_codec(ContainerFormat::Ac3, "ac3"));
        assert!(container_supports_audio_codec(
            ContainerFormat::Wv,
            "wavpack"
        ));
    }

    /// Negative control: the gap-closing must not make everything true. The
    /// sole owner of the "mp4 does not accept vorbis" assertion — it used to
    /// be duplicated verbatim across three tests
    /// (`container_does_not_support_mp4_vorbis`, `compatibility_rejects_vorbis_mp4`,
    /// and this one); the other two were removed.
    #[test]
    fn unrelated_container_codec_pairs_stay_false() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::WebM,
            "flac"
        ));
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            "vorbis"
        ));
    }

    /// Big-endian PCM (AIFF/CAF declare `pcm_s16be`, distinct from the
    /// little-endian `pcm` row's `pcm_s16le`) had no compatibility-matrix
    /// row at all — `container_supports_audio_codec` returned `false` for a
    /// combination `select_audio_encoder_for_container` already produces by
    /// default via tier 2. Keyed to the exact muxer-declared codec-ID name so
    /// it doesn't drift from `muxer_defaults::declared_codec`.
    #[test]
    fn big_endian_pcm_containers_recognized() {
        assert!(container_supports_audio_codec(
            ContainerFormat::Aiff,
            "pcm_s16be"
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Caf,
            "pcm_s16be"
        ));
    }

    /// WMA had no compatibility-matrix row at all, even though `.wma`/`.wmv`/
    /// `.asf` all declare `wmav2` as their default audio codec.
    #[test]
    fn wma_family_containers_recognized() {
        assert!(container_supports_audio_codec(
            ContainerFormat::Wma,
            "wmav2"
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Wmv,
            "wmav2"
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Asf,
            "wmav2"
        ));
    }

    /// Negative control for the two new rows: they must not leak into
    /// unrelated containers.
    #[test]
    fn new_rows_do_not_leak_into_unrelated_containers() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            "pcm_s16be"
        ));
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            "wmav2"
        ));
    }

    /// The falsifiable form of "the matrix must not disagree with a
    /// container's own default audio codec". For every `ContainerFormat`,
    /// derives the expected codec from `audio_default_for` — the same policy
    /// `select_audio_encoder_for_container` uses — and cross-checks it
    /// against `container_supports_audio_codec`, i.e. against the static
    /// table read by a genuinely independent source
    /// (`muxer_defaults::declared_codec`, which asks the linked `FFmpeg`
    /// build's muxers directly) for the `FromMuxer` containers. A key typo,
    /// or a container missing from a row, fails this test by naming the
    /// exact container and codec — no hardcoded pair to keep in sync by
    /// hand.
    #[test]
    fn every_containers_own_default_codec_is_accepted_by_the_matrix() {
        use strum::IntoEnumIterator;
        ensure_init_for_test();

        for container in ContainerFormat::iter() {
            let expected = match audio_default_for(container) {
                AudioDefault::NoAudio => {
                    // The sweep's one unguarded skip: nothing else pins the
                    // `NoAudio` *classification* itself (only
                    // `ivf_carries_no_audio` checks the one known instance).
                    // A container wrongly classified `NoAudio` here would
                    // otherwise be skipped by this sweep and caught by
                    // nothing. Cross-check against an INDEPENDENT oracle —
                    // FFmpeg's own muxer table via `muxer_defaults` — rather
                    // than `select_audio_encoder_for_container`, which itself
                    // returns `None` for `NoAudio` via this same
                    // `audio_default_for` call: asserting against that would
                    // be one classification read twice, not a cross-check.
                    // Residual gap: `declared_codec`'s own doc warns `None`
                    // has three causes, only one of which is "no audio slot
                    // at all" — an ABI-skew `None` (muxer declares a codec id
                    // this build's libavcodec can't name) would let a
                    // misclassified `NoAudio` through this cross-check too.
                    assert!(
                        muxer_defaults::declared_codec(container, codec_registry::MediaKind::Audio)
                            .is_none(),
                        "{container:?} is classified NoAudio but its muxer \
                         declares an audio codec"
                    );
                    continue;
                }
                AudioDefault::Override(codec) => codec,
                AudioDefault::FromMuxer => {
                    let Some(codec) =
                        muxer_defaults::declared_codec(container, codec_registry::MediaKind::Audio)
                    else {
                        // No muxer claims the extension, or the declared id
                        // is unrepresentable in this build — nothing to
                        // cross-check against for this container.
                        continue;
                    };
                    codec
                }
            };

            assert!(
                container_supports_audio_codec(container, expected),
                "{container:?}'s own default audio codec {expected:?} is not \
                 accepted by its own compatibility-matrix row"
            );
        }
    }

    /// Falsifiable, generalising form of the CRITICAL-8 regression guard:
    /// for every row, every declared alias must resolve (via
    /// `preferred_audio_encoder`) to the exact same encoder as the row's own
    /// primary codec key. Catches not just today's `pcm` alias but any
    /// future alias added to any row without per-alias test coverage.
    #[test]
    fn every_alias_resolves_to_its_rows_own_encoder() {
        use codec_registry::CodecRow;
        ensure_init_for_test();

        for row in AUDIO_CODEC_PREFERENCES {
            let primary = preferred_audio_encoder(row.codec());
            for alias in row.aliases() {
                // `assert_eq!` below passes vacuously when `primary` is
                // `None` (e.g. a row whose encoders are all absent from this
                // FFmpeg build) — guard against that so the equality check
                // is only ever trusted when it had something to compare.
                assert!(
                    primary.is_some(),
                    "alias {alias:?} is declared on codec {:?}, whose own \
                     encoders are unavailable in this build — the equality \
                     below would pass vacuously",
                    row.codec()
                );
                assert_eq!(
                    preferred_audio_encoder(alias),
                    primary,
                    "alias {alias:?} of codec {:?} must resolve to the same \
                     encoder as the primary key",
                    row.codec()
                );
            }
        }
    }
}
