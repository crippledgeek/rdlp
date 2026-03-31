//! Application configuration types

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use crate::browser_type::BrowserType;
use crate::postprocess::PostProcess;
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
    /// Post-processing configuration (remux, recode, audio, metadata, etc.).
    #[serde(default)]
    pub postprocess: PostProcess,

    // === Subtitles ===
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

    // === Filtering ===
    /// Match filter expressions (OR logic between multiple filters, AND within each).
    /// Evaluated against InfoDict before download — non-matching videos are skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_filters: Vec<String>,

    // === Plugin system ===
    /// Directories to search for plugins
    pub plugin_directories: Vec<PathBuf>,

    /// List of enabled plugins (None = all plugins enabled)
    pub enabled_plugins: Option<Vec<String>>,

    /// Load dynamic plugins
    pub load_plugins: bool,

    // === Download performance ===
    /// Enable adaptive chunk sizing and connection tuning for downloads.
    /// When true, the downloader adjusts chunk sizes and parallel connections
    /// based on observed throughput using an AIMD algorithm.
    #[serde(default = "default_true")]
    pub adaptive_downloads: bool,
}

fn default_true() -> bool {
    true
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
            postprocess: PostProcess::default(),

            // Subtitles
            write_auto_subtitles: false,
            subtitle_langs: Vec::new(),
            subtitle_format: None,
            list_subs: false,
            strict_subs: false,
            verify_sub_urls: false,
            retry_subs: false,

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

            // Filtering
            match_filters: Vec::new(),

            // Plugin system
            plugin_directories: Vec::new(),
            enabled_plugins: None,
            load_plugins: true,

            // Download performance
            adaptive_downloads: true,
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
        if let Some(end) = self.playlist_end
            && end < self.playlist_start
        {
            return Err(ConfigValidationError::InvalidPlaylistRange {
                start: self.playlist_start,
                end,
            });
        }

        // Stdout mode rejects post-processing options that require file I/O
        if self.output_to_stdout {
            let pp = &self.postprocess;
            let incompatible: &[(&str, bool)] = &[
                ("extract-audio", pp.extract_audio),
                ("remux", pp.remux_container.is_some()),
                ("recode-video", pp.recode_video.is_some()),
                ("embed-metadata", pp.embed_metadata),
                ("embed-thumbnail", pp.embed_thumbnail),
                ("embed-subtitles", pp.embed_subtitles),
                ("normalize-audio", pp.normalize_audio),
                ("loudnorm", pp.loudnorm),
                ("write-subtitles", pp.write_subtitles),
                ("write-thumbnail", pp.write_thumbnail),
                ("normalize-boost", pp.normalize_boost),
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
