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
use rdlp_types::RecodeAudioMode;
use rdlp_types::media_name::{AudioCodecOrEncoderName, AudioEncoder, AudioEncoderName, CodecName};
use serde::{Deserialize, Serialize};

use crate::ffmpeg::container_default::{ContainerDefault, Policy};
use crate::ffmpeg::source::Audio;
use crate::ffmpeg::{codec_registry, muxer_defaults};

/// The codec tier 3 substitutes when neither the preference table nor a
/// literal encoder-name match resolved anything.
///
/// AAC because it is the one audio codec essentially every general-purpose
/// delivery container carries and every build can encode. Named rather than
/// repeated as a literal at each of its two sites: it encodes a *policy*
/// ("when all else fails, this"), and two spellings can drift — including
/// out of sync with the `container_supports_audio_codec` gate that decides
/// whether the substitution is legal at all.
///
/// A `const` item bound to `CodecName::AAC`, so the name is validated at
/// compile time (via that const's `from_static`); constructing it inline in a
/// function body would instead be a runtime call that panics on a bad name.
const FALLBACK_AUDIO_CODEC: CodecName = CodecName::AAC;

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
    codec: CodecName,
    /// Human-readable display name.
    display_name: &'static str,
    /// Ordered encoder preference list: (`encoder_name`, `display_name`).
    encoders: &'static [(AudioEncoderName, &'static str)],
    /// Container formats this codec is compatible with.
    supported_containers: &'static [ContainerFormat],
    /// Alternate names that should also resolve to this row. Declared next
    /// to the row it describes rather than in a separate `match self.codec`
    /// keyed by string — that indirection let a rename of `codec` silently
    /// drop the alias through the `CodecRow::aliases` trait default. Empty
    /// for every row but `pcm_s16le`.
    aliases: &'static [CodecName],
}

