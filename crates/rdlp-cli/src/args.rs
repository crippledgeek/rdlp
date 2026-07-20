//! CLI argument definitions for rdlp.
//!
//! Contains the `Args` struct with clap derive macros for all
//! command-line options.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

// Task-oriented `--help` section headings. clap buckets arguments by exact
// heading-string match, so these live as named consts to prevent a typo
// silently splitting a group.
/// Core download options: output paths, format selection, verbosity.
pub const HELP_HEADING_GENERAL: &str = "General";
/// Simulation and info-listing flags that inspect metadata without downloading.
pub const HELP_HEADING_INFO: &str = "Simulation & Info";
/// Post-download processing: audio extraction, metadata/thumbnail embedding, remux.
pub const HELP_HEADING_POSTPROCESS: &str = "Post-Processing";
/// Subtitle discovery, selection, format, and embedding options.
pub const HELP_HEADING_SUBTITLES: &str = "Subtitles";
/// Video/audio recode target, encoder, and per-codec tuning flags.
pub const HELP_HEADING_RECODE: &str = "Recode & Encoding";
/// Peak and EBU R128 loudnorm audio-level normalization options.
pub const HELP_HEADING_AUDIO_NORM: &str = "Audio Normalization";
/// Proxy, timeouts, browser TLS emulation, and cookie sourcing.
pub const HELP_HEADING_NETWORK: &str = "Network & Cookies";
/// Rate limiting, download archive, and per-video metadata filters.
pub const HELP_HEADING_DOWNLOAD: &str = "Download Behaviour";
/// Keyword search and per-site search filter options.
pub const HELP_HEADING_SEARCH: &str = "Search";
/// Config file loading and plugin trust management.
pub const HELP_HEADING_CONFIG: &str = "Configuration & Plugins";

// Help layout: place the examples (`before_help`) AFTER the about line and
// BEFORE usage — GNU tar / clig.dev "lead with examples" — in both `-h` and
// `--help`. clap's default template has no slot there; this reorders the
// standard placeholders (verified: clap_builder command.rs help_template).
const HELP_TEMPLATE: &str = "\
{about}

{before-help}{usage-heading} {usage}

{all-args}{after-help}";

// Rendered in both `-h` and `--help` (before_help, not before_long_help). Every
// line MUST stay <= 78 chars (guarded by `help_examples_lines_fit_80_columns`)
// so it doesn't hard-wrap on an 80-col terminal — clap does not re-indent a
// wrapped before_help continuation line, which would break the aligned table.
const HELP_EXAMPLES: &str = "\
Examples:
  rdlp URL                                 Download a video (auto-resume)
  rdlp -i URL                              Pick format/quality interactively
  rdlp --cookies-from-browser chrome URL   Use browser cookies (login-gated)
  rdlp --recode-video=mkv URL              Recode video to MKV";

/// Rejects an empty or whitespace-only argument value.
///
/// The shared rule behind [`non_blank`] and [`non_blank_path`]. A blank value
/// otherwise reaches the domain layer looking like a real one: `--recode-audio=`
/// became `RecodeAudioMode::Encoder { name: "" }` and failed inside `FFmpeg`
/// after the download had already completed (#540).
///
/// clap's own [`NonEmptyStringValueParser`] is not enough — it tests
/// `OsStr::is_empty()` only, so `--flag="   "` passes straight through it.
///
/// [`NonEmptyStringValueParser`]: clap::builder::NonEmptyStringValueParser
fn reject_blank(value: &str) -> Result<&str, String> {
    if value.trim().is_empty() {
        Err("value must not be empty or whitespace-only".to_owned())
    } else {
        Ok(value)
    }
}

/// `value_parser` for string-valued arguments.
///
/// A bare `Fn(&str) -> Result<T, E>` is itself a `TypedValueParser` (clap's
/// blanket impl), so this needs no wrapper type. Rejections travel through
/// `Error::value_validation`, which renders the flag name, the offending value
/// and the usage line — context a post-parse check cannot recover.
fn non_blank(value: &str) -> Result<String, String> {
    reject_blank(value).map(ToOwned::to_owned)
}

/// `value_parser` for path-valued arguments.
///
/// Same rule as [`non_blank`], typed for the `PathBuf` args — a blank path is
/// no more meaningful than a blank string.
fn non_blank_path(value: &str) -> Result<PathBuf, String> {
    reject_blank(value).map(PathBuf::from)
}

