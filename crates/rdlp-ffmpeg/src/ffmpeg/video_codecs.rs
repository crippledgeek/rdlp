//! Video codec configuration registry.
//!
//! Provides a static `CODEC_PREFERENCES` table mapping codec names to ordered
//! encoder preference lists. Runtime detection via `ffmpeg_the_third::codec::encoder::find_by_name()`
//! determines which encoders are actually available in the linked `FFmpeg` build.
//!
//! Use [`preferred_video_encoder()`] to resolve the best available encoder for a
//! given codec name. Use [`list_available_codecs()`] to enumerate all codecs with
//! at least one available encoder (for UI population).

use rdlp_types::media_name::{CodecName, VideoEncoder, VideoEncoderName};
use serde::{Deserialize, Serialize};

use crate::ffmpeg::codec_registry::{self, CodecRow};

/// Information about a specific video encoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoEncoderInfo {
    /// `FFmpeg` encoder name (e.g., "libx264", "libsvtav1")
    pub encoder_name: String,
    /// Human-readable display name (e.g., "x264 (H.264)")
    pub display_name: String,
    /// Speed-control knobs this encoder exposes (preset / deadline+cpu-used /
    /// `speed_level`). Empty when the encoder has no panel-controllable knob.
    pub speed_controls: Vec<SpeedKnob>,
}

/// Information about a video codec and its available encoders.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCodecInfo {
    /// Canonical codec name (e.g., "h264", "av1")
    pub codec: String,
    /// Human-readable display name (e.g., "H.264 / AVC")
    pub display_name: String,
    /// Encoders available in the current `FFmpeg` build, in preference order
    pub encoders: Vec<VideoEncoderInfo>,
}

/// Which typed recode wire field a knob feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KnobField {
    /// The encoder's `-preset` option (named string).
    Preset,
    /// The libvpx `-deadline` option (best / good / realtime).
    Deadline,
    /// The libvpx `-cpu-used` option (integer quality/speed trade-off).
    CpuUsed,
    /// The xavs2 `-speed_level` option (integer 0–9).
    SpeedLevel,
}

/// Static-table form of a knob (lives next to `CODEC_PREFERENCES`).
#[derive(Debug, Clone, Copy)]
pub(crate) enum KnobKindDef {
    Choice {
        choices: &'static [&'static str],
        default: &'static str,
    },
    Int {
        min: i32,
        max: i32,
        default: i32,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SpeedKnobDef {
    pub field: KnobField,
    pub label: &'static str,
    pub kind: KnobKindDef,
}

/// Serde/IPC form of a knob (materialized into [`VideoEncoderInfo`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SpeedKnob {
    /// Named-choice knob (e.g. preset, deadline).
    Choice {
        /// Which wire field this knob maps to.
        field: KnobField,
        /// Human-readable label for the UI.
        label: String,
        /// Ordered list of valid choice strings.
        choices: Vec<String>,
        /// Default choice string.
        default: String,
    },
    /// Integer-range knob (e.g. cpu-used, speed-level, SVT-AV1 preset).
    Int {
        /// Which wire field this knob maps to.
        field: KnobField,
        /// Human-readable label for the UI.
        label: String,
        /// Minimum valid value (inclusive).
        min: i32,
        /// Maximum valid value (inclusive).
        max: i32,
        /// Default value.
        default: i32,
    },
}

impl SpeedKnob {
    pub(crate) fn from_def(d: &SpeedKnobDef) -> Self {
        match d.kind {
            KnobKindDef::Choice { choices, default } => Self::Choice {
                field: d.field,
                label: d.label.to_string(),
                choices: choices.iter().map(|s| (*s).to_string()).collect(),
                default: default.to_string(),
            },
            KnobKindDef::Int { min, max, default } => Self::Int {
                field: d.field,
                label: d.label.to_string(),
                min,
                max,
                default,
            },
        }
    }
}

