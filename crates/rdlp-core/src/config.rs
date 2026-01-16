use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration
///
/// This structure holds all configuration options for rdlp, including
/// output settings, format selection, download options, post-processing, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // === Output options ===
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
    /// Format selection expression (default: "bestvideo*+bestaudio/best")
    pub format: String,

    /// Merge output format when combining video+audio (e.g., "mp4", "mkv")
    pub merge_output_format: Option<String>,

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

    /// Audio format to convert to ("mp3", "m4a", "opus", etc.)
    pub audio_format: Option<String>,

    /// Audio quality (VBR level or bitrate, e.g., "192K")
    pub audio_quality: Option<String>,

    /// Video format to recode to
    pub recode_video: Option<String>,

    /// Embed thumbnail in video file
    pub embed_thumbnail: bool,

    /// Embed metadata in file
    pub embed_metadata: bool,

    /// Embed subtitles in video file
    pub embed_subtitles: bool,

    /// Keep original files after post-processing
    pub keep_video: bool,

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

    /// Subtitle format ("srt", "vtt", "ass", etc.)
    pub subtitle_format: Option<String>,

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
    /// Browser to extract cookies from ("chrome", "firefox", "safari", etc.)
    pub cookies_from_browser: Option<String>,

    /// Path to cookies file (Netscape format)
    pub cookies_file: Option<PathBuf>,

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
            output_template: "%(title)s.%(ext)s".to_string(),
            output_directory: PathBuf::from("."),
            restrict_filenames: false,
            overwrite: false,
            continue_downloads: true,
            no_part: false,

            // Format selection
            format: "bestvideo*+bestaudio/best".to_string(),
            merge_output_format: Some("mp4".to_string()),

            // Download options
            concurrent_fragments: 4,
            rate_limit: None,
            retries: 10,
            fragment_retries: 10,
            retry_initial_delay_ms: 1000,      // 1 second
            retry_max_delay_ms: 60000,         // 60 seconds
            retry_backoff_multiplier: 2.0,     // Double delay each retry
            buffer_size: 2 * 1024 * 1024, // 2 MB - larger buffer for faster downloads

            // Network options
            proxy: None,
            socket_timeout: Some(30),
            source_address: None,
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".to_string()),
            http_headers: Vec::new(),

            // Post-processing
            extract_audio: false,
            audio_format: None,
            audio_quality: None,
            recode_video: None,
            embed_thumbnail: false,
            embed_metadata: false,
            embed_subtitles: false,
            keep_video: false,
            ffmpeg_location: None,
            ffmpeg_args: Vec::new(),

            // Subtitles
            write_subtitles: false,
            write_auto_subtitles: false,
            subtitle_langs: Vec::new(),
            subtitle_format: None,

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

            // Plugin system
            plugin_directories: Vec::new(),
            enabled_plugins: None,
            load_plugins: true,
        }
    }
}

impl Config {
    /// Create a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Load configuration from a TOML file
    pub fn from_toml_file(path: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let config: Self = toml::from_str(&content)
            .map_err(|e| crate::RdlpError::Config(format!("Failed to parse TOML: {e}")))?;
        Ok(config)
    }

    /// Load configuration from a YAML file
    pub fn from_yaml_file(path: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let config: Self = serde_yaml::from_str(&content)
            .map_err(|e| crate::RdlpError::Config(format!("Failed to parse YAML: {e}")))?;
        Ok(config)
    }

    /// Save configuration to a TOML file
    pub fn to_toml_file(&self, path: impl AsRef<std::path::Path>) -> crate::Result<()> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| crate::RdlpError::Config(format!("Failed to serialize TOML: {e}")))?;
        std::fs::write(path.as_ref(), content)?;
        Ok(())
    }

    /// Save configuration to a YAML file
    pub fn to_yaml_file(&self, path: impl AsRef<std::path::Path>) -> crate::Result<()> {
        let content = serde_yaml::to_string(self)
            .map_err(|e| crate::RdlpError::Config(format!("Failed to serialize YAML: {e}")))?;
        std::fs::write(path.as_ref(), content)?;
        Ok(())
    }

    /// Validate configuration and return errors if invalid
    pub fn validate(&self) -> crate::Result<()> {
        // Validate concurrent_fragments
        if self.concurrent_fragments == 0 {
            return Err(crate::RdlpError::Config(
                "concurrent_fragments must be at least 1".to_string(),
            ));
        }

        // Validate buffer_size
        if self.buffer_size == 0 {
            return Err(crate::RdlpError::Config(
                "buffer_size must be at least 1".to_string(),
            ));
        }

        // Validate playlist_start
        if self.playlist_start == 0 {
            return Err(crate::RdlpError::Config(
                "playlist_start must be at least 1".to_string(),
            ));
        }

        // Validate playlist_end
        if let Some(end) = self.playlist_end {
            if end < self.playlist_start {
                return Err(crate::RdlpError::Config(
                    "playlist_end must be >= playlist_start".to_string(),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.output_template, "%(title)s.%(ext)s");
        assert_eq!(config.format, "bestvideo*+bestaudio/best");
        assert!(config.continue_downloads);
        assert_eq!(config.concurrent_fragments, 4);
    }

    #[test]
    fn test_validate_config() {
        let mut config = Config::default();

        // Valid config
        assert!(config.validate().is_ok());

        // Invalid concurrent_fragments
        config.concurrent_fragments = 0;
        assert!(config.validate().is_err());

        // Fix and test buffer_size
        config.concurrent_fragments = 4;
        config.buffer_size = 0;
        assert!(config.validate().is_err());

        // Fix and test playlist
        config.buffer_size = 1024;
        config.playlist_start = 10;
        config.playlist_end = Some(5);
        assert!(config.validate().is_err());
    }
}
