//! Post-processing configuration for the rdlp pipeline.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio_format::AudioFormat;
use crate::container::ContainerFormat;
use crate::fixup_policy::FixupPolicy;
use crate::recode_audio_mode::RecodeAudioMode;

/// Post-processing configuration controlling FFmpeg transforms and file handling.
///
/// This struct is the canonical post-processing config type. It is placed in
/// `rdlp-types` so all crates can reference it without depending on `rdlp-core`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostProcess {
    /// Extract audio only (discard video stream).
    pub extract_audio: bool,
    /// Target audio format for extraction.
    pub audio_format: Option<AudioFormat>,
    /// Audio quality hint (e.g. "0"–"9" for VBR or a bitrate like "192k").
    pub audio_quality: Option<String>,
    /// Re-encode video to this container format.
    pub recode_video: Option<ContainerFormat>,
    /// Remux (container-only copy) to this format.
    pub remux_container: Option<ContainerFormat>,
    /// Preferred output container when merging separate streams.
    pub merge_output_format: Option<ContainerFormat>,
    /// Embed thumbnail into the output file (default: true).
    pub embed_thumbnail: bool,
    /// Write thumbnail to a separate file alongside the output.
    pub write_thumbnail: bool,
    /// Embed metadata (title, uploader, etc.) into the output file.
    pub embed_metadata: bool,
    /// Embed subtitle tracks into the output file.
    pub embed_subtitles: bool,
    /// Write subtitles to separate `.srt`/`.vtt` files.
    pub write_subtitles: bool,
    /// Keep the original video file after audio extraction.
    pub keep_video: bool,
    /// Path to the FFmpeg binary or directory.
    pub ffmpeg_location: Option<PathBuf>,
    /// Extra FFmpeg arguments passed verbatim to the FFmpeg invocation.
    pub ffmpeg_args: Vec<String>,
    /// Normalise audio (peak mode) after download.
    pub normalize_audio: bool,
    /// Apply EBU R128 loudness normalisation (loudnorm filter).
    pub loudnorm: bool,
    /// Peak level target in dBFS (e.g. `-1.0`).
    pub audio_gain_target: Option<f64>,
    /// Named loudnorm preset (e.g. `"streaming"`, `"broadcast"`).
    pub loudnorm_preset: Option<String>,
    /// Loudnorm integrated loudness target (LUFS, e.g. `-14.0`).
    pub loudnorm_target_i: Option<f64>,
    /// Loudnorm true peak target (dBTP, e.g. `-1.0`).
    pub loudnorm_target_tp: Option<f64>,
    /// Loudnorm loudness range target (LU, e.g. `7.0`).
    pub loudnorm_target_lra: Option<f64>,
    /// Use dynamic (two-pass) loudnorm mode.
    pub loudnorm_dynamic: bool,
    /// Apply a pre-compression pass before loudnorm.
    pub loudnorm_precompress: bool,
    /// Apply a limiter-boost normalisation fallback.
    pub normalize_boost: bool,
    /// Gain applied by the limiter-boost stage (dB).
    pub normalize_boost_db: Option<f64>,
    /// FFmpeg encoder name for the video stream (e.g. `"libx265"`).
    pub video_encoder: Option<String>,
    /// How to handle audio during video recode.
    pub recode_audio: RecodeAudioMode,
    /// Override the output container for recode (independent of `recode_video`).
    pub recode_container: Option<ContainerFormat>,
    /// Policy for automatic fixup of downloaded files.
    pub fixup: FixupPolicy,
}

impl Default for PostProcess {
    fn default() -> Self {
        Self {
            extract_audio: false,
            audio_format: None,
            audio_quality: None,
            recode_video: None,
            remux_container: None,
            merge_output_format: None,
            embed_thumbnail: true,
            write_thumbnail: false,
            embed_metadata: false,
            embed_subtitles: false,
            write_subtitles: false,
            keep_video: false,
            ffmpeg_location: None,
            ffmpeg_args: Vec::new(),
            normalize_audio: false,
            loudnorm: false,
            audio_gain_target: None,
            loudnorm_preset: None,
            loudnorm_target_i: None,
            loudnorm_target_tp: None,
            loudnorm_target_lra: None,
            loudnorm_dynamic: false,
            loudnorm_precompress: false,
            normalize_boost: false,
            normalize_boost_db: None,
            video_encoder: None,
            recode_audio: RecodeAudioMode::default(),
            recode_container: None,
            fixup: FixupPolicy::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_embed_thumbnail_is_true() {
        assert!(PostProcess::default().embed_thumbnail);
    }

    #[test]
    fn default_fixup_is_detect_or_warn() {
        assert_eq!(PostProcess::default().fixup, FixupPolicy::DetectOrWarn);
    }

    #[test]
    fn serde_roundtrip() {
        let mut pp = PostProcess::default();
        pp.extract_audio = true;
        pp.audio_format = Some(AudioFormat::Mp3);
        pp.recode_video = Some(ContainerFormat::Mkv);
        pp.fixup = FixupPolicy::Never;

        let json = serde_json::to_string(&pp).unwrap();
        let parsed: PostProcess = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.extract_audio, pp.extract_audio);
        assert_eq!(parsed.audio_format, pp.audio_format);
        assert_eq!(parsed.recode_video, pp.recode_video);
        assert_eq!(parsed.fixup, pp.fixup);
        assert!(parsed.embed_thumbnail);
    }

    #[test]
    fn serde_missing_fields_use_defaults() {
        let parsed: PostProcess = serde_json::from_str("{}").unwrap();
        assert!(parsed.embed_thumbnail);
        assert_eq!(parsed.fixup, FixupPolicy::DetectOrWarn);
    }
}