// --- Per-encoder knob tables (verified against FFmpeg 8.0.1-mediaforge, 2026-06-09) ---
const PRESETS_X26X: &[&str] = &[
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
    "placebo",
];
const PRESETS_VVENC: &[&str] = &["faster", "fast", "medium", "slow", "slower"];
const PRESETS_XEVE: &[&str] = &["default", "fast", "medium", "slow", "placebo"];
const DEADLINE_VALUES: &[&str] = &["best", "good", "realtime"];

const KNOBS_X264: &[SpeedKnobDef] = &[SpeedKnobDef {
    field: KnobField::Preset,
    label: "Preset",
    kind: KnobKindDef::Choice {
        choices: PRESETS_X26X,
        default: "medium",
    },
}];
const KNOBS_X265: &[SpeedKnobDef] = KNOBS_X264; // identical preset vocabulary
const KNOBS_VVENC: &[SpeedKnobDef] = &[SpeedKnobDef {
    field: KnobField::Preset,
    label: "Preset",
    kind: KnobKindDef::Choice {
        choices: PRESETS_VVENC,
        default: "medium",
    },
}];
const KNOBS_XEVE: &[SpeedKnobDef] = &[SpeedKnobDef {
    field: KnobField::Preset,
    label: "Preset",
    kind: KnobKindDef::Choice {
        choices: PRESETS_XEVE,
        default: "medium",
    },
}];
const KNOBS_SVTAV1: &[SpeedKnobDef] = &[SpeedKnobDef {
    field: KnobField::Preset,
    label: "Preset",
    kind: KnobKindDef::Int {
        min: -2,
        max: 13,
        default: -2,
    }, // -2 = SVT-AV1 "auto" sentinel (encoder picks preset)
}];
const KNOBS_VP9: &[SpeedKnobDef] = &[
    SpeedKnobDef {
        field: KnobField::Deadline,
        label: "Deadline",
        kind: KnobKindDef::Choice {
            choices: DEADLINE_VALUES,
            default: "good",
        },
    },
    SpeedKnobDef {
        field: KnobField::CpuUsed,
        label: "CPU used",
        kind: KnobKindDef::Int {
            min: -8,
            max: 8,
            default: 1,
        },
    },
];
const KNOBS_VP8: &[SpeedKnobDef] = &[
    SpeedKnobDef {
        field: KnobField::Deadline,
        label: "Deadline",
        kind: KnobKindDef::Choice {
            choices: DEADLINE_VALUES,
            default: "good",
        },
    },
    SpeedKnobDef {
        field: KnobField::CpuUsed,
        label: "CPU used",
        kind: KnobKindDef::Int {
            min: -16,
            max: 16,
            default: 1,
        },
    },
];
const KNOBS_XAVS2: &[SpeedKnobDef] = &[SpeedKnobDef {
    field: KnobField::SpeedLevel,
    label: "Speed level",
    kind: KnobKindDef::Int {
        min: 0,
        max: 9,
        default: 0,
    },
}];

/// Speed-control descriptor for an encoder. Empty slice = encoder exposes no
/// panel-controllable speed knob (or is not modeled yet, e.g. unlinked encoders).
///
/// Takes a [`VideoEncoderName`] (not a bare `&str`): this table is keyed to
/// the video-encoder vocabulary, and typing the parameter is what stops an
/// audio encoder name from silently matching nothing and returning `&[]`
/// (#642's A1).
#[must_use]
pub(crate) fn speed_controls_def(encoder: &VideoEncoderName) -> &'static [SpeedKnobDef] {
    match encoder.as_str() {
        "libx264" => KNOBS_X264,
        "libx265" => KNOBS_X265,
        "libvvenc" => KNOBS_VVENC,
        "libxeve" => KNOBS_XEVE,
        "libsvtav1" => KNOBS_SVTAV1,
        "libvpx-vp9" => KNOBS_VP9,
        "libvpx" => KNOBS_VP8,
        "libxavs2" => KNOBS_XAVS2,
        _ => &[],
    }
}

