//! Audio codec configuration registry.
//!
//! Provides `AudioCodecConfig` and `AUDIO_CODECS` for mapping codec names
//! to encoder names, file extensions, quality scale ranges, and bitrate ranges.
//!
//! When `libfdk_aac` is available (custom `FFmpeg` build with `--enable-nonfree`),
//! it is automatically preferred over the built-in `aac` encoder for better
//! quality at equivalent bitrates — resolved via
//! [`super::audio_encoder_registry::preferred_audio_encoder`], the single
//! source of truth for encoder preference.

/// Audio codec configuration for extraction/conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioCodecConfig {
    /// `FFmpeg` encoder name (e.g., "libmp3lame", "aac")
    pub encoder: Option<&'static str>,
    /// Output file extension
    pub extension: &'static str,
    /// Quality scale range (worst, best) for -q:a
    pub quality_scale: Option<(u8, u8)>,
    /// Bitrate range in kbps (min, max) for -b:a
    pub bitrate_range: Option<(u32, u32)>,
}

/// Supported audio codecs and their configurations.
pub static AUDIO_CODECS: &[(&str, AudioCodecConfig)] = &[
    (
        "mp3",
        AudioCodecConfig {
            encoder: Some("libmp3lame"),
            extension: "mp3",
            quality_scale: Some((9, 0)), // VBR quality (0=best, 9=worst)
            bitrate_range: Some((32, 320)),
        },
    ),
    (
        "aac",
        AudioCodecConfig {
            encoder: Some("aac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: Some((32, 512)),
        },
    ),
    (
        "m4a",
        AudioCodecConfig {
            encoder: Some("aac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: Some((32, 512)),
        },
    ),
    (
        "opus",
        AudioCodecConfig {
            encoder: Some("libopus"),
            extension: "opus",
            quality_scale: None,
            bitrate_range: Some((6, 510)),
        },
    ),
    (
        "vorbis",
        AudioCodecConfig {
            encoder: Some("libvorbis"),
            extension: "ogg",
            quality_scale: Some((0, 10)), // Quality (0=worst, 10=best)
            bitrate_range: Some((32, 500)),
        },
    ),
    (
        "flac",
        AudioCodecConfig {
            encoder: Some("flac"), // Native FLAC encoder
            extension: "flac",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "alac",
        AudioCodecConfig {
            encoder: Some("alac"),
            extension: "m4a",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "wav",
        AudioCodecConfig {
            encoder: None, // PCM
            extension: "wav",
            quality_scale: None,
            bitrate_range: None,
        },
    ),
    (
        "ac3",
        AudioCodecConfig {
            encoder: Some("ac3"),
            extension: "ac3",
            quality_scale: None,
            bitrate_range: Some((64, 640)),
        },
    ),
    (
        "eac3",
        AudioCodecConfig {
            encoder: Some("eac3"),
            extension: "eac3",
            quality_scale: None,
            bitrate_range: Some((32, 6144)),
        },
    ),
    (
        "dts",
        AudioCodecConfig {
            encoder: Some("dca"),
            extension: "dts",
            quality_scale: None,
            bitrate_range: Some((32, 3840)),
        },
    ),
    (
        "mp2",
        AudioCodecConfig {
            encoder: Some("mp2"),
            extension: "mp2",
            quality_scale: None,
            bitrate_range: Some((32, 384)),
        },
    ),
    (
        "wavpack",
        AudioCodecConfig {
            encoder: Some("wavpack"),
            extension: "wv",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
    (
        "tta",
        AudioCodecConfig {
            encoder: Some("tta"),
            extension: "tta",
            quality_scale: None,
            bitrate_range: None, // Lossless
        },
    ),
];

/// Get audio codec configuration by name.
#[must_use]
pub fn get_audio_codec(name: &str) -> Option<&'static AudioCodecConfig> {
    AUDIO_CODECS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, config)| config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_audio_codec_aac() {
        let config = get_audio_codec("aac").unwrap();
        assert_eq!(config.extension, "m4a");
        assert!(config.encoder.is_some());
    }

    #[test]
    fn test_get_audio_codec_case_insensitive() {
        assert!(get_audio_codec("AAC").is_some());
        assert!(get_audio_codec("Mp3").is_some());
        assert!(get_audio_codec("FLAC").is_some());
    }

    #[test]
    fn test_get_audio_codec_unknown() {
        assert!(get_audio_codec("nonexistent").is_none());
    }
}
