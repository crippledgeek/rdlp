//! Video codec configuration registry.
//!
//! Provides a static `CODEC_PREFERENCES` table mapping codec names to ordered
//! encoder preference lists. Runtime detection via `ffmpeg_the_third::codec::encoder::find_by_name()`
//! determines which encoders are actually available in the linked `FFmpeg` build.
//!
//! Use [`preferred_video_encoder()`] to resolve the best available encoder for a
//! given codec name. Use [`list_available_codecs()`] to enumerate all codecs with
//! at least one available encoder (for UI population).

use std::collections::HashMap;
use std::sync::OnceLock;

use log::info;
use serde::{Deserialize, Serialize};

/// Information about a specific video encoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoEncoderInfo {
    /// `FFmpeg` encoder name (e.g., "libx264", "libsvtav1")
    pub encoder_name: String,
    /// Human-readable display name (e.g., "x264 (H.264)")
    pub display_name: String,
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

/// Entry in the codec preferences table.
struct CodecEntry {
    /// Canonical codec name
    codec: &'static str,
    /// Human-readable display name for the codec
    display_name: &'static str,
    /// Ordered encoder preference list: (`encoder_name`, `display_name`)
    encoders: &'static [(&'static str, &'static str)],
}

/// Static table mapping codec names to ordered encoder preference lists.
///
/// Codecs are listed in preference order — the first available encoder
/// in each list is selected at runtime. Aliases (e.g., "hevc" for "h265")
/// are separate entries that share the same encoder list.
static CODEC_PREFERENCES: &[CodecEntry] = &[
    CodecEntry {
        codec: "h264",
        display_name: "H.264 / AVC",
        encoders: &[
            ("libx264", "x264 (H.264)"),
            ("libopenh264", "OpenH264 (H.264)"),
        ],
    },
    CodecEntry {
        codec: "h265",
        display_name: "H.265 / HEVC",
        encoders: &[
            ("libx265", "x265 (H.265/HEVC)"),
            ("libkvazaar", "Kvazaar (H.265/HEVC)"),
        ],
    },
    CodecEntry {
        codec: "hevc",
        display_name: "H.265 / HEVC",
        encoders: &[
            ("libx265", "x265 (H.265/HEVC)"),
            ("libkvazaar", "Kvazaar (H.265/HEVC)"),
        ],
    },
    CodecEntry {
        codec: "av1",
        display_name: "AV1",
        encoders: &[
            ("libsvtav1", "SVT-AV1"),
            ("libaom-av1", "libaom AV1"),
            ("librav1e", "rav1e AV1"),
        ],
    },
    CodecEntry {
        codec: "vp9",
        display_name: "VP9",
        encoders: &[("libvpx-vp9", "libvpx VP9")],
    },
    CodecEntry {
        codec: "vp8",
        display_name: "VP8",
        encoders: &[("libvpx", "libvpx VP8")],
    },
    CodecEntry {
        codec: "vvc",
        display_name: "VVC / H.266",
        encoders: &[("libvvenc", "VVenC (VVC/H.266)")],
    },
    CodecEntry {
        codec: "h266",
        display_name: "VVC / H.266",
        encoders: &[("libvvenc", "VVenC (VVC/H.266)")],
    },
    CodecEntry {
        codec: "theora",
        display_name: "Theora",
        encoders: &[("libtheora", "libtheora")],
    },
    CodecEntry {
        codec: "mpeg4",
        display_name: "MPEG-4 Part 2",
        encoders: &[("mpeg4", "MPEG-4 (built-in)")],
    },
    CodecEntry {
        codec: "mpeg2",
        display_name: "MPEG-2 Video",
        encoders: &[("mpeg2video", "MPEG-2 Video (built-in)")],
    },
    CodecEntry {
        codec: "mpeg1",
        display_name: "MPEG-1 Video",
        encoders: &[("mpeg1video", "MPEG-1 Video (built-in)")],
    },
    CodecEntry {
        codec: "xvid",
        display_name: "XviD (MPEG-4)",
        encoders: &[("libxvid", "libxvid (XviD)")],
    },
    CodecEntry {
        codec: "prores",
        display_name: "Apple ProRes",
        encoders: &[("prores_ks", "ProRes KS")],
    },
    CodecEntry {
        codec: "dnxhd",
        display_name: "DNxHD / DNxHR",
        encoders: &[("dnxhd", "DNxHD (built-in)")],
    },
    CodecEntry {
        codec: "wmv2",
        display_name: "WMV2",
        encoders: &[("wmv2", "WMV2 (built-in)")],
    },
    CodecEntry {
        codec: "ffv1",
        display_name: "FFV1 (Lossless)",
        encoders: &[("ffv1", "FFV1 (built-in)")],
    },
];