/// Entry in the codec preferences table.
struct CodecEntry {
    /// Canonical codec name
    codec: CodecName,
    /// Human-readable display name for the codec
    display_name: &'static str,
    /// Ordered encoder preference list: (`encoder_name`, `display_name`)
    encoders: &'static [(VideoEncoderName, &'static str)],
    /// `FFmpeg` spelling variants that identify the *same* codec (`"avc"` for
    /// `"h264"`, `"hevc"` for `"h265"`, `"mpeg1video"` for `"mpeg1"`,
    /// `"mpeg2video"` for `"mpeg2"`). Declared next to the row it describes —
    /// mirrors the audio registry's `pcm`/`pcm_s16le` alias — so
    /// [`canonical_codec_key`] can answer "do these two spellings name the
    /// same codec" without a second, hand-maintained alias table living in
    /// `recode.rs`'s remux-compatibility rules (#576).
    aliases: &'static [CodecName],
}

impl codec_registry::CodecRow for CodecEntry {
    type Encoder = VideoEncoder;
    fn codec(&self) -> &CodecName {
        &self.codec
    }
    fn encoders(&self) -> &'static [(VideoEncoderName, &'static str)] {
        self.encoders
    }
    fn aliases(&self) -> &'static [CodecName] {
        self.aliases
    }
}

