use async_trait::async_trait;
use std::path::PathBuf;

use crate::{InfoDict, Result};

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
    pub fn new(info: InfoDict, files: Vec<PathBuf>) -> Self {
        Self {
            info,
            files,
            temp_files: Vec::new(),
        }
    }

    /// Add temporary files that can be cleaned up
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

    /// Audio format to convert to ("mp3", "m4a", "opus", etc.)
    pub audio_format: Option<String>,

    /// Audio quality (VBR level or bitrate)
    pub audio_quality: Option<String>,

    /// Video format to recode to
    pub recode_video: Option<String>,

    /// Merge output format (when combining video+audio)
    pub merge_output_format: Option<String>,

    /// Embed thumbnail in video file
    pub embed_thumbnail: bool,

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
}

impl Default for PostProcessConfig {
    fn default() -> Self {
        Self {
            extract_audio: false,
            audio_format: None,
            audio_quality: None,
            recode_video: None,
            merge_output_format: Some("mp4".to_string()),
            embed_thumbnail: false,
            embed_metadata: false,
            embed_subtitles: false,
            keep_video: false,
            ffmpeg_location: None,
            ffmpeg_args: Vec::new(),
        }
    }
}