/// Plugin management subcommands.
#[derive(Subcommand, Debug)]
pub enum PluginCmd {
    /// List installed plugins.
    List,
    /// Show details for a specific plugin.
    Info {
        /// Plugin name.
        #[arg(value_parser = non_blank)]
        name: String,
    },
    /// Accept a new identity for an already-installed plugin (use after the
    /// publisher legitimately rotated their signing key).
    Retrust {
        /// Plugin name.
        #[arg(value_parser = non_blank)]
        name: String,
    },
    /// Disable a plugin for future runs (writes to disabled list).
    Disable {
        /// Plugin name.
        #[arg(value_parser = non_blank)]
        name: String,
    },
    /// Re-enable a previously disabled plugin.
    Enable {
        /// Plugin name.
        #[arg(value_parser = non_blank)]
        name: String,
    },
    /// Remove a plugin entirely (deletes the plugin directory + trust entry).
    Uninstall {
        /// Plugin name.
        #[arg(value_parser = non_blank)]
        name: String,
    },
    /// Build a `.wasm` plugin from a yt-dlp-style Python extractor.
    BuildFromYtdlp {
        /// Path to the yt-dlp extractor .py file.
        #[arg(value_parser = non_blank_path)]
        plugin_py: std::path::PathBuf,
        /// Output directory (defaults to the parent of `plugin_py`).
        #[arg(short = 'o', long, value_parser = non_blank_path)]
        output_dir: Option<std::path::PathBuf>,
    },
}

/// CLI arguments parsed by clap.
// Clap CLI structs naturally accumulate flag fields; refactoring into bitfields
// would obscure the one-to-one correspondence with CLI flags.
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser)]
#[command(name = "rdlp")]
#[command(about = "Rust Download Program - A video downloader", long_about = None)]
#[command(version)]
#[command(help_template = HELP_TEMPLATE)]
#[command(before_help = HELP_EXAMPLES)]
pub struct Args {
    /// Video URL to download
    #[arg(value_parser = non_blank)]
    pub url: Option<String>,

    // Deliberately two paragraphs: para 1 renders in `-h`; the resume-determinism
    // caveat below renders only in `--help` (clap doc-comment split). Do not merge.
    /// Output template or directory (e.g., "%(title)s.%(ext)s" or "./downloads/")
    ///
    /// Note: resume across restarts requires a deterministic name. Templates using
    /// `%(epoch)s` render a different name each run, so an interrupted download cannot
    /// be resumed and restarts from zero. Build the template from stable metadata
    /// (`title`, `id`, `uploader`, `ext`, `upload_date`) for resumable downloads.
    #[arg(short, long, value_parser = non_blank, value_name = "TEMPLATE", help_heading = HELP_HEADING_GENERAL)]
    pub output: Option<String>,

    /// Output directory (always sets base directory, combinable with -o template)
    #[arg(short = 'P', long = "paths", value_parser = non_blank_path, value_name = "DIR", help_heading = HELP_HEADING_GENERAL)]
    pub output_dir: Option<PathBuf>,

    /// Format selection (e.g., "best", "bestvideo+bestaudio")
    // SELECTOR, not FORMAT: `format`.to_uppercase() == "FORMAT" is
    // indistinguishable from clap's inferred echo and would defeat the
    // placeholder canary.
    #[arg(short, long, value_parser = non_blank, value_name = "SELECTOR", help_heading = HELP_HEADING_GENERAL)]
    pub format: Option<String>,

    /// Require strict video-only + audio-only streams for merge.
    /// Changes default from b/bv*+ba to b/bv+ba.
    #[arg(long, help_heading = HELP_HEADING_GENERAL)]
    pub audio_multistreams: bool,

    /// Quiet mode (minimal output)
    #[arg(short, long, help_heading = HELP_HEADING_GENERAL)]
    pub quiet: bool,

    /// Verbose mode (detailed output)
    #[arg(short, long, help_heading = HELP_HEADING_GENERAL)]
    pub verbose: bool,

    /// List all supported extractors
    #[arg(long, help_heading = HELP_HEADING_INFO)]
    pub list_extractors: bool,

    /// List all supported download protocols
    #[arg(long, help_heading = HELP_HEADING_INFO)]
    pub list_downloaders: bool,

