//! Configuration option types for `FFmpeg` operations.
//!
//! Provides `RemuxOptions`, `AudioExtractOptions`, `VideoConvertOptions`,
//! and `ChapterEntry` used across remux, merge, transcode, and metadata modules.

use std::path::Path;

/// Whether output written to `path` should enable faststart.
///
/// This is the **boundary** form of the question: these callers receive only a
/// filesystem path and have no `ContainerFormat` in scope, so the extension is
/// parsed into the domain type once, here, and the answer comes from
/// [`ContainerFormat::supports_faststart`] rather than a local string list.
///
/// Interior callers that already hold a `ContainerFormat` (the remux and merge
/// pipeline stages) must call `supports_faststart()` directly instead of
/// round-tripping through a string.
///
/// An unrecognised extension answers `false` — matching the previous behaviour
/// for unknown containers.
pub fn faststart_for_output(path: &Path) -> bool {
    rdlp_types::ContainerFormat::from_path(path).is_some_and(|c| c.supports_faststart())
}

/// Options for remux and merge operations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemuxOptions {
    /// Enable MP4 faststart (moov atom at beginning of file).
    pub faststart: bool,
    /// Force output format (e.g., "mp4", "mkv").
    pub output_format: Option<String>,
    /// Override the `encoding_tool` metadata tag written by this operation.
    ///
    /// When `Some`, this value is used instead of the default stage name
    /// (e.g., "remux", "merge", "thumbnail"). Pass `msg.encoding_tool` from
    /// the pipeline to propagate the tag set by a prior content-creating stage.
    pub encoding_tool_override: Option<String>,
}

/// Options for audio extraction and transcoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioExtractOptions {
    /// Encoder name (e.g., "libmp3lame", "aac", "libopus").
    /// If None, uses the default encoder for the output format.
    pub encoder_name: Option<String>,
    /// If true, copy audio stream without re-encoding.
    pub copy: bool,
    /// Target bitrate in kbps (e.g., 192 for 192kbps).
    pub bitrate_kbps: Option<u32>,
    /// VBR quality scale value (codec-specific).
    /// For MP3: 0 (best) to 9 (worst).
    /// For Vorbis: 0 (worst) to 10 (best).
    pub quality_scale: Option<i32>,
}

/// Options for video conversion/transcoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoConvertOptions {
    /// If true, remux only (stream copy, no re-encoding).
    pub remux_only: bool,
    /// Video encoder name (e.g., "libx264", "libx265", "libvpx-vp9").
    pub video_codec: Option<String>,
    /// Encoder preset (e.g., "medium", "fast", "slow").
    pub preset: Option<String>,
    /// Constant Rate Factor for quality-based encoding.
    pub crf: Option<u32>,
    /// Resolved encoder thread count. `None` = let the encode layer resolve it
    /// from `available_parallelism()`. Audio encoding is unaffected.
    pub threads: Option<u32>,
    /// libvpx `-deadline` value, already lowered to its name string (e.g. `"good"`)
    /// from the typed `VpxDeadline` at the `RecodeStage` boundary.
    /// Validated upstream by `validate_speed_controls`.
    pub deadline: Option<String>,
    /// libvpx `-cpu-used`.
    pub cpu_used: Option<i32>,
    /// libxavs2 `-speed_level` (0..=9). `None` = encoder default (0).
    pub speed_level: Option<u32>,
    /// If true, copy audio stream without re-encoding.
    ///
    /// Takes precedence over `audio_codec`. When `audio_copy` is true and
    /// `audio_codec` is `Some`, audio is still copied unchanged.
    pub audio_copy: bool,
    /// Audio encoder name to use for audio re-encoding (e.g., "`libfdk_aac`", "libopus").
    ///
    /// Only used when `audio_copy` is false. When `None` and `audio_copy` is false,
    /// the existing behavior is preserved (implementation decides the encoder).
    pub audio_codec: Option<String>,
    /// When true, capture `FFmpeg` C-level log messages and forward them via
    /// the log callback. Enables verbose encoder output in the UI log viewer.
    pub verbose: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn video_convert_options_threads_defaults_none() {
        let opts = VideoConvertOptions::default();
        assert_eq!(opts.threads, None);
    }

    /// Every ISOBMFF container `supports_faststart()` claims must get faststart.
    ///
    /// `m4v` and `f4v` are the #539 regression: they route to the same
    /// mov/mp4 muxer as `mp4`/`mov` and `+faststart` demonstrably relocates
    /// their `moov` atom, but a hardcoded `"mp4" | "mov"` list silently
    /// excluded them.
    #[test]
    fn faststart_enabled_for_every_container_that_supports_it() {
        for fmt in ["mp4", "mov", "m4v", "f4v"] {
            let path = PathBuf::from(format!("out.{fmt}"));
            assert!(
                faststart_for_output(&path),
                ".{fmt} supports faststart (ContainerFormat::supports_faststart) \
                 but faststart_for_output said false"
            );
        }
    }

    /// Containers outside the mov/mp4 family must not get `movflags`.
    #[test]
    fn faststart_disabled_for_non_isobmff_containers() {
        for fmt in ["mkv", "webm", "avi", "ts", "flv", "mp3", "flac"] {
            let path = PathBuf::from(format!("out.{fmt}"));
            assert!(
                !faststart_for_output(&path),
                ".{fmt} does not support faststart but faststart_for_output said true"
            );
        }
    }

    /// Extension matching stays case-insensitive.
    #[test]
    fn faststart_is_case_insensitive() {
        assert!(faststart_for_output(&PathBuf::from("out.MP4")));
        assert!(faststart_for_output(&PathBuf::from("out.M4V")));
        assert!(!faststart_for_output(&PathBuf::from("out.MKV")));
    }

    /// A missing or unrecognised extension answers `false`, never panics.
    #[test]
    fn faststart_false_for_unknown_or_missing_extension() {
        assert!(!faststart_for_output(&PathBuf::from("out.qqq")));
        assert!(!faststart_for_output(&PathBuf::from("out")));
        assert!(!faststart_for_output(&PathBuf::from("")));
    }

    /// Aliases resolve to their container, so faststart follows the container.
    ///
    /// `ContainerFormat`'s `FromStr` is alias-aware, which the old two-literal
    /// comparison was not: a `.quicktime` file is a `QuickTime` file.
    #[test]
    fn faststart_honours_container_aliases() {
        assert!(faststart_for_output(&PathBuf::from("out.quicktime"))); // → Mov
        assert!(!faststart_for_output(&PathBuf::from("out.matroska"))); // → Mkv
    }

    /// The `fixup` caller defaults an extensionless input to `mp4`, so the
    /// output path it builds is `.mp4` and must get faststart.
    #[test]
    fn faststart_true_for_the_mp4_default_fixup_builds() {
        assert!(faststart_for_output(&PathBuf::from("v.rdlp-tmp-abc.mp4")));
    }
}

/// A chapter entry for metadata embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterEntry {
    /// Chapter ID (unique, typically sequential starting from 0).
    pub id: i64,
    /// Start time in milliseconds.
    pub start_ms: i64,
    /// End time in milliseconds.
    pub end_ms: i64,
    /// Chapter title.
    pub title: String,
}