impl codec_registry::CodecRow for AudioCodecEntry {
    type Encoder = AudioEncoder;
    fn codec(&self) -> &CodecName {
        &self.codec
    }
    fn encoders(&self) -> &'static [(AudioEncoderName, &'static str)] {
        self.encoders
    }
    fn aliases(&self) -> &'static [CodecName] {
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
        codec: CodecName::AAC,
        display_name: "AAC",
        encoders: &[
            (
                AudioEncoderName::from_static("libfdk_aac"),
                "FDK AAC (libfdk_aac)",
            ),
            (AudioEncoderName::from_static("aac"), "AAC (built-in)"),
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
            // Flv: `flvenc.c` `flv_audio_codec_ids` tags AAC (FLV_CODECID_AAC).
            // Nut: `ff_codec_wav_tags` 0x00ff, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Flv,
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::MP3,
        display_name: "MP3",
        encoders: &[(
            AudioEncoderName::from_static("libmp3lame"),
            "MP3 (libmp3lame)",
        )],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Avi,
            ContainerFormat::Ts,
            ContainerFormat::Mp3,
            // Flv's muxer declares mp3 as its default audio codec.
            ContainerFormat::Flv,
            // `ff_nut_audio_extra_tags` MP3, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::OPUS,
        display_name: "Opus",
        encoders: &[(AudioEncoderName::from_static("libopus"), "Opus (libopus)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mkv,
            // Mka: rdlp overrides matroska's own default to opus (see
            // `audio_default_for`); the matrix must accept it.
            ContainerFormat::Mka,
            ContainerFormat::WebM,
            ContainerFormat::Ogg,
            ContainerFormat::Opus,
            // `ff_nut_audio_extra_tags` OPUS, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::VORBIS,
        display_name: "Vorbis",
        encoders: &[(
            AudioEncoderName::from_static("libvorbis"),
            "Vorbis (libvorbis)",
        )],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::WebM,
            ContainerFormat::Ogg,
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::FLAC,
        display_name: "FLAC",
        encoders: &[(AudioEncoderName::from_static("flac"), "FLAC (built-in)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::Ogg,
            ContainerFormat::Flac,
            // `ff_codec_wav_tags` 0xF1AC, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::from_static("alac"),
        display_name: "ALAC",
        encoders: &[(AudioEncoderName::from_static("alac"), "ALAC (built-in)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::AC3,
        display_name: "AC-3 (Dolby Digital)",
        encoders: &[(AudioEncoderName::from_static("ac3"), "AC-3 (built-in)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Avi,
            ContainerFormat::Ts,
            ContainerFormat::Ac3,
            // `ff_codec_wav_tags` 0x2000, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::EAC3,
        display_name: "E-AC-3 (Dolby Digital Plus)",
        encoders: &[(AudioEncoderName::from_static("eac3"), "E-AC-3 (built-in)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mov,
            ContainerFormat::Mkv,
            ContainerFormat::Ts,
            // `ff_codec_wav_tags` 0x2000 (shared with AC-3), reached by
            // `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::from_static("dts"),
        display_name: "DTS",
        encoders: &[(AudioEncoderName::from_static("dca"), "DTS (built-in)")],
        // `ff_codec_wav_tags` 0x2001, reached by `ff_nut_codec_tags` (nut.c).
        supported_containers: &[ContainerFormat::Mkv, ContainerFormat::Nut],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::MP2,
        display_name: "MP2",
        encoders: &[(AudioEncoderName::from_static("mp2"), "MP2 (built-in)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::Ts,
            // Mpg/Vob's muxers both declare mp2 as their default audio codec.
            ContainerFormat::Mpg,
            ContainerFormat::Vob,
            // `ff_codec_wav_tags` 0x0050, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
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
        codec: CodecName::from_static("pcm_s16le"),
        display_name: "PCM 16-bit little-endian",
        encoders: &[(
            AudioEncoderName::from_static("pcm_s16le"),
            "PCM 16-bit (built-in)",
        )],
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
            // `ff_nut_audio_tags` PCM, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[CodecName::from_static("pcm")],
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
        codec: CodecName::from_static("pcm_s16be"),
        display_name: "PCM 16-bit big-endian",
        encoders: &[(
            AudioEncoderName::from_static("pcm_s16be"),
            "PCM 16-bit big-endian (built-in)",
        )],
        // `ff_nut_audio_tags` PCM, reached by `ff_nut_codec_tags` (nut.c).
        supported_containers: &[
            ContainerFormat::Aiff,
            ContainerFormat::Caf,
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::from_static("wavpack"),
        display_name: "WavPack",
        encoders: &[(
            AudioEncoderName::from_static("wavpack"),
            "WavPack (built-in)",
        )],
        // `ff_nut_audio_extra_tags` WAVPACK, reached by `ff_nut_codec_tags` (nut.c).
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::Wv,
            ContainerFormat::Nut,
        ],
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
        codec: CodecName::from_static("wmav2"),
        display_name: "WMA v2",
        encoders: &[(
            AudioEncoderName::from_static("wmav2"),
            "Windows Media Audio 2 (built-in)",
        )],
        supported_containers: &[
            ContainerFormat::Wma,
            ContainerFormat::Wmv,
            ContainerFormat::Asf,
            // `ff_codec_wav_tags` 0x0161, reached by `ff_nut_codec_tags` (nut.c).
            ContainerFormat::Nut,
        ],
        aliases: &[],
    },
    AudioCodecEntry {
        codec: CodecName::from_static("tta"),
        display_name: "TTA",
        encoders: &[(AudioEncoderName::from_static("tta"), "TTA (built-in)")],
        supported_containers: &[ContainerFormat::Mkv],
        aliases: &[],
    },
];

/// The audio codec registry, owning its own cache over [`AUDIO_CODEC_PREFERENCES`].
static AUDIO_REGISTRY: codec_registry::Registry<AudioCodecEntry> =
    codec_registry::Registry::new(AUDIO_CODEC_PREFERENCES, codec_registry::MediaKind::Audio);

/// Returns `true` if the named audio encoder is available in the current `FFmpeg` build.
///
/// Takes an [`AudioEncoderName`] rather than a bare `&str` so a codec name
/// cannot be checked as if it were an encoder name by accident (#642's A1).
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn is_audio_encoder_available(encoder: &AudioEncoderName) -> bool {
    codec_registry::is_encoder_available(encoder.as_str())
}

/// Returns the best available audio encoder for a given codec name.
///
/// Results are cached via a `OnceLock<HashMap>` so detection only runs once.
/// Returns `None` if no encoder is available or the codec is unknown.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn preferred_audio_encoder(codec: &str) -> Option<AudioEncoderName> {
    AUDIO_REGISTRY.preferred_encoder(codec)
}

/// Resolves an operator's audio request to an encoder.
///
/// The request is either a codec name (e.g., "aac") or a direct encoder name
/// (e.g., "`libfdk_aac`") — [`AudioCodecOrEncoderName`] carries exactly that
/// ambiguity, and this is the one point where it is resolved, against the
/// linked build. For codec names, returns the best available encoder via
/// [`preferred_audio_encoder`]; for encoder names, checks availability
/// directly. The order is codec-first, deliberately — see
/// `Registry::resolve` (#649).
///
/// Returns `None` if no encoder can be resolved or is available.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn resolve_audio_encoder(request: &AudioCodecOrEncoderName) -> Option<AudioEncoderName> {
    AUDIO_REGISTRY.resolve(request.as_str())
}

/// Pre-flight an operator's `recode_audio` request against the linked build.
///
/// An unknown codec/encoder name then fails at the configuration boundary
/// with a clear message rather than deep inside a recode (#649). Mirrors the
/// wording of `FFmpeg`'s own `find_codec` failure (`Unknown encoder '%s'`).
///
/// `None` (not specified), `Copy` and `Auto` have nothing to check.
///
/// # Errors
///
/// The message to show the operator when the name resolves to no available
/// encoder.
pub fn validate_recode_audio(mode: Option<&RecodeAudioMode>) -> std::result::Result<(), String> {
    match mode {
        Some(RecodeAudioMode::Encoder { name }) if resolve_audio_encoder(name).is_none() => {
            Err(format!(
                "unknown audio codec or encoder '{name}' (not in this FFmpeg build); \
                 valid values are copy, auto, a codec name such as aac or opus, or an \
                 encoder name such as libopus"
            ))
        }
        _ => Ok(()),
    }
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
                    encoder_name: enc.as_str().to_string(),
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
                    encoder_name: enc.as_str().to_string(),
                    display_name: display.to_string(),
                })
                .collect();

            if encoders.is_empty() {
                None
            } else {
                Some(AudioCodecInfo {
                    codec: entry.codec.as_str().to_string(),
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
/// Takes a [`CodecName`] rather than a bare `&str` so a value from the wrong
/// vocabulary (an encoder name, an RFC 6381 manifest string) cannot be asked
/// about by accident (#642's A1).
///
/// # Examples
///
/// ```no_run
/// use rdlp_ffmpeg::ffmpeg::audio_encoder_registry::container_supports_audio_codec;
/// use rdlp_types::ContainerFormat;
/// use rdlp_types::media_name::CodecName;
///
/// assert!(container_supports_audio_codec(
///     ContainerFormat::Mp4,
///     &CodecName::from_static("aac")
/// ));
/// assert!(!container_supports_audio_codec(
///     ContainerFormat::Mp4,
///     &CodecName::from_static("vorbis")
/// ));
/// assert!(container_supports_audio_codec(
///     ContainerFormat::WebM,
///     &CodecName::from_static("opus")
/// ));
/// assert!(!container_supports_audio_codec(
///     ContainerFormat::Mov,
///     &CodecName::from_static("opus")
/// ));
/// ```
#[must_use]
pub fn container_supports_audio_codec(container: ContainerFormat, codec: &CodecName) -> bool {
    AUDIO_REGISTRY
        .find_row(codec.as_str())
        .is_some_and(|entry| entry.supported_containers.contains(&container))
}

/// Policy for each container. Exhaustive with no `_` arm: a new
/// [`ContainerFormat`] variant must be classified here or the build fails.
const fn audio_default_for(container: ContainerFormat) -> ContainerDefault<Audio> {
    match container {
        // Mkv/Mka: Opus over the matroska muxer's historical `vorbis`
        // declaration — better quality per bit, and Matroska carries Opus
        // without caveat.
        //
        // Ogg is deliberately NOT here (#623): `.ogg` "applies now for
        // Vorbis I files only" (Xiph, MIME Types and File Extensions — kept
        // for hardware players that treat `.ogg` as Vorbis), and RFC 7845 §9
        // recommends `.opus` for Ogg Opus, which rdlp offers as
        // `ContainerFormat::Opus`. So `.ogg` defers to the muxer's declared
        // default, vorbis, like the #538 rule: the extension the user asked
        // for wins over an internal preference.
        ContainerFormat::Mkv | ContainerFormat::Mka => {
            ContainerDefault::new(Policy::Override(CodecName::OPUS))
        }

        // FFmpeg declares `amr_nb`: 8 kHz speech-only, and the native encoder
        // is absent from most builds. 3GPP TS 26.244 permits AMR, AMR-WB and
        // AAC, so AAC is spec-correct rather than a workaround.
        ContainerFormat::ThreeGp => ContainerDefault::new(Policy::Override(CodecName::AAC)),

        // Raw VP8/VP9/AV1 elementary stream; the muxer declares no audio codec
        // and refuses any audio stream outright.
        ContainerFormat::Ivf => ContainerDefault::new(Policy::NotATarget),

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
        | ContainerFormat::Ogg
        | ContainerFormat::Opus
        | ContainerFormat::Aac
        | ContainerFormat::Aiff
        | ContainerFormat::Wv
        | ContainerFormat::Caf
        | ContainerFormat::Ac3 => ContainerDefault::new(Policy::FromMuxer),
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
pub fn select_audio_encoder_for_container(container: ContainerFormat) -> Option<AudioEncoderName> {
    let default = audio_default_for(container);
    let codec = match default.policy() {
        Policy::NotATarget => return None,
        Policy::Override(codec) => codec.clone(),
        Policy::FromMuxer => {
            let Some(codec) =
                muxer_defaults::declared_codec(container, codec_registry::MediaKind::Audio)
            else {
                // Every other `None` path logs (the ABI-skew warn inside
                // `declared_codec`, the tier-3 AAC-fallback warn below) or is
                // a declared policy (`Policy::NotATarget`). "No muxer claims
                // this extension" is the one case that would otherwise pass
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

    resolve_declared_codec(&codec, container)
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
/// Tiers 1 and 2 additionally skip an encoder `FFmpeg` marks experimental
/// (#625): this is an *automatic* choice, and such an encoder would fail at
/// `avcodec_open2` on the default compliance level. An explicit request for
/// the same codec (`resolve_audio_encoder`) still resolves it — the gate is
/// lifted at open time for explicit requests only (#639,
/// `codec_registry::enable_experimental_if_flagged`).
fn resolve_declared_codec(
    codec: &CodecName,
    container: ContainerFormat,
) -> Option<AudioEncoderName> {
    let selectable = |enc: AudioEncoderName| {
        (!codec_registry::is_experimental_encoder(enc.as_str())).then_some(enc)
    };
    preferred_audio_encoder(codec.as_str())
        .and_then(selectable)
        .or_else(|| {
            // `codec` is being asked about as an ENCODER name here — a
            // deliberate vocabulary crossing (native single-encoder codecs
            // like PCM/WMA have no preference-table row because the codec-ID
            // name IS the encoder name), sanctioned via `retag` rather than a
            // reparse-and-`expect` round trip. Infallible and allocation-free:
            // `retag` reinterprets the same validated bytes, preserving
            // whichever `Cow` variant `codec` already held.
            let as_encoder: AudioEncoderName = codec.clone().retag();
            is_audio_encoder_available(&as_encoder)
                .then_some(as_encoder)
                .and_then(selectable)
        })
        .or_else(|| {
            // Neither the preference table nor a direct name match resolved
            // an available encoder. AAC is only a sane fallback when the
            // container can actually carry it — substituting it into a
            // container that provably cannot (e.g. `.mp3` on a build without
            // libmp3lame) would recreate the #618 failure class in reduced
            // form. Refuse truthfully instead; the caller already turns a
            // `None` here into `RecodeStage`'s/normalize's honest refusal.
            if !container_supports_audio_codec(container, &FALLBACK_AUDIO_CODEC) {
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
            preferred_audio_encoder(FALLBACK_AUDIO_CODEC.as_str())
        })
}

/// Test-only sugar shared by this crate's test modules: the selected
/// encoder's name, or `None`. `MediaName` deliberately has no `Deref<str>`
/// (see its module doc), so this explicit accessor replaces the
/// `.as_ref().map(MediaName::as_str)` chain that every assertion repeated.
#[cfg(test)]
pub(crate) mod test_ext {
    use rdlp_types::media_name::AudioEncoderName;

    // `pub`, not `pub(crate)`: the module is `pub(crate)`, so this is still
    // crate-internal, and clippy's `redundant_pub_crate` rejects the narrower
    // spelling (same rule as `KNOWN_UNDECLARED_SUPPORT`).
    pub trait EncoderNameExt {
        fn name(&self) -> Option<&str>;
    }

    impl EncoderNameExt for Option<AudioEncoderName> {
        fn name(&self) -> Option<&str> {
            self.as_ref().map(AudioEncoderName::as_str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_ext::EncoderNameExt;
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
        let enc = resolve_audio_encoder(&req("aac"));
        assert!(enc.is_some(), "should resolve aac codec");
    }

    /// The tier-3 fallback is one named constant, not three literals. Guards the
    /// #618 failure class: three independent spellings can drift, and the tier-3
    /// gate (`container_supports_audio_codec`) must ask about the same codec the
    /// fallback actually resolves.
    #[test]
    fn the_fallback_codec_is_the_one_tier_three_resolves() {
        ensure_init_for_test();
        assert!(
            preferred_audio_encoder(FALLBACK_AUDIO_CODEC.as_str()).is_some(),
            "the named fallback must actually resolve an encoder"
        );
    }

    /// CRITICAL-8 regression guard: `resolve_audio_encoder(&req("pcm"))` must
    /// resolve via the `pcm_s16le` row's alias, not fall through to `None`.
    /// `pcm_s16le` is a built-in `FFmpeg` encoder present in every build (the
    /// same assumption `audio_only_containers_get_their_own_codec_not_aac`
    /// already makes unguarded), so this is safe to assert unconditionally.
    #[test]
    fn resolve_encoder_by_alias_pcm() {
        ensure_init_for_test();
        assert_eq!(resolve_audio_encoder(&req("pcm")).name(), Some("pcm_s16le"));
        assert_eq!(preferred_audio_encoder("pcm").name(), Some("pcm_s16le"));
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
        if is_audio_encoder_available(&AudioEncoderName::from_static("libmp3lame")) {
            assert_eq!(
                resolve_audio_encoder(&req("libmp3lame")).name(),
                Some("libmp3lame")
            );
        } else {
            assert_eq!(resolve_audio_encoder(&req("libmp3lame")), None);
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
        assert!(container_supports_audio_codec(
            ContainerFormat::Mp4,
            &CodecName::from_static("opus")
        ));
    }

    #[test]
    fn container_does_not_support_mov_opus() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mov,
            &CodecName::from_static("opus")
        ));
    }

    #[test]
    fn select_encoder_for_mp4_returns_aac() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let enc = select_audio_encoder_for_container(ContainerFormat::Mp4);
        let enc = enc.name();
        assert!(
            enc == Some("aac") || enc == Some("libfdk_aac"),
            "expected aac encoder for mp4, got {enc:?}"
        );
    }

    #[test]
    fn compatibility_accepts_opus_mkv() {
        assert!(container_supports_audio_codec(
            ContainerFormat::Mkv,
            &CodecName::from_static("opus")
        ));
    }

    #[test]
    fn unknown_codec_returns_false() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            &CodecName::from_static("nonexistent_codec")
        ));
    }

    /// The operator's request type, built the way the CLI builds it.
    fn req(name: &str) -> AudioCodecOrEncoderName {
        AudioCodecOrEncoderName::new(name).unwrap()
    }

    /// #649: for a name that is both a codec and a native encoder, the codec
    /// preference wins — `opus` resolves to `libopus`, never to the native
    /// `opus` encoder (experimental, `libavcodec/opus/enc.c`), which is what
    /// `FFmpeg`'s encoder-name-first `-c:a` would land on. Pinned so a
    /// "parity" flip cannot silently degrade the request. Gated on libopus
    /// being linked; a build without it has no better answer to prefer.
    #[test]
    fn codec_name_that_is_also_a_native_encoder_resolves_to_the_preferred_encoder() {
        ensure_init_for_test();
        if is_audio_encoder_available(&AudioEncoderName::from_static("libopus")) {
            assert_eq!(resolve_audio_encoder(&req("opus")).name(), Some("libopus"));
        }
        if is_audio_encoder_available(&AudioEncoderName::from_static("libvorbis")) {
            assert_eq!(
                resolve_audio_encoder(&req("vorbis")).name(),
                Some("libvorbis")
            );
        }
    }

    /// #649: the pre-flight refuses an unknown name with an actionable
    /// message and passes everything that has nothing to check.
    #[test]
    fn validate_recode_audio_refuses_only_an_unresolvable_name() {
        ensure_init_for_test();
        assert_eq!(validate_recode_audio(None), Ok(()));
        assert_eq!(validate_recode_audio(Some(&RecodeAudioMode::Copy)), Ok(()));
        assert_eq!(validate_recode_audio(Some(&RecodeAudioMode::Auto)), Ok(()));
        let known = RecodeAudioMode::Encoder { name: req("pcm") };
        assert_eq!(validate_recode_audio(Some(&known)), Ok(()));
        let unknown = RecodeAudioMode::Encoder {
            name: req("nonexistent_codec_xyz"),
        };
        let err = validate_recode_audio(Some(&unknown)).unwrap_err();
        assert!(
            err.contains("nonexistent_codec_xyz") && err.contains("copy, auto"),
            "{err}"
        );
    }

    #[test]
    fn unknown_codec_resolve_returns_none() {
        assert!(resolve_audio_encoder(&req("nonexistent_codec_xyz")).is_none());
    }

    #[test]
    fn registered_but_unavailable_encoder_resolves_none() {
        // `libfdk_aac` is registered in AUDIO_CODEC_PREFERENCES but is a nonfree
        // encoder usually absent from a stock ffmpeg build. When present it
        // resolves to itself; when absent the name matches yet the availability
        // gate rejects it → None. Pins the `.and_then(is_available.then_some(..))`
        // unavailable branch of the iterator chain.
        if is_audio_encoder_available(&AudioEncoderName::from_static("libfdk_aac")) {
            assert_eq!(
                resolve_audio_encoder(&req("libfdk_aac")).name(),
                Some("libfdk_aac")
            );
        } else {
            assert_eq!(resolve_audio_encoder(&req("libfdk_aac")), None);
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
        let mp3 = mp3.name();
        assert!(
            mp3 == Some("libmp3lame")
                || !is_audio_encoder_available(&AudioEncoderName::from_static("libmp3lame")),
            "expected libmp3lame for .mp3 when available, got {mp3:?}"
        );
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Flac).name(),
            Some("flac")
        );
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Wav).name(),
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
            let enc = enc.name();
            assert!(
                enc == Some("wmav2")
                    || !is_audio_encoder_available(&AudioEncoderName::from_static("wmav2")),
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

    /// The three overrides, asserted against literal encoder names — NOT
    /// against `declared_codec(..)`, which would be a tautology that
    /// survives mutating the override away. Guarded the same honest way as
    /// `asf_family_gets_wmav2`: only asserted when the encoder is
    /// actually available, since tier 3's AAC fallback is correct by design
    /// otherwise.
    #[test]
    fn overrides_are_exactly_these_three() {
        use strum::IntoEnumIterator;

        ensure_init_for_test();
        for c in [ContainerFormat::Mkv, ContainerFormat::Mka] {
            let enc = select_audio_encoder_for_container(c);
            let enc = enc.name();
            assert!(
                enc == Some("libopus")
                    || !is_audio_encoder_available(&AudioEncoderName::from_static("libopus")),
                "{c:?}: expected libopus when available, got {enc:?}"
            );
        }

        let threegp = select_audio_encoder_for_container(ContainerFormat::ThreeGp);

        let threegp = threegp.name();
        assert!(
            threegp == Some("aac") || threegp == Some("libfdk_aac"),
            "expected an aac-family encoder for 3gp, got {threegp:?}"
        );

        // The "exactly" half: no OTHER container may be classified
        // `Policy::Override`. Without this, adding e.g.
        // `Avi => Override("opus")` to `audio_default_for` would leave the
        // assertions above green.
        let known_overrides = [
            ContainerFormat::Mkv,
            ContainerFormat::Mka,
            ContainerFormat::ThreeGp,
        ];
        for container in ContainerFormat::iter() {
            let is_override = matches!(audio_default_for(container).policy(), Policy::Override(_));
            assert_eq!(
                is_override,
                known_overrides.contains(&container),
                "{container:?}: Policy::Override classification does not \
                 match the known set of three overrides"
            );
        }
    }

    /// #623: `.ogg` means Ogg Vorbis (Xiph; RFC 7845 §9 gives Opus its own
    /// `.opus`), so its automatic default is the muxer's declared vorbis —
    /// `libvorbis` when linked; the native `vorbis` encoder is experimental
    /// (#625) and Ogg cannot carry the AAC fallback, so a build without
    /// libvorbis refuses honestly rather than writing Opus into `.ogg`.
    #[test]
    fn ogg_defaults_to_vorbis_not_opus() {
        ensure_init_for_test();
        let enc = select_audio_encoder_for_container(ContainerFormat::Ogg);
        let enc = enc.name();
        if is_audio_encoder_available(&AudioEncoderName::from_static("libvorbis")) {
            assert_eq!(enc, Some("libvorbis"));
        } else {
            assert_eq!(enc, None, "no libvorbis: refuse, never substitute opus/aac");
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

        let ts = ts.name();
        assert!(
            ts == Some("mp2") || !is_audio_encoder_available(&AudioEncoderName::from_static("mp2")),
            "expected mp2 for .ts when available, got {ts:?}"
        );

        let nut = select_audio_encoder_for_container(ContainerFormat::Nut);

        let nut = nut.name();
        assert!(
            nut == Some("libvorbis")
                || !is_audio_encoder_available(&AudioEncoderName::from_static("libvorbis")),
            "expected libvorbis for .nut when available, got {nut:?}"
        );

        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Ac3).name(),
            Some("ac3")
        );
        assert_eq!(
            select_audio_encoder_for_container(ContainerFormat::Aiff).name(),
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
            let enc = enc.name();
            assert!(
                enc == Some("aac") || enc == Some("libfdk_aac"),
                "expected an aac-family encoder for {c:?}, got {enc:?}"
            );
        }
    }

    /// Tier 3 (the AAC-with-warning fallback the whole design hinges on
    /// being VISIBLE) had no test — tiers 1-2 were covered, tier 3 wasn't.
    /// #625: an automatic default must never resolve to an encoder `FFmpeg`
    /// marks `AV_CODEC_CAP_EXPERIMENTAL` — `avcodec_open2` refuses it at the
    /// default compliance level (`libavcodec/avcodec.c`, "The encoder '%s'
    /// is experimental"). `dts`'s only encoder is the native `dca`, which
    /// carries the flag (`dcaenc.c`), so the declared codec must fall past
    /// tiers 1 and 2 to the AAC fallback (Mkv carries AAC).
    #[test]
    fn automatic_default_skips_an_experimental_only_encoder() {
        ensure_init_for_test();
        let dts = CodecName::from_static("dts");
        assert!(
            codec_registry::is_experimental_encoder("dca"),
            "precondition: this build's dca is the experimental native encoder"
        );
        let enc = resolve_declared_codec(&dts, ContainerFormat::Mkv);
        let enc = enc.name();
        assert!(
            enc == Some("aac") || enc == Some("libfdk_aac"),
            "expected the experimental dca to be skipped in favour of the AAC \
             fallback, got {enc:?}"
        );
    }

    /// An explicit request for the codec still resolves its encoder; the
    /// experimental gate is lifted at open time instead (#639).
    #[test]
    fn explicit_resolution_still_returns_the_experimental_encoder() {
        ensure_init_for_test();
        let enc = resolve_audio_encoder(&req("dts"));
        assert_eq!(enc.name(), Some("dca"));
    }

    /// A bogus codec name skips the preference table (tier 1) and the
    /// direct-name check (tier 2) unconditionally, landing on tier 3
    /// regardless of what any particular linked `FFmpeg` build declares.
    #[test]
    fn tier_three_falls_back_to_aac_for_an_unresolvable_codec() {
        ensure_init_for_test();
        let bogus = CodecName::new("definitely_not_a_real_codec_xyz").expect("valid name");
        let enc = resolve_declared_codec(&bogus, ContainerFormat::Mp4);
        let enc = enc.name();
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
            !container_supports_audio_codec(ContainerFormat::Wav, &CodecName::from_static("aac")),
            "test premise: wav must not accept aac"
        );
        let bogus = CodecName::new("definitely_not_a_real_codec_xyz").expect("valid name");
        let enc = resolve_declared_codec(&bogus, ContainerFormat::Wav);
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
            &CodecName::from_static("flac")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Wav,
            &CodecName::from_static("pcm")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Mp3,
            &CodecName::MP3
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Opus,
            &CodecName::from_static("opus")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Ac3,
            &CodecName::from_static("ac3")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Wv,
            &CodecName::from_static("wavpack")
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
            &CodecName::from_static("flac")
        ));
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            &CodecName::from_static("vorbis")
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
            &CodecName::from_static("pcm_s16be")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Caf,
            &CodecName::from_static("pcm_s16be")
        ));
    }

    /// WMA had no compatibility-matrix row at all, even though `.wma`/`.wmv`/
    /// `.asf` all declare `wmav2` as their default audio codec.
    #[test]
    fn wma_family_containers_recognized() {
        assert!(container_supports_audio_codec(
            ContainerFormat::Wma,
            &CodecName::from_static("wmav2")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Wmv,
            &CodecName::from_static("wmav2")
        ));
        assert!(container_supports_audio_codec(
            ContainerFormat::Asf,
            &CodecName::from_static("wmav2")
        ));
    }

    /// Negative control for the two new rows: they must not leak into
    /// unrelated containers.
    #[test]
    fn new_rows_do_not_leak_into_unrelated_containers() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            &CodecName::from_static("pcm_s16be")
        ));
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            &CodecName::from_static("wmav2")
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
            let default = audio_default_for(container);
            let expected = match default.policy() {
                Policy::NotATarget => {
                    // The sweep's one unguarded skip: nothing else pins the
                    // `NotATarget` *classification* itself (only
                    // `ivf_carries_no_audio` checks the one known instance).
                    // A container wrongly classified `NotATarget` here would
                    // otherwise be skipped by this sweep and caught by
                    // nothing. Cross-check against an INDEPENDENT oracle —
                    // FFmpeg's own muxer table via `muxer_defaults` — rather
                    // than `select_audio_encoder_for_container`, which itself
                    // returns `None` for `NotATarget` via this same
                    // `audio_default_for` call: asserting against that would
                    // be one classification read twice, not a cross-check.
                    // Residual gap: `declared_codec`'s own doc warns `None`
                    // has three causes, only one of which is "no audio slot
                    // at all" — an ABI-skew `None` (muxer declares a codec id
                    // this build's libavcodec can't name) would let a
                    // misclassified `NotATarget` through this cross-check too.
                    //
                    // Why this cross-check is still meaningful rather than
                    // circular: for the audio kind, `NotATarget` is currently
                    // capability-backed (`Ivf` is the only member, and it
                    // genuinely has no audio slot), so `declared_codec`
                    // returning `None` here is an independent fact about the
                    // linked build, not a restatement of `audio_default_for`'s
                    // own policy choice. If a future container is classified
                    // `NotATarget` purely as a policy decision (the muxer
                    // *does* declare an audio codec, rdlp just declines to
                    // target it — mirroring the video side's `M4a`/`Wma`
                    // policy-not-capability arms), this assertion would fail
                    // for that container even though the classification is
                    // correct, because "the variant's documented meaning is
                    // policy" is then weaker than what this assertion checks.
                    assert!(
                        muxer_defaults::declared_codec(container, codec_registry::MediaKind::Audio)
                            .is_none(),
                        "{container:?} is classified NotATarget but its muxer \
                         declares an audio codec"
                    );
                    continue;
                }
                Policy::Override(codec) => codec.clone(),
                Policy::FromMuxer => {
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
                container_supports_audio_codec(container, &expected),
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
            let primary = preferred_audio_encoder(row.codec().as_str());
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
                    preferred_audio_encoder(alias.as_str()),
                    primary,
                    "alias {alias:?} of codec {:?} must resolve to the same \
                     encoder as the primary key",
                    row.codec()
                );
            }
        }
    }
}

#[cfg(test)]
mod matrix_soundness {
    use super::*;
    use strum::IntoEnumIterator;

    /// Every cell of `AUDIO_CODEC_PREFERENCES` must be a pairing the linked
    /// muxer actually accepts, as judged by the same tri-state oracle the
    /// remux path enforces with (#633). The matrix is deliberately narrower
    /// than the muxers — it lists sensible targets, not every representable
    /// one — but it must never be *wider*: a cell the muxer rejects routes a
    /// user to a mux that fails (#627 found `Flv`/`Nut` missing; this sweep
    /// found five listed cells the oracle rejected, fixed by teaching the
    /// #633 evidence table what `mpegtsenc.c` and `oggenc.c` implement).
    #[test]
    fn every_matrix_cell_is_accepted_by_its_muxer() {
        crate::ffmpeg::ensure_init().expect("ffmpeg init");
        let mut rejected = Vec::new();
        for entry in AUDIO_CODEC_PREFERENCES {
            for container in ContainerFormat::iter() {
                let listed = entry.supported_containers.contains(&container);
                if listed
                    && !muxer_defaults::muxer_can_represent(
                        container,
                        &entry.codec,
                        codec_registry::MediaKind::Audio,
                    )
                {
                    rejected.push(format!("{}/{}", entry.codec.as_str(), container.as_ext()));
                }
            }
        }
        assert!(
            rejected.is_empty(),
            "matrix cells the muxer rejects: {rejected:?}"
        );
    }

    /// #627's two named gaps, pinned so they cannot be dropped by a later
    /// "tidy" of the table: Flv carries AAC, and Nut carries every codec that
    /// has a tag in `ff_nut_codec_tags` (aac, mp3, opus, vorbis, flac, ac3,
    /// eac3, dts, mp2 here — NOT alac, which has no NUT tag).
    #[test]
    fn flv_carries_aac_and_nut_carries_every_tagged_codec() {
        let aac = &CodecName::AAC;
        assert!(container_supports_audio_codec(ContainerFormat::Flv, aac));
        for codec in [
            "aac", "mp3", "opus", "vorbis", "flac", "ac3", "eac3", "dts", "mp2",
        ] {
            assert!(
                container_supports_audio_codec(
                    ContainerFormat::Nut,
                    &CodecName::from_static(codec)
                ),
                "nut/{codec}"
            );
        }
    }
}