    /// List all supported audio and video codecs
    #[arg(long, help_heading = HELP_HEADING_INFO)]
    pub list_codecs: bool,

    /// Simulate (don't actually download, shows extraction summary)
    #[arg(short = 's', long, help_heading = HELP_HEADING_INFO)]
    pub simulate: bool,

    /// Dump full metadata as JSON to stdout (no download)
    #[arg(short = 'j', long, help_heading = HELP_HEADING_INFO)]
    pub dump_json: bool,

    /// List available formats as a table (no download)
    #[arg(short = 'F', long, help_heading = HELP_HEADING_INFO)]
    pub list_formats: bool,

    /// Print specific field(s) from metadata (no download)
    /// e.g., --print title or --print "id,title,extractor"
    #[arg(long, value_parser = non_blank, value_name = "FIELD", help_heading = HELP_HEADING_INFO)]
    pub print: Option<String>,

    /// Interactive format selection
    #[arg(short = 'i', long, help_heading = HELP_HEADING_GENERAL)]
    pub interactive: bool,

    // === Post-processing options ===
    /// Extract audio only (requires `FFmpeg`)
    #[arg(short = 'x', long, help_heading = HELP_HEADING_POSTPROCESS)]
    pub extract_audio: bool,

    /// Audio format for extraction
    /// Use --audio-format for interactive, --audio-format=mp3 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true, value_parser = non_blank, value_name = "FORMAT", help_heading = HELP_HEADING_POSTPROCESS)]
    pub audio_format: Option<String>,

    /// Audio quality (VBR level 0-9 or bitrate like "192K")
    #[arg(long, value_parser = non_blank, value_name = "QUALITY", help_heading = HELP_HEADING_POSTPROCESS)]
    pub audio_quality: Option<String>,

    /// Embed metadata (title, artist, etc.) in the file
    #[arg(long, help_heading = HELP_HEADING_POSTPROCESS)]
    pub embed_metadata: bool,

    /// Disable automatic thumbnail download and embedding
    #[arg(long, help_heading = HELP_HEADING_POSTPROCESS)]
    pub no_thumbnail: bool,

    /// Write thumbnail image to disk alongside media file
    #[arg(long, help_heading = HELP_HEADING_POSTPROCESS)]
    pub write_thumbnail: bool,

    // === Subtitle options ===
    /// Download subtitles
    #[arg(long, alias = "write-subs", help_heading = HELP_HEADING_SUBTITLES)]
    pub write_subtitles: bool,

    /// Download auto-generated subtitles
    #[arg(long, alias = "write-auto-subs", help_heading = HELP_HEADING_SUBTITLES)]
    pub write_auto_subtitles: bool,

    /// Subtitle languages to download (comma-separated, e.g., "en,es")
    /// Use "all" to download all available
    #[arg(long, alias = "sub-langs", value_parser = non_blank, value_name = "LANGS", help_heading = HELP_HEADING_SUBTITLES)]
    pub sub_langs: Option<String>,

    /// Preferred subtitle format (srt, vtt, ass, ssa, lrc)
    #[arg(long, alias = "sub-format", value_parser = non_blank, value_name = "FORMAT", help_heading = HELP_HEADING_SUBTITLES)]
    pub sub_format: Option<String>,

    /// Embed subtitles in video file (requires `FFmpeg`)
    #[arg(long, alias = "embed-subs", help_heading = HELP_HEADING_SUBTITLES)]
    pub embed_subtitles: bool,

    /// Interactive subtitle selection + video download (implies --write-subtitles)
    #[arg(long, alias = "list-subs", help_heading = HELP_HEADING_SUBTITLES)]
    pub list_subs: bool,

    /// Show subtitle menu, download only subtitles (no video), then exit
    #[arg(long, alias = "list-subs-only", help_heading = HELP_HEADING_SUBTITLES)]
    pub list_subs_only: bool,

    /// Strict subtitle mode: fail download if requested subs are missing
    #[arg(long, help_heading = HELP_HEADING_SUBTITLES)]
    pub strict_subs: bool,

    /// Pre-validate subtitle URLs with HEAD requests before download
    #[arg(long, help_heading = HELP_HEADING_SUBTITLES)]
    pub verify_sub_urls: bool,

