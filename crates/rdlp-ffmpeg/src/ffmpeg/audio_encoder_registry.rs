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

use crate::ffmpeg::codec_registry;

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
}

impl codec_registry::CodecRow for AudioCodecEntry {
    fn codec(&self) -> &'static str {
        self.codec
    }
    fn encoders(&self) -> &'static [(&'static str, &'static str)] {
        self.encoders
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
        ],
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
        ],
    },
    AudioCodecEntry {
        codec: "opus",
        display_name: "Opus",
        encoders: &[("libopus", "Opus (libopus)")],
        supported_containers: &[
            ContainerFormat::Mp4,
            ContainerFormat::Mkv,
            ContainerFormat::WebM,
            ContainerFormat::Ogg,
        ],
    },
    AudioCodecEntry {
        codec: "vorbis",
        display_name: "Vorbis",
        encoders: &[("libvorbis", "Vorbis (libvorbis)")],
        supported_containers: &[
            ContainerFormat::Mkv,
            ContainerFormat::WebM,
            ContainerFormat::Ogg,
        ],
    },
    AudioCodecEntry {
        codec: "flac",
        display_name: "FLAC",
        encoders: &[("flac", "FLAC (built-in)")],
        supported_containers: &[ContainerFormat::Mkv, ContainerFormat::Ogg],
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
        ],
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
    },
    AudioCodecEntry {
        codec: "dts",
        display_name: "DTS",
        encoders: &[("dca", "DTS (built-in)")],
        supported_containers: &[ContainerFormat::Mkv],
    },
    AudioCodecEntry {
        codec: "mp2",
        display_name: "MP2",
        encoders: &[("mp2", "MP2 (built-in)")],
        supported_containers: &[ContainerFormat::Mkv, ContainerFormat::Ts],
    },
    AudioCodecEntry {
        codec: "pcm",
        display_name: "WAV/PCM",
        encoders: &[("pcm_s16le", "PCM 16-bit (built-in)")],
        supported_containers: &[ContainerFormat::Mkv, ContainerFormat::Avi],
    },
    AudioCodecEntry {
        codec: "wavpack",
        display_name: "WavPack",
        encoders: &[("wavpack", "WavPack (built-in)")],
        supported_containers: &[ContainerFormat::Mkv],
    },
    AudioCodecEntry {
        codec: "tta",
        display_name: "TTA",
        encoders: &[("tta", "TTA (built-in)")],
        supported_containers: &[ContainerFormat::Mkv],
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

/// Returns the best default audio encoder for the given container format.
///
/// Selection is based on what is most universally compatible:
/// - MP4, MOV, TS → AAC (best available)
/// - `WebM` → Opus
/// - OGG → Opus (if available) or Vorbis
/// - MKV → Opus (quality/size efficiency)
/// - AVI → MP3
/// - Default fallback → AAC
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn select_audio_encoder_for_container(container: ContainerFormat) -> &'static str {
    match container {
        ContainerFormat::WebM => preferred_audio_encoder("opus").unwrap_or("libopus"),
        ContainerFormat::Ogg => preferred_audio_encoder("opus")
            .or_else(|| preferred_audio_encoder("vorbis"))
            .unwrap_or("libopus"),
        ContainerFormat::Mkv => {
            // MKV supports everything; prefer Opus for quality/size efficiency
            preferred_audio_encoder("opus").unwrap_or("libopus")
        }
        ContainerFormat::Avi => preferred_audio_encoder("mp3").unwrap_or("libmp3lame"),
        _ => {
            // MP4, MOV, TS, and everything else → AAC
            preferred_audio_encoder("aac").unwrap_or("aac")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_aac_encoder_returns_valid() {
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
    fn container_does_not_support_mp4_vorbis() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            "vorbis"
        ));
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
        let enc = select_audio_encoder_for_container(ContainerFormat::Mp4);
        assert!(
            enc == "aac" || enc == "libfdk_aac",
            "expected aac encoder for mp4, got {enc}"
        );
    }

    #[test]
    fn preferred_aac_encoder_wrapper_consistency() {
        // The audio_codecs.rs preferred_aac_encoder() should match registry
        let registry_result = preferred_audio_encoder("aac").unwrap_or("aac");
        let legacy_result = crate::ffmpeg::audio_codecs::preferred_aac_encoder();
        assert_eq!(
            registry_result, legacy_result,
            "registry and legacy preferred_aac_encoder must agree"
        );
    }

    #[test]
    fn compatibility_rejects_vorbis_mp4() {
        assert!(!container_supports_audio_codec(
            ContainerFormat::Mp4,
            "vorbis"
        ));
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
}
