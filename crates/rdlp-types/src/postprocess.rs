//! Post-processing configuration for the rdlp pipeline.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio_format::AudioFormat;
use crate::container::ContainerFormat;
use crate::fixup_policy::FixupPolicy;
use crate::recode_audio_mode::RecodeAudioMode;
use crate::vpx_deadline::VpxDeadline;

/// A container the user explicitly asked for, paired with the CLI flag that
/// asked for it.
///
/// The flag travels with the format so operator-facing messages can name the
/// exact flag that won the precedence chain (`--recode-video=ts`), rather than
/// a bare container that leaves the user guessing which of their flags took
/// effect. Mirrors yt-dlp, whose equivalent conflict message names both sides
/// (`"--remux-video is ignored since --recode-video was given"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplicitContainer {
    /// The CLI flag that requested this container, e.g. `"--recode-video"`.
    pub flag: &'static str,
    /// The requested container format.
    pub format: ContainerFormat,
}

/// Post-processing configuration controlling `FFmpeg` transforms and file handling.
///
/// This struct is the canonical post-processing config type. It is placed in
/// `rdlp-types` so all crates can reference it without depending on `rdlp-core`.
#[allow(clippy::struct_excessive_bools)] // PostProcess carries many boolean pipeline flags
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
    /// Path to the `FFmpeg` binary or directory.
    pub ffmpeg_location: Option<PathBuf>,
    /// Extra `FFmpeg` arguments passed verbatim to the `FFmpeg` invocation.
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
    /// `FFmpeg` encoder name for the video stream (e.g. `"libx265"`).
    pub video_encoder: Option<String>,
    /// How to handle audio during video recode.
    pub recode_audio: RecodeAudioMode,
    /// Override the output container for recode (independent of `recode_video`).
    pub recode_container: Option<ContainerFormat>,
    /// Encoder thread count for video recode. `None` = auto-detect at startup
    /// (`min(available_parallelism(), 8)`). `Some(n)` sets an explicit count.
    /// Validated post-load by `Config::validate()`: must be 1..=64 when set
    /// (the 64 ceiling bounds peak encoder RAM: threads × frame buffers).
    /// Audio stages are unaffected — audio encoders are single-threaded.
    pub recode_threads: Option<u32>,
    /// Encoder preset override for video recode (e.g. `"faster"`, `"medium"`).
    /// `None` = use `RecodeStage`'s per-codec default preset. When `Some`, this
    /// value replaces that default and is passed verbatim to the encoder.
    pub recode_preset: Option<String>,
    /// libvpx `-deadline` (VP8/VP9). `None` = encoder default (`good`).
    pub recode_deadline: Option<VpxDeadline>,
    /// libvpx `-cpu-used` (VP8: -16..=16, VP9: -8..=8). `None` = encoder default.
    pub recode_cpu_used: Option<i32>,
    /// libxavs2 `-speed_level` (0..=9). `None` = encoder default (0).
    pub recode_speed_level: Option<u32>,
    /// Policy for automatic fixup of downloaded files.
    pub fixup: FixupPolicy,
}

