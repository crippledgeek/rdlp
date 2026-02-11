use async_trait::async_trait;
use std::path::PathBuf;

use crate::{AudioFormat, ContainerFormat, InfoDict, Result};

/// Post-processing operations on downloaded files
///
/// Post-processors transform downloaded files (merge video+audio, extract audio,
/// embed metadata, thumbnails, etc.). They run after downloads complete.
#[async_trait]
pub trait PostProcessor: Send + Sync {
    /// Name of the post-processor (e.g., "FFmpeg", "EmbedThumbnail", "Metadata")
    fn name(&self) -> &str;

    /// Process the downloaded file(s)
    ///
    /// # Arguments
    /// * `info` - Video metadata
    /// * `files` - List of downloaded file paths to process
    /// * `config` - Post-processing configuration
    ///
    /// # Returns
    /// Updated InfoDict and potentially new file paths after processing
    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
    ) -> Result<PostProcessResult>;

    /// Check if this post-processor should run based on config
    ///
    /// # Arguments
    /// * `info` - Video metadata
    /// * `config` - Post-processing configuration
    ///
    /// # Returns
    /// `true` if this post-processor should run
    fn should_run(&self, info: &InfoDict, config: &PostProcessConfig) -> bool;

    /// Priority for this post-processor (higher runs first)
    ///
    /// Default is 0. Use this to ensure post-processors run in the correct order.
    /// For example, merging video+audio should happen before embedding thumbnails.
    fn priority(&self) -> i32 {
        0
    }
}

/// Result of post-processing
#[derive(Debug, Clone)]
pub struct PostProcessResult {
    /// Updated InfoDict (may contain new metadata)
    pub info: InfoDict,

    /// Output file paths after processing
    pub files: Vec<PathBuf>,

    /// Files that can be deleted (intermediate files)
    pub temp_files: Vec<PathBuf>,
}

impl PostProcessResult {
    /// Create a new post-process result
    #[must_use]
    pub fn new(info: InfoDict, files: Vec<PathBuf>) -> Self {
        Self {
            info,
            files,
            temp_files: Vec::new(),
        }
    }

    /// Add temporary files that can be cleaned up
    #[must_use]
    pub fn with_temp_files(mut self, temp_files: Vec<PathBuf>) -> Self {
        self.temp_files = temp_files;
        self
    }
}

/// Post-processing configuration
#[derive(Debug, Clone)]
pub struct PostProcessConfig {
    /// Extract audio only
    pub extract_audio: bool,

    /// Audio format to convert to
    pub audio_format: Option<AudioFormat>,

    /// Audio quality (VBR level or bitrate)
    pub audio_quality: Option<String>,

    /// Video format to recode to
    pub recode_video: Option<ContainerFormat>,

    /// Remux to container format (stream copy, no re-encoding)
    pub remux_container: Option<ContainerFormat>,

    /// Merge output format (when combining video+audio)
    pub merge_output_format: Option<ContainerFormat>,

    /// Embed thumbnail in video file
    pub embed_thumbnail: bool,

    /// Write thumbnail image to disk (keep after embedding)
    pub write_thumbnail: bool,

    /// Embed metadata (title, artist, etc.)
    pub embed_metadata: bool,

    /// Embed subtitles in video file
    pub embed_subtitles: bool,

    /// Keep original files after processing
    pub keep_video: bool,

    /// FFmpeg location (if not in PATH)
    pub ffmpeg_location: Option<PathBuf>,

    /// Additional FFmpeg arguments
    pub ffmpeg_args: Vec<String>,

    /// Normalize audio levels (peak mode unless loudnorm is set)
    pub normalize_audio: bool,

    /// Use EBU R128 loudnorm normalization (implies normalize_audio)
    pub loudnorm: bool,

    /// Target peak level in dBFS for peak normalization (default -1.0)
    pub audio_gain_target: Option<f64>,

    /// Loudnorm preset name (broadcast, streaming, loud)
    pub loudnorm_preset: Option<String>,

    /// Target integrated loudness in LUFS for loudnorm (default -14.0)
    pub loudnorm_target_i: Option<f64>,

    /// Target true peak in dBTP for loudnorm (default -1.5)
    pub loudnorm_target_tp: Option<f64>,

    /// Target loudness range in LU for loudnorm (default 11.0)
    pub loudnorm_target_lra: Option<f64>,

    /// Force dynamic (per-frame compression) mode in loudnorm pass 2
    pub loudnorm_dynamic: bool,

    /// Prepend a mild acompressor before loudnorm in pass 2
    pub loudnorm_precompress: bool,

    /// Enable limiter-boost fallback for over-compressed content
    pub normalize_boost: bool,

    /// Gain in dB for limiter-boost fallback (default 12.0 when None)
    pub normalize_boost_db: Option<f64>,
}

impl Default for PostProcessConfig {
    fn default() -> Self {
        Self {
            extract_audio: false,
            audio_format: None,
            audio_quality: None,
            recode_video: None,
            remux_container: None,
            merge_output_format: Some(ContainerFormat::Mp4),
            embed_thumbnail: false,
            write_thumbnail: false,
            embed_metadata: false,
            embed_subtitles: false,
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
        }
    }
}