    /// Retry subtitle downloads for already-downloaded videos missing subs
    #[arg(long, help_heading = HELP_HEADING_SUBTITLES)]
    pub retry_subs: bool,

    /// Video encoder to use (e.g., libsvtav1, libx264).
    /// Overrides automatic encoder selection.
    #[arg(long, value_name = "NAME", value_parser = non_blank, help_heading = HELP_HEADING_RECODE)]
    pub video_encoder: Option<String>,

    /// List available video encoders and exit.
    #[arg(long, help_heading = HELP_HEADING_INFO)]
    pub list_encoders: bool,

    /// Convert video to specified format
    /// Use --recode-video for interactive, --recode-video=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true, value_parser = non_blank, value_name = "FORMAT", help_heading = HELP_HEADING_RECODE)]
    pub recode_video: Option<String>,

    /// Target container format for video recode (e.g., mp4, mkv, webm).
    /// Takes precedence over --recode-video when both are specified.
    #[arg(long, value_name = "FMT", value_parser = non_blank, help_heading = HELP_HEADING_RECODE)]
    pub recode_container: Option<String>,

    /// Audio mode during video recode: copy (default), auto, or an encoder name
    /// (e.g., libopus, aac, libmp3lame).
    /// `copy` copies audio unchanged; `auto` selects the best encoder for the
    /// target container; any other value is treated as an explicit encoder name.
    //
    // Deliberately no clap `default_value`: absent must mean "not specified on
    // the command line" so a `recode_audio` set in the config file survives. A
    // default made that indistinguishable from an explicit `copy`, and the
    // unconditional assignment downstream then discarded the config value on
    // every run (#540). Plain `//` — a `///` here would print this note in
    // `rdlp --help`.
    #[arg(long, value_name = "MODE", value_parser = non_blank, help_heading = HELP_HEADING_RECODE)]
    pub recode_audio: Option<String>,

    // Help text hardcodes the bounds: keep `1-64` in sync with
    // `rdlp_types::config::MAX_RECODE_THREADS` and `8` in sync with
    // `rdlp_ffmpeg`'s `AUTO_RECODE_THREADS_CAP`.
    /// Encoder threads for video recode (1-64). Omit for auto (min(cores, 8)).
    #[arg(long, value_name = "N", help_heading = HELP_HEADING_RECODE)]
    pub recode_threads: Option<u32>,

    /// Encoder preset override for video recode (e.g. `faster`, `medium`, `slow`).
    /// Omit to use the per-codec default. `libvvenc`: try `faster` for speed.
    #[arg(long, value_name = "PRESET", value_parser = non_blank, help_heading = HELP_HEADING_RECODE)]
    pub recode_preset: Option<String>,

    /// libvpx deadline for VP8/VP9 recode: best | good | realtime.
    #[arg(long, value_name = "MODE", value_parser = non_blank, help_heading = HELP_HEADING_RECODE)]
    pub recode_deadline: Option<String>,

    /// libvpx cpu-used for VP8/VP9 recode (VP9: -8..8, VP8: -16..16).
    #[arg(long, value_name = "N", allow_hyphen_values = true, help_heading = HELP_HEADING_RECODE)]
    pub recode_cpu_used: Option<i32>,

    /// libxavs2 `speed_level` for AVS2 recode (0..9).
    #[arg(long, value_name = "N", help_heading = HELP_HEADING_RECODE)]
    pub recode_speed_level: Option<u32>,

    /// Remux to container for better seeking - no re-encoding
    /// Use --remux for interactive, --remux=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true, value_parser = non_blank, value_name = "FORMAT", help_heading = HELP_HEADING_POSTPROCESS)]
    pub remux: Option<String>,

    /// Normalize audio levels (peak mode: volume + limiter)
    #[arg(long, help_heading = HELP_HEADING_AUDIO_NORM)]
    pub normalize_audio: bool,

    /// Use EBU R128 loudnorm normalization (two-pass, implies --normalize-audio)
    #[arg(long, help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm: bool,

    /// Target peak level in dBFS for peak normalization (default: -1.0)
    #[arg(long, allow_hyphen_values = true, value_name = "DBFS", help_heading = HELP_HEADING_AUDIO_NORM)]
    pub audio_gain_target: Option<f64>,

