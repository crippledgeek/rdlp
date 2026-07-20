//! Application configuration types

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use crate::browser_emulation::BrowserEmulation;
use crate::browser_type::BrowserType;
use crate::postprocess::PostProcess;
use crate::subtitle_format::SubtitleFormat;

/// Hard ceiling for an explicit [`PostProcess::recode_threads`] value.
///
/// Mirrors `concurrent_fragments`' 64 cap: bounds peak encoder memory, which
/// scales with thread count (each frame thread allocates reconstruction
/// buffers ≈ one uncompressed frame). The *auto* default is far lower; this is
/// only the ceiling for explicit overrides.
pub const MAX_RECODE_THREADS: u32 = 64;

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
    /// A configuration field value is outside its allowed range.
    OutOfRange {
        /// The field name (e.g. `"plugin_timeout_extract_s"`)
        field: &'static str,
        /// Human-readable explanation of the allowed range
        reason: &'static str,
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
            Self::OutOfRange { field, reason } => {
                write!(f, "{field}: {reason}")
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
#[allow(clippy::struct_excessive_bools)] // Config structs legitimately carry many boolean feature flags
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
    /// Number of concurrent fragments to download.
    ///
    /// Default `8`: power-of-two, well below H2 `SETTINGS_MAX_CONCURRENT_STREAMS`
    /// (RFC default 100; Cloudflare advertises 100), conservative enough for
    /// H1.1 fallback. Validated 1..=64.
    ///
    /// Memory note: under the parallel pre-resolved-fragments path, peak
    /// transient memory ≈ `concurrent_fragments × max_fragment_size`. With the
    /// default 8 and typical 2–5 MiB segments this is ~16–40 MiB. The 64 cap
    /// keeps the worst case bounded under operator-tunable configurations.
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

    /// Connect-axis timeout in seconds (TCP + TLS handshake).
    ///
    /// Default: 30. Range validation enforces 1..=300 if set.
    ///
    /// Named `socket_timeout` for historical compatibility with the
    /// pre-1.0 single-knob model; the actual semantics are connect-only.
    /// Read and pool-idle timeouts are configured separately via
    /// `read_timeout` and `pool_idle_timeout`.
    pub socket_timeout: Option<u64>,

    /// Read-axis timeout in seconds (per-read inactivity, not total).
    ///
    /// Default: 60. Range: 1..=600.
    #[serde(default)]
    pub read_timeout: Option<u64>,

    /// Pool idle-connection timeout in seconds.
    ///
    /// Default: 90. Range: 0..=3600.
    ///
    /// `0` is a sentinel meaning "disable idle eviction entirely" — wired
    /// through to wreq/reqwest as `pool_idle_timeout(None)`. Any positive
    /// value caps how long an idle connection is kept in the pool.
    ///
    /// Connection count is still capped by `pool_max_idle_per_host`
    /// (default 10), so disabling eviction does not permit unbounded
    /// pool growth.
    #[serde(default)]
    pub pool_idle_timeout: Option<u64>,

    /// Total download timeout in seconds — the entire download of one
    /// file/format must complete within this. Default: 3600 (1 hour).
    /// Range: 1..=86400. Unset keeps the downloader default.
    #[serde(default)]
    pub download_timeout: Option<u64>,

    /// Merge (mux/concat) operation timeout in seconds — the chunk/segment
    /// merge must complete within this. Default: 1800 (30 min).
    /// Range: 1..=86400. Unset keeps the downloader default.
    #[serde(default)]
    pub merge_timeout: Option<u64>,

    /// Wall-clock cap on a single HEAD probe used to detect content-length on
    /// non-HLS formats. The operation is a single HEAD request with a
    /// Range-GET fallback.
    /// Validated post-load by `Config::validate()`: must be 1..=300 seconds.
    pub hls_head_probe_timeout: Option<u64>,

    /// Minimum file size in bytes at which the HTTP downloader switches from
    /// sequential to parallel chunked download. Below this, parallel fan-out
    /// overhead (HEAD probes, chunk-merge step) outweighs the throughput gain.
    /// `None` falls back to the downloader's default
    /// (`DEFAULT_PARALLEL_THRESHOLD_BYTES`, currently 10 MiB).
    /// Validated post-load by `Config::validate()`: must be `1..=1_073_741_824` bytes (1 GiB).
    pub parallel_threshold: Option<u64>,

    /// Source IP address to bind to
    pub source_address: Option<String>,

    /// User agent string
    pub user_agent: Option<String>,

    /// Browser emulation profile for the TLS / HTTP stack.
    ///
    /// Drives the JA4 / JA4H fingerprint presented to servers. Defaults
    /// to `BrowserEmulation::ChromeLatest`.
    #[serde(default)]
    pub browser_emulation: BrowserEmulation,

    /// Custom HTTP headers
    pub http_headers: Vec<(String, String)>,

    // === Post-processing ===
    /// Post-processing configuration (remux, recode, audio, metadata, etc.).
    #[serde(default)]
    pub postprocess: PostProcess,

    // === Subtitles ===
    /// Write automatic captions
    pub write_auto_subtitles: bool,

    /// Subtitle languages to download (e.g., \["en", "es"\])
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
    /// Evaluated against `InfoDict` before download — non-matching videos are skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_filters: Vec<String>,

    // === Plugin system ===
    /// Directories to search for plugins
    pub plugin_directories: Vec<PathBuf>,

    /// List of enabled plugins (None = all plugins enabled)
    pub enabled_plugins: Option<Vec<String>>,

    /// Load dynamic plugins
    pub load_plugins: bool,

    /// Plugin metadata-call timeout in milliseconds (default 100).
    #[serde(default)]
    pub plugin_timeout_metadata_ms: Option<u32>,

    /// Plugin extract-call timeout in seconds (default 30).
    #[serde(default)]
    pub plugin_timeout_extract_s: Option<u32>,

    /// Plugin search-call timeout in seconds (default 60).
    #[serde(default)]
    pub plugin_timeout_search_s: Option<u32>,

    /// Plugin instance memory cap in MB (default 64).
    #[serde(default)]
    pub plugin_memory_limit_mb: Option<u32>,

    /// Plugin instance stack cap in MB (default 1).
    #[serde(default)]
    pub plugin_stack_limit_mb: Option<u32>,

    /// Pre-trusted publisher identities for non-interactive plugin install.
    /// Identity strings are e.g. `sigstore:github:user/repo` or `ed25519:<8-byte-hex>`.
    #[serde(default)]
    pub plugin_trusted_publishers: Vec<String>,

    // === Download performance ===
    /// Enable adaptive chunk sizing and connection tuning for downloads.
    /// When true, the downloader adjusts chunk sizes and parallel connections
    /// based on observed throughput using an AIMD algorithm.
    #[serde(default = "default_true")]
    pub adaptive_downloads: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Output options
            output_to_stdout: false,
            // yt-dlp parity. The pipe-default + ID suffix protects
            // against extractors that produce empty/null titles
            // (e.g. godresource API returning `title: null`) — without
            // the fallback, the filename is `.mp4` (sanitised to `mp4`)
            // and downloads from the same extractor collide on stem.
            // The bracketed `[id]` suffix is always present so even
            // anonymous URLs land at distinguishable paths.
            output_template: "%(title|Unknown)s [%(id)s].%(ext)s".to_string(),
            output_directory: PathBuf::from("."),
            restrict_filenames: false,
            overwrite: false,
            continue_downloads: true,
            no_part: false,

            // Format selection
            format: None,
            audio_multistreams: false,

            // Download options
            concurrent_fragments: 8,
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
            read_timeout: None,
            pool_idle_timeout: None,
            download_timeout: None,
            merge_timeout: None,
            hls_head_probe_timeout: Some(5),
            parallel_threshold: Some(10 * 1024 * 1024),
            source_address: None,
            user_agent: Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".to_string(),
            ),
            browser_emulation: BrowserEmulation::default(),
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
            plugin_timeout_metadata_ms: None,
            plugin_timeout_extract_s: None,
            plugin_timeout_search_s: None,
            plugin_memory_limit_mb: None,
            plugin_stack_limit_mb: None,
            plugin_trusted_publishers: Vec::new(),

            // Download performance
            adaptive_downloads: true,
        }
    }
}