/// Returns `true` if `encoder` is available in the current `FFmpeg` build.
///
/// Uses `ffmpeg_the_third::codec::encoder::find_by_name()` for detection.
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn is_encoder_available(encoder: &str) -> bool {
    ffmpeg_the_third::codec::encoder::find_by_name(encoder).is_some()
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
pub fn preferred_video_encoder(codec: &str) -> Option<&'static str> {
    static CACHE: OnceLock<HashMap<&'static str, Option<&'static str>>> = OnceLock::new();

    let cache = CACHE.get_or_init(|| {
        let mut map = HashMap::new();
        for entry in CODEC_PREFERENCES {
            let selected = entry
                .encoders
                .iter()
                .find(|(enc, _)| is_encoder_available(enc))
                .map(|(enc, _)| *enc);

            if let Some(enc) = selected {
                info!("Using {enc} as {codec} encoder", codec = entry.codec);
            }

            map.insert(entry.codec, selected);
        }
        map
    });

    // Only canonically-known codecs are in the cache; unknown codec names return None
    cache
        .get(codec.to_ascii_lowercase().as_str())
        .copied()
        .flatten()
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
pub fn resolve_encoder(input: &str) -> Option<&'static str> {
    let lower = input.to_ascii_lowercase();

    // Check if input is a known codec name first
    if let Some(enc) = preferred_video_encoder(&lower) {
        return Some(enc);
    }

    // Otherwise treat as a direct encoder name — check if it's available
    // and find the matching static str in our table
    for entry in CODEC_PREFERENCES {
        for (enc, _) in entry.encoders {
            if enc.eq_ignore_ascii_case(input) {
                if is_encoder_available(enc) {
                    return Some(enc);
                }
                return None;
            }
        }
    }

    None
}

/// Returns all available encoders for a given codec name, in preference order.
///
/// Returns an empty `Vec` if the codec is unknown or has no available encoders.
///
/// Requires [`super::ensure_init`] to have been called first.
#[must_use]
pub fn available_encoders_for_codec(codec: &str) -> Vec<VideoEncoderInfo> {
    let lower = codec.to_ascii_lowercase();
    CODEC_PREFERENCES
        .iter()
        .find(|e| e.codec == lower.as_str())
        .map(|entry| {
            entry
                .encoders
                .iter()
                .filter(|(enc, _)| is_encoder_available(enc))
                .map(|(enc, display)| VideoEncoderInfo {
                    encoder_name: (*enc).to_string(),
                    display_name: (*display).to_string(),
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
            let encoders: Vec<VideoEncoderInfo> = entry
                .encoders
                .iter()
                .filter(|(enc, _)| is_encoder_available(enc))
                .map(|(enc, display)| VideoEncoderInfo {
                    encoder_name: (*enc).to_string(),
                    display_name: (*display).to_string(),
                })
                .collect();

            if encoders.is_empty() {
                None
            } else {
                Some(VideoCodecInfo {
                    codec: entry.codec.to_string(),
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
        assert_eq!(enc, Some("mpeg4"));
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
}