    /// Loudnorm preset: broadcast (-23 LUFS), streaming (-14 LUFS), loud (-11 LUFS)
    #[arg(long, value_parser = non_blank, value_name = "PRESET", help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm_preset: Option<String>,

    /// Target integrated loudness in LUFS for loudnorm (e.g., -14)
    #[arg(long, allow_hyphen_values = true, value_name = "LUFS", help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm_i: Option<f64>,

    /// Target true peak in dBTP for loudnorm (e.g., -1)
    #[arg(long, allow_hyphen_values = true, value_name = "DBTP", help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm_tp: Option<f64>,

    /// Target loudness range in LU for loudnorm (e.g., 11)
    #[arg(long, value_name = "LU", help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm_lra: Option<f64>,

    /// Force dynamic (per-frame compression) mode in loudnorm pass 2
    #[arg(long, help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm_dynamic: bool,

    /// Prepend a mild acompressor before loudnorm to tame extreme peaks
    #[arg(long, help_heading = HELP_HEADING_AUDIO_NORM)]
    pub loudnorm_precompress: bool,

    /// Enable limiter-boost fallback (+12 dB gain + hard limiter) for
    /// over-compressed content (implies --loudnorm)
    #[arg(long, help_heading = HELP_HEADING_AUDIO_NORM)]
    pub normalize_boost: bool,

    /// Gain in dB for limiter-boost fallback (default: 12.0)
    #[arg(long, allow_hyphen_values = true, value_name = "DB", help_heading = HELP_HEADING_AUDIO_NORM)]
    pub normalize_boost_db: Option<f64>,

    /// Fixup policy: never, warn, `detect_or_warn` [default: `detect_or_warn`]
    //
    // Deliberately no clap `default_value`, for the mirror-image reason to
    // `recode_audio` above: a default made an explicit `--fixup=detect_or_warn`
    // indistinguishable from an omitted flag, so the sentinel guard downstream
    // skipped the assignment and a config-file `fixup` wrongly won (#583).
    // clap's `ArgMatches::value_source` answers "was this passed?" directly but
    // is builder-only, so the derive API expresses it as `Option`, with the
    // default supplied by `Config::default()`. Plain `//` — a `///` here would
    // print this note in `rdlp --help`.
    #[arg(long, value_parser = non_blank, value_name = "POLICY", help_heading = HELP_HEADING_POSTPROCESS)]
    pub fixup: Option<String>,

    /// Keep original video file after post-processing
    #[arg(long, help_heading = HELP_HEADING_POSTPROCESS)]
    pub keep_video: bool,

    /// Path to `FFmpeg` executable (if not in PATH)
    #[arg(long, value_parser = non_blank_path, value_name = "PATH", help_heading = HELP_HEADING_POSTPROCESS)]
    pub ffmpeg_location: Option<PathBuf>,

    // === Network options ===
    /// HTTP/HTTPS/SOCKS proxy URL (e.g., <socks5://127.0.0.1:1080>)
    #[arg(long, value_parser = non_blank, value_name = "URL", help_heading = HELP_HEADING_NETWORK)]
    pub proxy: Option<String>,

    /// Connect/handshake timeout in seconds.
    /// Validated post-parse by `Config::validate()`: must be 1..=300.
    #[arg(long, value_name = "SECS", help_heading = HELP_HEADING_NETWORK)]
    pub socket_timeout: Option<u64>,

    /// Per-read idle timeout in seconds.
    /// Validated post-parse by `Config::validate()`: must be 1..=600.
    #[arg(long, value_name = "SECS", help_heading = HELP_HEADING_NETWORK)]
    pub read_timeout: Option<u64>,

    /// Idle keep-alive socket eviction timeout in seconds. `0` is the
    /// documented sentinel meaning "disable eviction (keep idle sockets
    /// forever)".
    /// Validated post-parse by `Config::validate()`: must be 0..=3600.
    #[arg(long, value_name = "SECS", help_heading = HELP_HEADING_NETWORK)]
    pub pool_idle_timeout: Option<u64>,

    /// Total download timeout in seconds (the entire file must complete within
    /// this). Default 3600.
    /// Validated post-parse by `Config::validate()`: must be 1..=86400.
    #[arg(long, value_name = "SECS", help_heading = HELP_HEADING_NETWORK)]
    pub download_timeout: Option<u64>,

