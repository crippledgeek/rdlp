//! Application configuration types

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use crate::audio_format::AudioFormat;
use crate::browser_type::BrowserType;
use crate::container::ContainerFormat;
use crate::subtitle_format::SubtitleFormat;

/// Errors from configuration validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValidationError {
    /// `concurrent_fragments` must be at least 1
    InvalidConcurrentFragments,
    /// `buffer_size` must be at least 1
    InvalidBufferSize,
    /// `playlist_start` must be at least 1
    InvalidPlaylistStart,
    /// `playlist_end` must be >= `playlist_start`
    InvalidPlaylistRange {
        /// Configured start
        start: usize,
        /// Configured end
        end: usize,
    },
    /// A post-processing option is incompatible with stdout output (`-o -`).
    ///
    /// Uses `String` rather than a typed enum because these are CLI flag names
    /// (`--extract-audio`, `--remux`, etc.) used only for error display. The set
    /// grows as new post-processing flags are added.
    StdoutIncompatible {
        /// The incompatible CLI option name (e.g. `"extract-audio"`)
        option: String,
    },
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConcurrentFragments => {
                write!(f, "concurrent_fragments must be at least 1")
            }
            Self::InvalidBufferSize => write!(f, "buffer_size must be at least 1"),
            Self::InvalidPlaylistStart => write!(f, "playlist_start must be at least 1"),
            Self::InvalidPlaylistRange { start, end } => {
                write!(
                    f,
                    "playlist_end ({end}) must be >= playlist_start ({start})"
                )
            }
            Self::StdoutIncompatible { option } => {
                write!(f, "--{option} is not compatible with -o - (stdout output)")
            }
        }
    }
}

impl std::error::Error for ConfigValidationError {}

