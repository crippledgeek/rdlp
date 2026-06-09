//! Configuration option types for `FFmpeg` operations.
//!
//! Provides `RemuxOptions`, `AudioExtractOptions`, `VideoConvertOptions`,
//! and `ChapterEntry` used across remux, merge, transcode, and metadata modules.

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
    /// libvpx `-deadline` value (already a name string, e.g. "good").
    pub deadline: Option<String>,
    /// libvpx `-cpu-used`.
    pub cpu_used: Option<i32>,
    /// libxavs2 `-speed_level`.
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

    #[test]
    fn video_convert_options_threads_defaults_none() {
        let opts = VideoConvertOptions::default();
        assert_eq!(opts.threads, None);
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