impl PostProcess {
    /// The container the user explicitly requested, resolved in precedence
    /// order: `recode_container` > `recode_video` > `remux_container`.
    /// `None` means no explicit container was requested anywhere, i.e. rdlp
    /// is free to choose one itself.
    ///
    /// The recode target outranks the remux target because `RecodeStage`
    /// (pipeline index 4) runs AFTER `RemuxStage` (index 3), so when both are
    /// set it is the recode target that survives to the end of the pipeline.
    /// yt-dlp resolves the same conflict the same way, clearing `remuxvideo`
    /// with `"--remux-video is ignored since --recode-video was given"`.
    ///
    /// Single source of truth for this precedence chain — do NOT re-spell the
    /// `.or()` chain at a call site. A hand-copied mirror of it in
    /// `ThumbnailStage` is exactly what shipped #551: the guard keyed on
    /// `remux_container` alone and silently discarded `--recode-video=ts`.
    ///
    /// Callers needing a hard default apply `.unwrap_or(..)` themselves —
    /// *which* default is a per-consumer concern, and `ThumbnailStage`
    /// genuinely wants the `None`.
    #[must_use]
    pub fn explicit_container(&self) -> Option<ExplicitContainer> {
        // Deliberately NOT a destructuring pattern: `PostProcess` has ~35
        // fields, so binding every one to force a compile error when a field
        // is added would fire on every unrelated addition (`recode_threads`,
        // `loudnorm_*`, ...) — a false-positive treadmill, not a safety net.
        // The doc-comment above is the contract; the unit tests pin the order.
        self.recode_container
            .map(|format| ExplicitContainer {
                flag: "--recode-container",
                format,
            })
            .or_else(|| {
                self.recode_video.map(|format| ExplicitContainer {
                    flag: "--recode-video",
                    format,
                })
            })
            .or_else(|| {
                self.remux_container.map(|format| ExplicitContainer {
                    flag: "--remux",
                    format,
                })
            })
    }
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
            recode_threads: None,
            recode_preset: None,
            recode_deadline: None,
            recode_cpu_used: None,
            recode_speed_level: None,
            fixup: FixupPolicy::default(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn default_embed_thumbnail_is_true() {
        assert!(PostProcess::default().embed_thumbnail);
    }

    // --- explicit_container precedence chain (#551) ---
    //
    // Each field is exercised alone (so no arm is dead), then every pairwise
    // conflict is exercised (so the ORDER is pinned, not just the membership).
    // A chain reordered to put `remux_container` first still passes the
    // alone-cases; only the conflict cases catch it.

    #[test]
    fn explicit_container_is_none_when_nothing_requested() {
        assert_eq!(PostProcess::default().explicit_container(), None);
    }

    #[test]
    fn explicit_container_resolves_recode_container_alone() {
        let config = PostProcess {
            recode_container: Some(ContainerFormat::Ts),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::Ts);
        assert_eq!(explicit.flag, "--recode-container");
    }

    #[test]
    fn explicit_container_resolves_recode_video_alone() {
        let config = PostProcess {
            recode_video: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::Mkv);
        assert_eq!(explicit.flag, "--recode-video");
    }

    #[test]
    fn explicit_container_resolves_remux_container_alone() {
        let config = PostProcess {
            remux_container: Some(ContainerFormat::F4v),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::F4v);
        assert_eq!(explicit.flag, "--remux");
    }

    #[test]
    fn recode_container_outranks_recode_video() {
        let config = PostProcess {
            recode_container: Some(ContainerFormat::Ts),
            recode_video: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::Ts);
        assert_eq!(explicit.flag, "--recode-container");
    }

    /// `RecodeStage` (index 4) runs after `RemuxStage` (index 3), so the
    /// recode target is what survives to the end of the pipeline.
    #[test]
    fn recode_video_outranks_remux_container() {
        let config = PostProcess {
            recode_video: Some(ContainerFormat::Ts),
            remux_container: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::Ts);
        assert_eq!(explicit.flag, "--recode-video");
    }

    #[test]
    fn recode_container_outranks_remux_container() {
        let config = PostProcess {
            recode_container: Some(ContainerFormat::Ts),
            remux_container: Some(ContainerFormat::Mkv),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::Ts);
        assert_eq!(explicit.flag, "--recode-container");
    }

    /// All three set at once — the full chain resolves to the head.
    #[test]
    fn recode_container_wins_the_full_three_way_chain() {
        let config = PostProcess {
            recode_container: Some(ContainerFormat::Ts),
            recode_video: Some(ContainerFormat::Mkv),
            remux_container: Some(ContainerFormat::F4v),
            ..PostProcess::default()
        };
        let explicit = config.explicit_container().expect("some");
        assert_eq!(explicit.format, ContainerFormat::Ts);
        assert_eq!(explicit.flag, "--recode-container");
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

    #[test]
    fn recode_threads_and_preset_default_to_none() {
        let pp = PostProcess::default();
        assert_eq!(pp.recode_threads, None);
        assert_eq!(pp.recode_preset, None);
    }

    #[test]
    fn speed_control_fields_default_none() {
        let pp = PostProcess::default();
        assert_eq!(pp.recode_deadline, None);
        assert_eq!(pp.recode_cpu_used, None);
        assert_eq!(pp.recode_speed_level, None);
    }
}