/// Application configuration
///
/// This structure holds all configuration options for rdlp, including
/// output settings, format selection, download options, post-processing, etc.
///
/// For file I/O operations (loading from TOML/YAML), use the extension
/// functions in `rdlp_core::config_io`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // === Output options ===
    /// Stream output to stdout instead of a file (`-o -`)
    pub output_to_stdout: bool,

    /// Output filename template (e.g., "%(title)s.%(ext)s")
    pub output_template: String,

    /// Output directory path
    pub output_directory: PathBuf,

    /// Restrict filenames to ASCII characters
    pub restrict_filenames: bool,

    /// Overwrite existing files
    pub overwrite: bool,

    /// Continue incomplete downloads
    pub continue_downloads: bool,

    /// Don't use .part files
    pub no_part: bool,

    // === Format selection ===
    /// Format selection expression.
    /// `None` means "use dynamic default based on runtime capabilities".
    /// `Some(...)` means the user (or config file) explicitly set this.
    pub format: Option<String>,

    /// Merge output format when combining video+audio
    pub merge_output_format: Option<ContainerFormat>,

    /// Require strict video-only + audio-only streams for merge selection.
    /// When true, default selector changes from `bv*+ba/b` to `bv+ba/b`.
    pub audio_multistreams: bool,

    // === Download options ===
    /// Number of concurrent fragments to download
    pub concurrent_fragments: usize,

    /// Rate limit in bytes per second
    pub rate_limit: Option<u64>,

    /// Number of retries for failed downloads
    pub retries: usize,

    /// Number of retries for failed fragment downloads
    pub fragment_retries: usize,

    /// Initial retry delay in milliseconds (default: 1000ms)
    pub retry_initial_delay_ms: u64,

    /// Maximum retry delay in milliseconds (default: 60000ms = 1 minute)
    pub retry_max_delay_ms: u64,

    /// Exponential backoff multiplier (default: 2.0)
    pub retry_backoff_multiplier: f64,

    /// Buffer size for downloads (bytes)
    pub buffer_size: usize,

    // === Network options ===
    /// HTTP/SOCKS proxy URL
    pub proxy: Option<String>,

    /// Socket timeout in seconds
    pub socket_timeout: Option<u64>,

    /// Source IP address to bind to
    pub source_address: Option<String>,

    /// User agent string
    pub user_agent: Option<String>,

    /// Custom HTTP headers
    pub http_headers: Vec<(String, String)>,

    // === Post-processing ===
    /// Extract audio only
    pub extract_audio: bool,

    /// Audio format to convert to
    pub audio_format: Option<AudioFormat>,

    /// Audio quality (VBR level or bitrate, e.g., "192K")
    pub audio_quality: Option<String>,

    /// Video format to recode to
    pub recode_video: Option<ContainerFormat>,

    /// Remux to container format for better seeking
    pub remux_container: Option<ContainerFormat>,

    /// Embed thumbnail in video file
    pub embed_thumbnail: bool,

    /// Embed metadata in file
    pub embed_metadata: bool,

    /// Embed subtitles in video file
    pub embed_subtitles: bool,

    /// Keep original files after post-processing
    pub keep_video: bool,

    /// Normalize audio levels (peak mode unless loudnorm is set)
    pub normalize_audio: bool,

    /// Use EBU R128 loudnorm normalization (implies normalize_audio)
    pub loudnorm: bool,

    /// Target peak level in dBFS for peak normalization (default -1.0)
    pub audio_gain_target: Option<f64>,

    /// Loudnorm preset name (broadcast, streaming, loud)
    pub loudnorm_preset: Option<String>,

    /// Target integrated loudness in LUFS for loudnorm
    pub loudnorm_target_i: Option<f64>,

    /// Target true peak in dBTP for loudnorm
    pub loudnorm_target_tp: Option<f64>,

    /// Target loudness range in LU for loudnorm
    pub loudnorm_target_lra: Option<f64>,

    /// Force dynamic (per-frame compression) mode in loudnorm pass 2
    pub loudnorm_dynamic: bool,

    /// Prepend a mild acompressor before loudnorm in pass 2
    pub loudnorm_precompress: bool,

    /// Enable limiter-boost fallback for over-compressed content
    pub normalize_boost: bool,

    /// Gain in dB for limiter-boost fallback (default 12.0 when None)
    pub normalize_boost_db: Option<f64>,

    /// FFmpeg location (if not in PATH)
    pub ffmpeg_location: Option<PathBuf>,

    /// Additional FFmpeg arguments
    pub ffmpeg_args: Vec<String>,

    // === Subtitles ===
    /// Write subtitle files
    pub write_subtitles: bool,

    /// Write automatic captions
    pub write_auto_subtitles: bool,

    /// Subtitle languages to download (e.g., ["en", "es"])
    pub subtitle_langs: Vec<String>,

    /// Subtitle format
    pub subtitle_format: Option<SubtitleFormat>,

    /// Show interactive subtitle selection (--list-subs)
    pub list_subs: bool,

    /// Strict subtitle mode: fail download if requested subs are missing
    pub strict_subs: bool,

    /// Verify subtitle URLs via HEAD request before download
    pub verify_sub_urls: bool,

    /// Re-attempt subtitle downloads for completed videos missing subtitle files
    pub retry_subs: bool,

    // === Thumbnail ===
    /// Write thumbnail image to disk
    pub write_thumbnail: bool,

    // === Verbosity ===
    /// Quiet mode (minimal output)
    pub quiet: bool,

    /// Verbose mode (detailed output)
    pub verbose: bool,

    /// Print progress bar
    pub progress: bool,

    // === Simulation ===
    /// Simulate download (don't download anything)
    pub simulate: bool,

    /// Skip download, only write info JSON
    pub skip_download: bool,

    // === Playlist options ===
    /// Process playlist/channel entries
    pub extract_playlist: bool,

    /// Playlist start index (1-based)
    pub playlist_start: usize,

    /// Playlist end index (1-based, inclusive)
    pub playlist_end: Option<usize>,

    /// Download only matching playlist items
    pub playlist_items: Option<String>,

    // === Authentication ===
    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication
    pub password: Option<String>,

    /// Two-factor authentication code
    pub two_factor: Option<String>,

    /// Path to .netrc file
    pub netrc: bool,

    // === Cookies ===
    /// Browser to extract cookies from
    pub cookies_from_browser: Option<BrowserType>,

    /// Path to cookies file (Netscape format)
    pub cookies_file: Option<PathBuf>,

    // === Archive ===
    /// Path to download archive file (logs completed downloads to skip on re-run)
    pub download_archive: Option<PathBuf>,

    // === Plugin system ===
    /// Directories to search for plugins
    pub plugin_directories: Vec<PathBuf>,

    /// List of enabled plugins (None = all plugins enabled)
    pub enabled_plugins: Option<Vec<String>>,

    /// Load dynamic plugins
    pub load_plugins: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Output options
            output_to_stdout: false,
            output_template: "%(title)s.%(ext)s".to_string(),
            output_directory: PathBuf::from("."),
            restrict_filenames: false,
            overwrite: false,
            continue_downloads: true,
            no_part: false,

            // Format selection
            format: None,
            merge_output_format: None,
            audio_multistreams: false,

            // Download options
            concurrent_fragments: 4,
            rate_limit: None,
            retries: 10,
            fragment_retries: 10,
            retry_initial_delay_ms: 1000,  // 1 second
            retry_max_delay_ms: 60000,     // 60 seconds
            retry_backoff_multiplier: 2.0, // Double delay each retry
            buffer_size: 2 * 1024 * 1024,  // 2 MB - larger buffer for faster downloads

            // Network options
            proxy: None,
            socket_timeout: Some(30),
            source_address: None,
            user_agent: Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".to_string(),
            ),
            http_headers: Vec::new(),

            // Post-processing
            extract_audio: false,
            audio_format: None,
            audio_quality: None,
            recode_video: None,
            remux_container: None,
            embed_thumbnail: true,
            embed_metadata: false,
            embed_subtitles: false,
            keep_video: false,
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
            ffmpeg_location: None,
            ffmpeg_args: Vec::new(),

            // Subtitles
            write_subtitles: false,
            write_auto_subtitles: false,
            subtitle_langs: Vec::new(),
            subtitle_format: None,
            list_subs: false,
            strict_subs: false,
            verify_sub_urls: false,
            retry_subs: false,

            // Thumbnail
            write_thumbnail: false,

            // Verbosity
            quiet: false,
            verbose: false,
            progress: true,

            // Simulation
            simulate: false,
            skip_download: false,

            // Playlist options
            extract_playlist: true,
            playlist_start: 1,
            playlist_end: None,
            playlist_items: None,

            // Authentication
            username: None,
            password: None,
            two_factor: None,
            netrc: false,

            // Cookies
            cookies_from_browser: None,
            cookies_file: None,

            // Archive
            download_archive: None,

            // Plugin system
            plugin_directories: Vec::new(),
            enabled_plugins: None,
            load_plugins: true,
        }
    }
}

