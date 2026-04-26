//! CLI argument definitions for rdlp.
//!
//! Contains the `Args` struct with clap derive macros for all
//! command-line options.

use clap::Parser;
use std::path::PathBuf;

/// CLI arguments parsed by clap.
#[derive(Parser)]
#[command(name = "rdlp")]
#[command(about = "Rust Download Program - A video downloader", long_about = None)]
#[command(version)]
pub(crate) struct Args {
    /// Video URL to download
    pub url: Option<String>,

    /// Output template or directory (e.g., "%(title)s.%(ext)s" or "./downloads/")
    #[arg(short, long)]
    pub output: Option<String>,

    /// Output directory (always sets base directory, combinable with -o template)
    #[arg(short = 'P', long = "paths")]
    pub output_dir: Option<PathBuf>,

    /// Format selection (e.g., "best", "bestvideo+bestaudio")
    #[arg(short, long)]
    pub format: Option<String>,

    /// Require strict video-only + audio-only streams for merge.
    /// Changes default from b/bv*+ba to b/bv+ba.
    #[arg(long)]
    pub audio_multistreams: bool,

    /// Quiet mode (minimal output)
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode (detailed output)
    #[arg(short, long)]
    pub verbose: bool,

    /// List all supported extractors
    #[arg(long)]
    pub list_extractors: bool,

    /// List all supported download protocols
    #[arg(long)]
    pub list_downloaders: bool,

    /// List all supported audio and video codecs
    #[arg(long)]
    pub list_codecs: bool,

    /// Simulate (don't actually download, shows extraction summary)
    #[arg(short = 's', long)]
    pub simulate: bool,

    /// Dump full metadata as JSON to stdout (no download)
    #[arg(short = 'j', long)]
    pub dump_json: bool,

    /// List available formats as a table (no download)
    #[arg(short = 'F', long)]
    pub list_formats: bool,

    /// Print specific field(s) from metadata (no download)
    /// e.g., --print title or --print "id,title,extractor"
    #[arg(long)]
    pub print: Option<String>,

    /// Interactive format selection
    #[arg(short = 'i', long)]
    pub interactive: bool,

    // === Post-processing options ===
    /// Extract audio only (requires FFmpeg)
    #[arg(short = 'x', long)]
    pub extract_audio: bool,

    /// Audio format for extraction
    /// Use --audio-format for interactive, --audio-format=mp3 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    pub audio_format: Option<String>,

    /// Audio quality (VBR level 0-9 or bitrate like "192K")
    #[arg(long)]
    pub audio_quality: Option<String>,

    /// Embed metadata (title, artist, etc.) in the file
    #[arg(long)]
    pub embed_metadata: bool,

    /// Disable automatic thumbnail download and embedding
    #[arg(long)]
    pub no_thumbnail: bool,

    /// Write thumbnail image to disk alongside media file
    #[arg(long)]
    pub write_thumbnail: bool,

    // === Subtitle options ===
    /// Download subtitles
    #[arg(long, alias = "write-subs")]
    pub write_subtitles: bool,

    /// Download auto-generated subtitles
    #[arg(long, alias = "write-auto-subs")]
    pub write_auto_subtitles: bool,

    /// Subtitle languages to download (comma-separated, e.g., "en,es")
    /// Use "all" to download all available
    #[arg(long, alias = "sub-langs")]
    pub sub_langs: Option<String>,

    /// Preferred subtitle format (srt, vtt, ass, ssa, lrc)
    #[arg(long, alias = "sub-format")]
    pub sub_format: Option<String>,

    /// Embed subtitles in video file (requires FFmpeg)
    #[arg(long, alias = "embed-subs")]
    pub embed_subtitles: bool,

    /// Interactive subtitle selection + video download (implies --write-subtitles)
    #[arg(long, alias = "list-subs")]
    pub list_subs: bool,

    /// Show subtitle menu, download only subtitles (no video), then exit
    #[arg(long, alias = "list-subs-only")]
    pub list_subs_only: bool,

    /// Strict subtitle mode: fail download if requested subs are missing
    #[arg(long)]
    pub strict_subs: bool,

    /// Pre-validate subtitle URLs with HEAD requests before download
    #[arg(long)]
    pub verify_sub_urls: bool,

    /// Retry subtitle downloads for already-downloaded videos missing subs
    #[arg(long)]
    pub retry_subs: bool,

    /// Video encoder to use (e.g., libsvtav1, libx264).
    /// Overrides automatic encoder selection.
    #[arg(long, value_name = "NAME")]
    pub video_encoder: Option<String>,

    /// List available video encoders and exit.
    #[arg(long)]
    pub list_encoders: bool,

    /// Convert video to specified format
    /// Use --recode-video for interactive, --recode-video=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    pub recode_video: Option<String>,

    /// Target container format for video recode (e.g., mp4, mkv, webm).
    /// Takes precedence over --recode-video when both are specified.
    #[arg(long, value_name = "FMT")]
    pub recode_container: Option<String>,