    /// Merge (mux/concat) operation timeout in seconds. Default 1800.
    /// Validated post-parse by `Config::validate()`: must be 1..=86400.
    #[arg(long, value_name = "SECS", help_heading = HELP_HEADING_NETWORK)]
    pub merge_timeout: Option<u64>,

    /// Browser emulation profile for the TLS / HTTP stack
    /// (chrome-latest, firefox-latest, safari-latest, or a pinned
    /// identifier like chrome-137). Controls JA4 / JA4H fingerprint.
    /// Falls back to the `RDLP_BROWSER_EMULATION` env var, then
    /// `ChromeLatest`.
    #[arg(long, value_name = "PROFILE", value_parser = non_blank, help_heading = HELP_HEADING_NETWORK)]
    pub browser: Option<String>,

    /// Limit download speed (e.g., "1M", "500K", "10M", "2.5M")
    #[arg(long, short = 'r', value_parser = non_blank, value_name = "RATE", help_heading = HELP_HEADING_DOWNLOAD)]
    pub limit_rate: Option<String>,

    // === Cookie options ===
    /// Load cookies from browser (chrome, firefox)
    #[arg(long, value_parser = non_blank, value_name = "BROWSER", help_heading = HELP_HEADING_NETWORK)]
    pub cookies_from_browser: Option<String>,

    /// Path to Netscape-format cookies file
    #[arg(long, value_parser = non_blank_path, value_name = "FILE", help_heading = HELP_HEADING_NETWORK)]
    pub cookies: Option<PathBuf>,

    /// Path to download archive file (skip already-downloaded videos)
    #[arg(long, value_parser = non_blank_path, value_name = "FILE", help_heading = HELP_HEADING_DOWNLOAD)]
    pub download_archive: Option<PathBuf>,

    /// Filter videos by metadata (yt-dlp syntax). Repeatable (OR logic between filters).
    /// Examples: "duration > 60", "!`is_live`", "title *= cats", "`like_count` >? 100"
    #[arg(long = "match-filter", action = clap::ArgAction::Append, value_parser = non_blank, value_name = "FILTER", help_heading = HELP_HEADING_DOWNLOAD)]
    pub match_filter: Vec<String>,

    // === Search options ===
    /// Perform a keyword search instead of downloading a URL
    #[arg(long, value_parser = non_blank, value_name = "QUERY", help_heading = HELP_HEADING_SEARCH)]
    pub search: Option<String>,

    /// Site to search (required with --search, e.g., "xhamster")
    #[arg(long, value_parser = non_blank, value_name = "SITE", help_heading = HELP_HEADING_SEARCH)]
    pub search_site: Option<String>,

    /// Search filter in key=value format (repeatable)
    #[arg(long = "search-filter", value_parser = non_blank, value_name = "KEY=VALUE", help_heading = HELP_HEADING_SEARCH)]
    pub search_filter: Vec<String>,

    // === Config file options ===
    /// Ignore config file (don't load from default location)
    #[arg(long, help_heading = HELP_HEADING_CONFIG)]
    pub ignore_config: bool,

    /// Path to config file (TOML format)
    #[arg(long, value_parser = non_blank_path, value_name = "FILE", help_heading = HELP_HEADING_CONFIG)]
    pub config_location: Option<PathBuf>,

    // === Plugin options ===
    /// Pre-trust a publisher identity for non-interactive plugin install.
    /// Pass repeatedly for multiple identities.
    /// Format: `sigstore:github:user/repo` or `ed25519:<8-byte-hex>`.
    #[arg(long, global = true, value_parser = non_blank, value_name = "PUBLISHER", help_heading = HELP_HEADING_CONFIG)]
    pub trust_publisher: Vec<String>,

    /// Plugin management subcommand.
    #[command(subcommand)]
    pub plugin: Option<PluginSubcommand>,
}

/// Top-level subcommand wrapper for plugin management.
#[derive(Subcommand, Debug)]
pub enum PluginSubcommand {
    /// Manage installed WASM plugins.
    Plugin(PluginCmdArgs),
}

/// Argument holder for plugin subcommand (required by clap's subcommand+subcommand nesting).
#[derive(clap::Args, Debug)]
pub struct PluginCmdArgs {
    /// Plugin management action.
    #[command(subcommand)]
    pub cmd: PluginCmd,
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod args_tests;