/// Static table mapping codec names to ordered encoder preference lists.
///
/// Codecs are listed in preference order — the first available encoder
/// in each list is selected at runtime. Aliases (e.g., "hevc" for "h265")
/// are separate entries that share the same encoder list.
static CODEC_PREFERENCES: &[CodecEntry] = &[
    CodecEntry {
        codec: CodecName::H264,
        display_name: "H.264 / AVC",
        encoders: &[
            (VideoEncoderName::from_static("libx264"), "x264 (H.264)"),
            (
                VideoEncoderName::from_static("libopenh264"),
                "OpenH264 (H.264)",
            ),
        ],
        aliases: &[CodecName::from_static("avc")],
    },
    CodecEntry {
        codec: CodecName::from_static("h265"),
        display_name: "H.265 / HEVC",
        encoders: &[
            (
                VideoEncoderName::from_static("libx265"),
                "x265 (H.265/HEVC)",
            ),
            (
                VideoEncoderName::from_static("libkvazaar"),
                "Kvazaar (H.265/HEVC)",
            ),
        ],
        aliases: &[CodecName::HEVC],
    },
    CodecEntry {
        codec: CodecName::HEVC,
        display_name: "H.265 / HEVC",
        encoders: &[
            (
                VideoEncoderName::from_static("libx265"),
                "x265 (H.265/HEVC)",
            ),
            (
                VideoEncoderName::from_static("libkvazaar"),
                "Kvazaar (H.265/HEVC)",
            ),
        ],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::AV1,
        display_name: "AV1",
        encoders: &[
            (VideoEncoderName::from_static("libsvtav1"), "SVT-AV1"),
            (VideoEncoderName::from_static("libaom-av1"), "libaom AV1"),
            (VideoEncoderName::from_static("librav1e"), "rav1e AV1"),
        ],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::VP9,
        display_name: "VP9",
        encoders: &[(VideoEncoderName::from_static("libvpx-vp9"), "libvpx VP9")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::VP8,
        display_name: "VP8",
        encoders: &[(VideoEncoderName::from_static("libvpx"), "libvpx VP8")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("vvc"),
        display_name: "VVC / H.266",
        encoders: &[(
            VideoEncoderName::from_static("libvvenc"),
            "VVenC (VVC/H.266)",
        )],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("h266"),
        display_name: "VVC / H.266",
        encoders: &[(
            VideoEncoderName::from_static("libvvenc"),
            "VVenC (VVC/H.266)",
        )],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("evc"),
        display_name: "EVC / MPEG-5",
        encoders: &[(
            VideoEncoderName::from_static("libxeve"),
            "xeve (MPEG-5 EVC)",
        )],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("avs2"),
        display_name: "AVS2 / IEEE 1857.4",
        encoders: &[(VideoEncoderName::from_static("libxavs2"), "xavs2 (AVS2)")],
        aliases: &[],
    },
    // NOTE: APV (liboapv) intentionally NOT registered — it rejects standard
    // 8-bit yuv420p web video (pro 4:2:2/10-bit intra codec), so it is not a
    // viable recode target for downloaded content.
    CodecEntry {
        codec: CodecName::from_static("theora"),
        display_name: "Theora",
        encoders: &[(VideoEncoderName::from_static("libtheora"), "libtheora")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("mpeg4"),
        display_name: "MPEG-4 Part 2",
        encoders: &[(VideoEncoderName::from_static("mpeg4"), "MPEG-4 (built-in)")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("mpeg2"),
        display_name: "MPEG-2 Video",
        encoders: &[(
            VideoEncoderName::from_static("mpeg2video"),
            "MPEG-2 Video (built-in)",
        )],
        aliases: &[CodecName::from_static("mpeg2video")],
    },
    CodecEntry {
        codec: CodecName::from_static("mpeg1"),
        display_name: "MPEG-1 Video",
        encoders: &[(
            VideoEncoderName::from_static("mpeg1video"),
            "MPEG-1 Video (built-in)",
        )],
        aliases: &[CodecName::from_static("mpeg1video")],
    },
    CodecEntry {
        codec: CodecName::from_static("xvid"),
        display_name: "XviD (MPEG-4)",
        encoders: &[(VideoEncoderName::from_static("libxvid"), "libxvid (XviD)")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("prores"),
        display_name: "Apple ProRes",
        encoders: &[(VideoEncoderName::from_static("prores_ks"), "ProRes KS")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("dnxhd"),
        display_name: "DNxHD / DNxHR",
        encoders: &[(VideoEncoderName::from_static("dnxhd"), "DNxHD (built-in)")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("wmv2"),
        display_name: "WMV2",
        encoders: &[(VideoEncoderName::from_static("wmv2"), "WMV2 (built-in)")],
        aliases: &[],
    },
    CodecEntry {
        codec: CodecName::from_static("ffv1"),
        display_name: "FFV1 (Lossless)",
        encoders: &[(VideoEncoderName::from_static("ffv1"), "FFV1 (built-in)")],
        aliases: &[],
    },
    // Keyed to the exact FFmpeg codec-ID name (matching
    // `muxer_defaults::declared_codec`'s output, same discipline as the
    // audio registry's `pcm_s16le`/`wmav2` rows) rather than a friendlier
    // alias, so `resolve_encoder(default_codec_for_container(Dv))` actually
    // finds this row. Added alongside Task 13 (#618): before this row
    // existed, `default_codec_for_container(Dv)` correctly returned
    // "dvvideo" but `resolve_encoder("dvvideo")` returned `None` (no row
    // matched by either codec-key or literal-encoder-name), so
    // `--recode-video=dv` failed with "pipeline terminated with no output
    // and no error" — a different failure than the original #618 bug
    // (`h264` rejected by the dv muxer), but still failing end-to-end.
    CodecEntry {
        codec: CodecName::from_static("dvvideo"),
        display_name: "DV (Digital Video)",
        encoders: &[(
            VideoEncoderName::from_static("dvvideo"),
            "DV Video (built-in)",
        )],
        aliases: &[],
    },
    // Same discipline and same #618 discovery as `dvvideo` above: the
    // Wmv/Asf muxers declare codec-ID `msmpeg4v3`, but FFmpeg's *encoder*
    // for that codec ID is named `msmpeg4` (see `ffmpeg -encoders`), not
    // `msmpeg4v3` — so this row must be keyed to the codec-ID name with the
    // differently-named encoder, the same shape as the `mpeg2`/`mpeg2video`
    // and `mpeg1`/`mpeg1video` rows above.
    CodecEntry {
        codec: CodecName::from_static("msmpeg4v3"),
        display_name: "MS MPEG-4 v3 (WMV1-era)",
        encoders: &[(
            VideoEncoderName::from_static("msmpeg4"),
            "MS MPEG-4 v3 (built-in)",
        )],
        aliases: &[],
    },
];

/// The video codec registry, owning its own cache over [`CODEC_PREFERENCES`].
static VIDEO_REGISTRY: codec_registry::Registry<CodecEntry> =
    codec_registry::Registry::new(CODEC_PREFERENCES, codec_registry::MediaKind::Video);

/// Returns `true` if `encoder` is available in the current `FFmpeg` build.
///
/// Uses `ffmpeg_the_third::codec::encoder::find_by_name()` for detection.
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn is_encoder_available(encoder: &str) -> bool {
    codec_registry::is_encoder_available(encoder)
}

/// Returns the best available video encoder for a given codec name.
///
/// Results are cached per codec name using a `OnceLock<HashMap>` so detection
/// only runs once per codec. Logs which encoder was selected on first resolution.
///
/// Returns `None` if no encoder is available for the requested codec, or if the
/// codec name is unknown.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn preferred_video_encoder(codec: &str) -> Option<VideoEncoderName> {
    VIDEO_REGISTRY.preferred_encoder(codec)
}

/// Resolves an encoder name.
///
/// Accepts either a codec name (e.g., "h264") or a direct encoder name
/// (e.g., "libx264"). For codec names, returns the best available encoder
/// via [`preferred_video_encoder()`]. For encoder names, checks availability
/// directly and returns the name if available.
///
/// Returns `None` if no encoder can be resolved or is available.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn resolve_encoder(input: &str) -> Option<VideoEncoderName> {
    VIDEO_REGISTRY.resolve(input)
}

/// Resolves `name` to its codec group's canonical (primary table) key,
/// case-insensitively, via `codec_registry::CodecRow::aliases`.
///
/// `"avc"` and `"h264"` both resolve to `"h264"`; `"hevc"` and `"h265"` both
/// resolve to `"h265"`. A name with no table row or alias (an unmodeled or
/// unknown codec) resolves to `None`, so it compares unequal to every real
/// codec group rather than aliasing itself to something arbitrary.
///
/// This is the single source of alias knowledge for
/// [`rdlp_postprocess`](https://docs.rs/rdlp-postprocess)'s remux-compatibility
/// rules (#576): a caller comparing two codec spellings for "same codec"
/// resolves both through this function instead of hand-listing every known
/// spelling pair.
#[must_use]
pub fn canonical_codec_key(name: &str) -> Option<&'static str> {
    VIDEO_REGISTRY
        .find_row(name)
        .map(|row| row.codec().as_str())
}

/// Returns all available encoders for a given codec name, in preference order.
///
/// Returns an empty `Vec` if the codec is unknown or has no available encoders.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn available_encoders_for_codec(codec: &str) -> Vec<VideoEncoderInfo> {
    VIDEO_REGISTRY
        .find_row(codec)
        .map(|row| {
            codec_registry::available_encoders(row)
                .map(|(enc, display)| VideoEncoderInfo {
                    encoder_name: enc.as_str().to_string(),
                    display_name: display.to_string(),
                    speed_controls: speed_controls_def(&enc)
                        .iter()
                        .map(SpeedKnob::from_def)
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lists all codecs that have at least one available encoder in the current
/// `FFmpeg` build.
///
/// Alias entries (e.g., "hevc" / "h265") that share the same encoder list are
/// both included if their encoders are available. Callers that want a deduplicated
/// list may filter by `codec` name.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn list_available_codecs() -> Vec<VideoCodecInfo> {
    CODEC_PREFERENCES
        .iter()
        .filter_map(|entry| {
            let encoders: Vec<VideoEncoderInfo> = codec_registry::available_encoders(entry)
                .map(|(enc, display)| VideoEncoderInfo {
                    encoder_name: enc.as_str().to_string(),
                    display_name: display.to_string(),
                    speed_controls: speed_controls_def(&enc)
                        .iter()
                        .map(SpeedKnob::from_def)
                        .collect(),
                })
                .collect();

            if encoders.is_empty() {
                None
            } else {
                Some(VideoCodecInfo {
                    codec: entry.codec.as_str().to_string(),
                    display_name: entry.display_name.to_string(),
                    encoders,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preferred_video_encoder_h264() {
        let enc = preferred_video_encoder("h264");
        assert!(enc.is_some());
        let name = enc.unwrap();
        assert!(name == "libx264" || name == "libopenh264");
    }

    #[test]
    fn test_preferred_video_encoder_unknown() {
        assert!(preferred_video_encoder("nonexistent").is_none());
    }

    #[test]
    fn test_resolve_encoder_codec_name() {
        let enc = resolve_encoder("h264");
        assert!(enc.is_some());
    }

    #[test]
    fn test_resolve_encoder_encoder_name() {
        // Built-in encoders are always available
        let enc = resolve_encoder("mpeg4");
        assert_eq!(
            enc.as_ref().map(rdlp_types::media_name::MediaName::as_str),
            Some("mpeg4")
        );
    }

    #[test]
    fn test_evc_avs2_resolve_when_available() {
        // EVC + AVS2 were registered in CODEC_PREFERENCES (Task 2). They resolve
        // only when their encoders are built into this ffmpeg; gate on
        // availability so a build without the codecs still passes.
        if is_encoder_available("libxeve") {
            assert_eq!(
                resolve_encoder("evc")
                    .as_ref()
                    .map(rdlp_types::media_name::MediaName::as_str),
                Some("libxeve")
            );
            assert_eq!(
                resolve_encoder("libxeve")
                    .as_ref()
                    .map(rdlp_types::media_name::MediaName::as_str),
                Some("libxeve")
            );
        } else {
            // Registered in the table but not built into this ffmpeg: the
            // name matches yet the availability gate rejects it → None. Pins
            // the `.and_then(is_available.then_some(..))` unavailable branch.
            assert_eq!(resolve_encoder("libxeve"), None);
        }
        if is_encoder_available("libxavs2") {
            assert_eq!(
                resolve_encoder("avs2")
                    .as_ref()
                    .map(rdlp_types::media_name::MediaName::as_str),
                Some("libxavs2")
            );
        } else {
            assert_eq!(resolve_encoder("libxavs2"), None);
        }
        // APV must NOT be registered (intentionally excluded).
        assert_eq!(resolve_encoder("apv"), None);
    }

    #[test]
    fn test_resolve_encoder_unknown() {
        assert!(resolve_encoder("nonexistent").is_none());
    }

    #[test]
    fn test_is_encoder_available_builtin() {
        assert!(is_encoder_available("mpeg4"));
    }

    #[test]
    fn test_list_available_codecs_nonempty() {
        let codecs = list_available_codecs();
        assert!(
            !codecs.is_empty(),
            "at least built-in codecs should be available"
        );
    }

    #[test]
    fn test_available_encoders_for_known_codec() {
        let encoders = available_encoders_for_codec("h264");
        assert!(!encoders.is_empty());
    }

    #[test]
    fn test_available_encoders_for_unknown_codec() {
        let encoders = available_encoders_for_codec("nonexistent");
        assert!(encoders.is_empty());
    }

    #[test]
    fn vp8_and_vp9_cpu_used_ranges_differ() {
        let cpu = |k: &[SpeedKnobDef]| {
            k.iter()
                .find_map(|d| match (d.field, d.kind) {
                    (KnobField::CpuUsed, KnobKindDef::Int { min, max, .. }) => Some((min, max)),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            cpu(speed_controls_def(&VideoEncoderName::from_static(
                "libvpx-vp9"
            ))),
            (-8, 8)
        );
        assert_eq!(
            cpu(speed_controls_def(&VideoEncoderName::from_static("libvpx"))),
            (-16, 16)
        );
    }

    #[test]
    fn vvenc_presets_are_the_five_names() {
        let knobs = speed_controls_def(&VideoEncoderName::from_static("libvvenc"));
        let first = knobs.first().expect("vvenc must have at least one knob");
        match first.kind {
            KnobKindDef::Choice { choices, default } => {
                assert_eq!(choices, &["faster", "fast", "medium", "slow", "slower"]);
                assert_eq!(default, "medium");
            }
            KnobKindDef::Int { .. } => panic!("vvenc preset should be a Choice"),
        }
    }

    #[test]
    fn svtav1_preset_is_numeric_range() {
        let knobs = speed_controls_def(&VideoEncoderName::from_static("libsvtav1"));
        let first = knobs.first().expect("svtav1 must have at least one knob");
        match first.kind {
            KnobKindDef::Int { min, max, default } => {
                assert_eq!((min, max, default), (-2, 13, -2));
            }
            KnobKindDef::Choice { .. } => panic!("svtav1 preset should be Int"),
        }
    }

    #[test]
    fn unlinked_or_unknown_encoder_has_no_knobs() {
        assert!(speed_controls_def(&VideoEncoderName::from_static("libkvazaar")).is_empty());
        assert!(speed_controls_def(&VideoEncoderName::from_static("nonsense")).is_empty());
    }

    #[test]
    fn x264_and_x265_share_preset_vocabulary() {
        let names = |k: &[SpeedKnobDef]| {
            k.iter()
                .find_map(|d| match d.kind {
                    KnobKindDef::Choice { choices, .. } => Some(choices),
                    KnobKindDef::Int { .. } => None,
                })
                .unwrap()
        };
        assert_eq!(
            names(speed_controls_def(&VideoEncoderName::from_static(
                "libx264"
            ))),
            names(speed_controls_def(&VideoEncoderName::from_static(
                "libx265"
            )))
        );
    }

    #[test]
    fn list_available_codecs_carries_speed_controls() {
        super::super::ensure_init().ok();
        for c in list_available_codecs() {
            for e in &c.encoders {
                if e.encoder_name == "libvvenc" {
                    assert!(
                        !e.speed_controls.is_empty(),
                        "libvvenc must expose a preset knob"
                    );
                }
            }
        }
    }

    /// REGRESSION GUARD: the audio and video tables are disjoint.
    ///
    /// `Registry` now owns its cache, so cross-registry cache leakage is not
    /// representable — this test instead pins the real invariant that
    /// remains: `CODEC_PREFERENCES` and `AUDIO_CODEC_PREFERENCES` share no
    /// codec name, so a codec that belongs to one registry must resolve to
    /// `None` through the other. Resolve through each registry FIRST so both
    /// caches are populated before the cross checks.
    #[test]
    fn audio_and_video_registries_have_independent_caches() {
        use crate::ffmpeg::audio_encoder_registry::preferred_audio_encoder;

        crate::ffmpeg::ensure_init().expect("ffmpeg init");

        // Populate both caches.
        assert!(
            preferred_video_encoder("h264").is_some(),
            "h264 is a video codec"
        );
        assert!(
            preferred_audio_encoder("aac").is_some(),
            "aac is an audio codec"
        );

        // Cross checks — these are what a shared cache would break.
        assert_eq!(
            preferred_video_encoder("aac"),
            None,
            "video registry returned an answer for an audio-only codec — caches are shared"
        );
        assert_eq!(
            preferred_audio_encoder("h264"),
            None,
            "audio registry returned an answer for a video-only codec — caches are shared"
        );
    }
}