    /// Audio mode during video recode: copy (default), auto, or an encoder name
    /// (e.g., libopus, aac, libmp3lame).
    /// `copy` copies audio unchanged; `auto` selects the best encoder for the
    /// target container; any other value is treated as an explicit encoder name.
    #[arg(long, value_name = "MODE", default_value = "copy")]
    pub recode_audio: String,

    /// Remux to container for better seeking - no re-encoding
    /// Use --remux for interactive, --remux=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    pub remux: Option<String>,

    /// Normalize audio levels (peak mode: volume + limiter)
    #[arg(long)]
    pub normalize_audio: bool,

    /// Use EBU R128 loudnorm normalization (two-pass, implies --normalize-audio)
    #[arg(long)]
    pub loudnorm: bool,

    /// Target peak level in dBFS for peak normalization (default: -1.0)
    #[arg(long, allow_hyphen_values = true)]
    pub audio_gain_target: Option<f64>,

    /// Loudnorm preset: broadcast (-23 LUFS), streaming (-14 LUFS), loud (-11 LUFS)
    #[arg(long)]
    pub loudnorm_preset: Option<String>,

    /// Target integrated loudness in LUFS for loudnorm (e.g., -14)
    #[arg(long, allow_hyphen_values = true)]
    pub loudnorm_i: Option<f64>,

    /// Target true peak in dBTP for loudnorm (e.g., -1)
    #[arg(long, allow_hyphen_values = true)]
    pub loudnorm_tp: Option<f64>,

    /// Target loudness range in LU for loudnorm (e.g., 11)
    #[arg(long)]
    pub loudnorm_lra: Option<f64>,

    /// Force dynamic (per-frame compression) mode in loudnorm pass 2
    #[arg(long)]
    pub loudnorm_dynamic: bool,

    /// Prepend a mild acompressor before loudnorm to tame extreme peaks
    #[arg(long)]
    pub loudnorm_precompress: bool,

    /// Enable limiter-boost fallback (+12 dB gain + hard limiter) for
    /// over-compressed content (implies --loudnorm)
    #[arg(long)]
    pub normalize_boost: bool,

    /// Gain in dB for limiter-boost fallback (default: 12.0)
    #[arg(long, allow_hyphen_values = true)]
    pub normalize_boost_db: Option<f64>,

    /// Fixup policy: never, warn, detect_or_warn (default: detect_or_warn)
    #[arg(long, default_value = "detect_or_warn")]
    pub fixup: String,

    /// Keep original video file after post-processing
    #[arg(long)]
    pub keep_video: bool,

    /// Path to FFmpeg executable (if not in PATH)
    #[arg(long)]
    pub ffmpeg_location: Option<PathBuf>,

    // === Network options ===
    /// HTTP/HTTPS/SOCKS proxy URL (e.g., socks5://127.0.0.1:1080)
    #[arg(long)]
    pub proxy: Option<String>,

    /// Browser emulation profile for the TLS / HTTP stack
    /// (chrome-latest, firefox-latest, safari-latest, or a pinned
    /// identifier like chrome-137). Controls JA4 / JA4H fingerprint.
    /// Falls back to the RDLP_BROWSER_EMULATION env var, then
    /// ChromeLatest.
    #[arg(long, value_name = "PROFILE")]
    pub browser: Option<String>,

    /// Limit download speed (e.g., "1M", "500K", "10M", "2.5M")
    #[arg(long, short = 'r')]
    pub limit_rate: Option<String>,

    // === Cookie options ===
    /// Load cookies from browser (chrome, firefox)
    #[arg(long)]
    pub cookies_from_browser: Option<String>,

    /// Path to Netscape-format cookies file
    #[arg(long)]
    pub cookies: Option<PathBuf>,

    /// Path to download archive file (skip already-downloaded videos)
    #[arg(long)]
    pub download_archive: Option<PathBuf>,

    /// Filter videos by metadata (yt-dlp syntax). Repeatable (OR logic between filters).
    /// Examples: "duration > 60", "!is_live", "title *= cats", "like_count >? 100"
    #[arg(long = "match-filter", action = clap::ArgAction::Append)]
    pub match_filter: Vec<String>,

    // === Search options ===
    /// Perform a keyword search instead of downloading a URL
    #[arg(long)]
    pub search: Option<String>,

    /// Site to search (required with --search, e.g., "xhamster")
    #[arg(long)]
    pub search_site: Option<String>,

    /// Search filter in key=value format (repeatable)
    #[arg(long = "search-filter")]
    pub search_filter: Vec<String>,

    // === Config file options ===
    /// Ignore config file (don't load from default location)
    #[arg(long)]
    pub ignore_config: bool,

    /// Path to config file (TOML format)
    #[arg(long)]
    pub config_location: Option<PathBuf>,
}