impl Config {
    /// Validate configuration and return errors if invalid.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigValidationError`] if any field is out of range or
    /// inconsistent (e.g. `concurrent_fragments == 0`, `buffer_size == 0`,
    /// invalid `playlist_start`).
    #[allow(clippy::too_many_lines)] // Linear sequence of independent range checks; splitting harms readability.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.concurrent_fragments == 0 {
            return Err(ConfigValidationError::InvalidConcurrentFragments);
        }
        if self.concurrent_fragments > 64 {
            return Err(ConfigValidationError::OutOfRange {
                field: "concurrent_fragments",
                reason: "must be 1..=64 (caps peak transient memory under parallel fragment fetch)",
            });
        }
        if let Some(threads) = self.postprocess.recode_threads
            && !(1..=MAX_RECODE_THREADS).contains(&threads)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "recode_threads",
                reason: "must be 1..=64 (bounds peak encoder RAM; 0 is invalid — \
                         leave unset for auto-detect)",
            });
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

        // Plugin timeout / resource range checks
        if let Some(t) = self.plugin_timeout_extract_s
            && t > 600
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "plugin_timeout_extract_s",
                reason: "must be <= 600 (10 min ceiling)",
            });
        }
        if let Some(t) = self.plugin_timeout_search_s
            && t > 600
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "plugin_timeout_search_s",
                reason: "must be <= 600",
            });
        }
        if let Some(t) = self.plugin_timeout_metadata_ms
            && t > 60_000
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "plugin_timeout_metadata_ms",
                reason: "must be <= 60000ms (60 sec ceiling)",
            });
        }
        if let Some(m) = self.plugin_memory_limit_mb
            && !(1..=1024).contains(&m)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "plugin_memory_limit_mb",
                reason: "must be 1..=1024 MB",
            });
        }
        if let Some(s) = self.plugin_stack_limit_mb
            && !(1..=64).contains(&s)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "plugin_stack_limit_mb",
                reason: "must be 1..=64 MB",
            });
        }

        // HTTP timeout range checks
        if let Some(t) = self.socket_timeout
            && !(1..=300).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "socket_timeout",
                reason: "must be 1..=300 seconds",
            });
        }
        if let Some(t) = self.read_timeout
            && !(1..=600).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "read_timeout",
                reason: "must be 1..=600 seconds",
            });
        }
        if let Some(t) = self.pool_idle_timeout
            && t > 3600
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "pool_idle_timeout",
                reason: "must be 0..=3600 seconds (0 = disabled)",
            });
        }
        if let Some(t) = self.download_timeout
            && !(1..=86400).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "download_timeout",
                reason: "must be 1..=86400 seconds",
            });
        }
        if let Some(t) = self.merge_timeout
            && !(1..=86400).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "merge_timeout",
                reason: "must be 1..=86400 seconds",
            });
        }
        if let Some(t) = self.hls_head_probe_timeout
            && !(1..=300).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "hls_head_probe_timeout",
                reason: "must be 1..=300 seconds",
            });
        }
        if let Some(t) = self.parallel_threshold
            && !(1..=1024 * 1024 * 1024).contains(&t)
        {
            return Err(ConfigValidationError::OutOfRange {
                field: "parallel_threshold",
                reason: "must be 1..=1_073_741_824 bytes (1 GiB)",
            });
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