impl Config {
    /// Validate configuration and return errors if invalid.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.concurrent_fragments == 0 {
            return Err(ConfigValidationError::InvalidConcurrentFragments);
        }
        if self.buffer_size == 0 {
            return Err(ConfigValidationError::InvalidBufferSize);
        }
        if self.playlist_start == 0 {
            return Err(ConfigValidationError::InvalidPlaylistStart);
        }
        if let Some(end) = self.playlist_end {
            if end < self.playlist_start {
                return Err(ConfigValidationError::InvalidPlaylistRange {
                    start: self.playlist_start,
                    end,
                });
            }
        }

        // Stdout mode rejects post-processing options that require file I/O
        if self.output_to_stdout {
            let incompatible: &[(&str, bool)] = &[
                ("extract-audio", self.extract_audio),
                ("remux", self.remux_container.is_some()),
                ("recode-video", self.recode_video.is_some()),
                ("embed-metadata", self.embed_metadata),
                ("embed-thumbnail", self.embed_thumbnail),
                ("embed-subtitles", self.embed_subtitles),
                ("normalize-audio", self.normalize_audio),
                ("loudnorm", self.loudnorm),
                ("write-subtitles", self.write_subtitles),
                ("write-thumbnail", self.write_thumbnail),
                ("normalize-boost", self.normalize_boost),
            ];
            for &(option, active) in incompatible {
                if active {
                    return Err(ConfigValidationError::StdoutIncompatible {
                        option: option.to_string(),
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
