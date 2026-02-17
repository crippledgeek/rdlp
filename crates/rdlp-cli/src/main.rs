//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

use anyhow::Result;
use clap::Parser;
use dialoguer::{Select, theme::ColorfulTheme};
use indicatif::MultiProgress;
use rdlp_api::{RdlpApiError, RdlpClient};
use rdlp_cli::event_handler::CliEventHandler;
use rdlp_cli::interactive::DialoguerCallback;
use rdlp_core::{
    AudioFormat, BrowserType, Config, ContainerFormat, InfoDict, SubtitleFormat, config_io,
};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Get optimal number of worker threads for I/O-heavy workloads
fn optimal_worker_threads() -> usize {
    // For I/O-bound work (downloads), use 2x CPU cores
    // This allows more concurrent I/O operations
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    (cpu_count * 2).min(32) // Cap at 32 threads
}

#[derive(Parser)]
#[command(name = "rdlp")]
#[command(about = "Rust Download Program - A video downloader", long_about = None)]
#[command(version)]
struct Args {
    /// Video URL to download
    url: Option<String>,

    /// Output template or directory (e.g., "%(title)s.%(ext)s" or "./downloads/")
    #[arg(short, long)]
    output: Option<String>,

    /// Output directory (always sets base directory, combinable with -o template)
    #[arg(short = 'P', long = "paths")]
    output_dir: Option<PathBuf>,

    /// Format selection (e.g., "best", "bestvideo+bestaudio")
    #[arg(short, long)]
    format: Option<String>,

    /// Quiet mode (minimal output)
    #[arg(short, long)]
    quiet: bool,

    /// Verbose mode (detailed output)
    #[arg(short, long)]
    verbose: bool,

    /// List all supported extractors
    #[arg(long)]
    list_extractors: bool,

    /// List all supported download protocols
    #[arg(long)]
    list_downloaders: bool,

    /// List all supported audio and video codecs
    #[arg(long)]
    list_codecs: bool,

    /// Simulate (don't actually download, shows extraction summary)
    #[arg(short = 's', long)]
    simulate: bool,

    /// Dump full metadata as JSON to stdout (no download)
    #[arg(short = 'j', long)]
    dump_json: bool,

    /// Print specific field(s) from metadata (no download)
    /// e.g., --print title or --print "id,title,extractor"
    #[arg(long)]
    print: Option<String>,

    /// Interactive format selection
    #[arg(short = 'i', long)]
    interactive: bool,

    // === Post-processing options ===
    /// Extract audio only (requires FFmpeg)
    #[arg(short = 'x', long)]
    extract_audio: bool,

    /// Audio format for extraction
    /// Use --audio-format for interactive, --audio-format=mp3 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    audio_format: Option<String>,

    /// Audio quality (VBR level 0-9 or bitrate like "192K")
    #[arg(long)]
    audio_quality: Option<String>,

    /// Embed metadata (title, artist, etc.) in the file
    #[arg(long)]
    embed_metadata: bool,

    /// Disable automatic thumbnail download and embedding
    #[arg(long)]
    no_thumbnail: bool,

    /// Write thumbnail image to disk alongside media file
    #[arg(long)]
    write_thumbnail: bool,

    // === Subtitle options ===
    /// Download subtitles
    #[arg(long, alias = "write-subs")]
    write_subtitles: bool,

    /// Download auto-generated subtitles
    #[arg(long, alias = "write-auto-subs")]
    write_auto_subtitles: bool,

    /// Subtitle languages to download (comma-separated, e.g., "en,es")
    /// Use "all" to download all available
    #[arg(long, alias = "sub-langs")]
    sub_langs: Option<String>,

    /// Preferred subtitle format (srt, vtt, ass, ssa, lrc)
    #[arg(long, alias = "sub-format")]
    sub_format: Option<String>,

    /// Embed subtitles in video file (requires FFmpeg)
    #[arg(long, alias = "embed-subs")]
    embed_subtitles: bool,

    /// Interactive subtitle selection + video download (implies --write-subtitles)
    #[arg(long, alias = "list-subs")]
    list_subs: bool,

    /// Show subtitle menu, download only subtitles (no video), then exit
    #[arg(long, alias = "list-subs-only")]
    list_subs_only: bool,

    /// Strict subtitle mode: fail download if requested subs are missing
    #[arg(long)]
    strict_subs: bool,

    /// Pre-validate subtitle URLs with HEAD requests before download
    #[arg(long)]
    verify_sub_urls: bool,

    /// Retry subtitle downloads for already-downloaded videos missing subs
    #[arg(long)]
    retry_subs: bool,

    /// Convert video to specified format
    /// Use --recode-video for interactive, --recode-video=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    recode_video: Option<String>,

    /// Remux to container for better seeking - no re-encoding
    /// Use --remux for interactive, --remux=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    remux: Option<String>,

    /// Normalize audio levels (peak mode: volume + limiter)
    #[arg(long)]
    normalize_audio: bool,

    /// Use EBU R128 loudnorm normalization (two-pass, implies --normalize-audio)
    #[arg(long)]
    loudnorm: bool,

    /// Target peak level in dBFS for peak normalization (default: -1.0)
    #[arg(long, allow_hyphen_values = true)]
    audio_gain_target: Option<f64>,

    /// Loudnorm preset: broadcast (-23 LUFS), streaming (-14 LUFS), loud (-11 LUFS)
    #[arg(long)]
    loudnorm_preset: Option<String>,

    /// Target integrated loudness in LUFS for loudnorm (e.g., -14)
    #[arg(long, allow_hyphen_values = true)]
    loudnorm_i: Option<f64>,

    /// Target true peak in dBTP for loudnorm (e.g., -1)
    #[arg(long, allow_hyphen_values = true)]
    loudnorm_tp: Option<f64>,

    /// Target loudness range in LU for loudnorm (e.g., 11)
    #[arg(long)]
    loudnorm_lra: Option<f64>,

    /// Force dynamic (per-frame compression) mode in loudnorm pass 2
    #[arg(long)]
    loudnorm_dynamic: bool,

    /// Prepend a mild acompressor before loudnorm to tame extreme peaks
    #[arg(long)]
    loudnorm_precompress: bool,

    /// Enable limiter-boost fallback (+12 dB gain + hard limiter) for
    /// over-compressed content (implies --loudnorm)
    #[arg(long)]
    normalize_boost: bool,

    /// Gain in dB for limiter-boost fallback (default: 12.0)
    #[arg(long, allow_hyphen_values = true)]
    normalize_boost_db: Option<f64>,

    /// Keep original video file after post-processing
    #[arg(long)]
    keep_video: bool,

    /// Path to FFmpeg executable (if not in PATH)
    #[arg(long)]
    ffmpeg_location: Option<PathBuf>,

    // === Network options ===
    /// HTTP/HTTPS/SOCKS proxy URL (e.g., socks5://127.0.0.1:1080)
    #[arg(long)]
    proxy: Option<String>,

    /// Limit download speed (e.g., "1M", "500K", "10M", "2.5M")
    #[arg(long, short = 'r')]
    limit_rate: Option<String>,

    // === Cookie options ===
    /// Load cookies from browser (chrome, firefox)
    #[arg(long)]
    cookies_from_browser: Option<String>,

    /// Path to Netscape-format cookies file
    #[arg(long)]
    cookies: Option<PathBuf>,

    /// Path to download archive file (skip already-downloaded videos)
    #[arg(long)]
    download_archive: Option<PathBuf>,

    // === Config file options ===
    /// Ignore config file (don't load from default location)
    #[arg(long)]
    ignore_config: bool,

    /// Path to config file (TOML format)
    #[arg(long)]
    config_location: Option<PathBuf>,
}

fn main() -> Result<()> {
    // Create optimized multi-threaded runtime for I/O-heavy workloads
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(optimal_worker_threads())
        .enable_all()
        .build()?;

    runtime.block_on(async_main())
}

/// Interactive remux container selection
fn select_remux_container() -> Result<Option<ContainerFormat>> {
    let containers = [
        // Video containers
        (
            ContainerFormat::Mp4,
            "Best compatibility, faststart for streaming",
        ),
        (
            ContainerFormat::Mkv,
            "Supports all codecs, efficient cues index",
        ),
        (
            ContainerFormat::WebM,
            "Web-optimized, VP8/VP9/AV1 + Opus/Vorbis",
        ),
        (ContainerFormat::Mov, "Apple QuickTime, good for editing"),
        (ContainerFormat::Avi, "Legacy format, wide support"),
        (ContainerFormat::Ts, "MPEG-TS, broadcast/streaming"),
        (ContainerFormat::Flv, "Flash Video, legacy"),
        (ContainerFormat::ThreeGp, "3GPP mobile video"),
        (ContainerFormat::Mpg, "MPEG-1/2 program stream"),
        (ContainerFormat::F4v, "Flash Video (MP4 variant)"),
        (ContainerFormat::Asf, "Windows Media / ASF"),
        (
            ContainerFormat::Mxf,
            "Material eXchange, broadcast/professional",
        ),
        (ContainerFormat::Vob, "DVD Video Object"),
        (ContainerFormat::Dv, "Digital Video"),
        (ContainerFormat::Nut, "NUT (FFmpeg native container)"),
        (ContainerFormat::Ivf, "On2 IVF (VP8/VP9/AV1 raw)"),
        // Audio containers
        (ContainerFormat::Mp3, "Audio only, MPEG Layer 3"),
        (ContainerFormat::Flac, "Audio only, lossless"),
        (ContainerFormat::Wav, "Audio only, PCM waveform"),
        (ContainerFormat::Ogg, "Audio only, Ogg container"),
        (ContainerFormat::M4a, "Audio only, MPEG-4 Audio"),
        (ContainerFormat::Opus, "Audio only, Ogg Opus"),
        (ContainerFormat::Aac, "Audio only, raw ADTS AAC"),
        (ContainerFormat::Aiff, "Audio only, Apple AIFF"),
        (ContainerFormat::Mka, "Audio only, Matroska Audio"),
        (ContainerFormat::Wv, "Audio only, WavPack lossless"),
        (ContainerFormat::Caf, "Audio only, Core Audio Format"),
        (ContainerFormat::Ac3, "Audio only, Dolby AC-3"),
    ];

    let items: Vec<String> = containers
        .iter()
        .map(|(fmt, desc)| format!("{:<6} {desc}", fmt.as_ext()))
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select remux container (ESC to cancel)")
        .items(&items)
        .default(0)
        .interact_opt()?;

    Ok(selection.map(|idx| containers[idx].0))
}

/// Interactive audio format selection
fn select_audio_format() -> Result<Option<AudioFormat>> {
    let formats = [
        (AudioFormat::Mp3, "MPEG Layer 3, most compatible"),
        (AudioFormat::Aac, "Advanced Audio Coding"),
        (AudioFormat::M4a, "AAC in M4A container"),
        (AudioFormat::Opus, "Opus codec, excellent quality/size"),
        (AudioFormat::Vorbis, "Ogg Vorbis"),
        (AudioFormat::Flac, "Free Lossless Audio Codec"),
        (AudioFormat::Alac, "Apple Lossless"),
        (AudioFormat::Wav, "PCM waveform, uncompressed"),
        (AudioFormat::Ac3, "Dolby Digital"),
        (AudioFormat::Eac3, "Dolby Digital Plus"),
        (AudioFormat::Dts, "DTS Coherent Acoustics"),
        (AudioFormat::Mp2, "MPEG Layer 2"),
        (AudioFormat::WavPack, "WavPack lossless"),
        (AudioFormat::Tta, "True Audio lossless"),
    ];

    let items: Vec<String> = formats
        .iter()
        .map(|(fmt, desc)| format!("{:<8} {desc}", fmt.as_ext()))
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select audio format (ESC to cancel)")
        .items(&items)
        .default(0)
        .interact_opt()?;

    Ok(selection.map(|idx| formats[idx].0))
}

/// Interactive video recode format selection
fn select_recode_video() -> Result<Option<ContainerFormat>> {
    let formats = [
        (ContainerFormat::Mp4, "h264", "Best compatibility, H.264"),
        (ContainerFormat::Mkv, "h264", "Matroska, H.264"),
        (ContainerFormat::WebM, "vp9", "Web-optimized, VP9"),
        (ContainerFormat::Mov, "h264", "Apple QuickTime, H.264"),
        (ContainerFormat::Avi, "h264", "Legacy AVI, H.264"),
        (ContainerFormat::Mpg, "mpeg2", "MPEG program stream, MPEG-2"),
        (ContainerFormat::Ts, "h264", "MPEG-TS, H.264"),
        (ContainerFormat::ThreeGp, "h264", "3GPP mobile, H.264"),
        (ContainerFormat::Flv, "h264", "Flash Video, H.264"),
        (ContainerFormat::Asf, "wmv2", "Windows Media, WMV2"),
    ];

    let items: Vec<String> = formats
        .iter()
        .map(|(fmt, codec, desc)| format!("{:<6} [{codec}] {desc}", fmt.as_ext()))
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select video format (ESC to cancel)")
        .items(&items)
        .default(0)
        .interact_opt()?;

    Ok(selection.map(|idx| formats[idx].0))
}

/// Print all supported codecs
fn print_codecs() {
    println!("Audio codecs (14):");
    let audio_codecs = [
        ("mp3", "libmp3lame", "MPEG Layer 3"),
        ("aac", "aac", "Advanced Audio Coding"),
        ("m4a", "aac", "AAC in M4A container"),
        ("opus", "libopus", "Opus codec"),
        ("vorbis", "libvorbis", "Ogg Vorbis"),
        ("flac", "flac", "Free Lossless Audio Codec"),
        ("alac", "alac", "Apple Lossless"),
        ("wav", "pcm_s16le", "PCM waveform"),
        ("ac3", "ac3", "Dolby Digital"),
        ("eac3", "eac3", "Dolby Digital Plus"),
        ("dts", "dca", "DTS Coherent Acoustics"),
        ("mp2", "mp2", "MPEG Layer 2"),
        ("wavpack", "wavpack", "WavPack lossless"),
        ("tta", "tta", "True Audio lossless"),
    ];
    for (name, encoder, desc) in audio_codecs {
        println!("  {name:<10} [{encoder}]  {desc}");
    }

    println!();
    println!("Video codecs (16):");
    let video_codecs = [
        ("h264", "libx264", "H.264 / AVC"),
        ("h265", "libx265", "H.265 / HEVC"),
        ("vp9", "libvpx-vp9", "VP9"),
        ("vp8", "libvpx", "VP8"),
        ("av1", "libaom-av1", "AV1"),
        ("vvc", "libvvenc", "H.266 / VVC"),
        ("mpeg1", "mpeg1video", "MPEG-1 Video"),
        ("mpeg2", "mpeg2video", "MPEG-2 Video"),
        ("mpeg4", "mpeg4", "MPEG-4 Part 2"),
        ("theora", "libtheora", "Theora"),
        ("prores", "prores_ks", "Apple ProRes"),
        ("dnxhd", "dnxhd", "Avid DNxHD"),
        ("wmv2", "wmv2", "Windows Media Video 8"),
        ("ffv1", "ffv1", "FFV1 lossless archival"),
        ("xvid", "libxvid", "Xvid (MPEG-4 ASP)"),
    ];
    for (name, encoder, desc) in video_codecs {
        println!("  {name:<10} [{encoder}]  {desc}");
    }
}

/// Print specific fields from an InfoDict
fn print_fields(info: &InfoDict, fields: &str) -> Result<()> {
    let value = serde_json::to_value(info)?;
    let map = value.as_object().expect("InfoDict serializes to object");

    for field in fields.split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        match map.get(field) {
            Some(serde_json::Value::String(s)) => println!("{field}: {s}"),
            Some(serde_json::Value::Null) => println!("{field}:"),
            Some(v) => println!("{field}: {v}"),
            None => {
                warn!("Unknown field: {field}");
                eprintln!("Warning: unknown field '{field}'");
            }
        }
    }
    Ok(())
}

/// Writer that suspends progress bars while writing to prevent visual duplication
#[derive(Clone)]
struct SuspendingWriter {
    multi_progress: Arc<MultiProgress>,
}

impl std::io::Write for SuspendingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.multi_progress.suspend(|| std::io::stderr().write(buf))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SuspendingWriter {
    type Writer = SuspendingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Resolved values from interactive CLI prompts.
///
/// These are resolved BEFORE the pure config merge so that `merge_config()`
/// remains free of side effects and is testable.
struct ResolvedInteractiveValues {
    audio_format: Option<AudioFormat>,
    recode_video: Option<ContainerFormat>,
    remux_container: Option<ContainerFormat>,
}

/// Resolve interactive CLI values (dialoguer prompts) before config merge.
///
/// Returns `None` values for fields that weren't set to "interactive".
fn resolve_interactive_values(args: &Args) -> Result<ResolvedInteractiveValues> {
    let audio_format = match args.audio_format.as_deref() {
        Some("interactive") => select_audio_format()?,
        _ => None,
    };

    let recode_video = match args.recode_video.as_deref() {
        Some("interactive") => select_recode_video()?,
        _ => None,
    };

    let remux_container = match args.remux.as_deref() {
        Some("interactive") => select_remux_container()?,
        _ => None,
    };

    Ok(ResolvedInteractiveValues {
        audio_format,
        recode_video,
        remux_container,
    })
}

/// Pure config merge: defaults < config file < CLI args.
///
/// This function has no side effects (no interactive prompts, no I/O).
/// Interactive values must be pre-resolved via `resolve_interactive_values()`.
fn merge_config(
    args: &Args,
    file_config: Config,
    interactive_values: ResolvedInteractiveValues,
) -> Result<Config> {
    let mut config = file_config;

    // Output: -o always sets output_template (the template engine handles
    // both template fields like "%(title)s" and plain filenames like "video.mp4")
    if let Some(ref output) = args.output {
        config.output_template = output.clone();
    }
    if let Some(ref dir) = args.output_dir {
        config.output_directory = dir.clone();
    }
    if let Some(ref format) = args.format {
        config.format = format.clone();
    }
    if args.quiet {
        config.quiet = true;
    }
    if args.verbose {
        config.verbose = true;
    }
    if args.simulate {
        config.simulate = true;
    }
    if args.extract_audio {
        config.extract_audio = true;
    }

    // Audio format: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.audio_format {
        config.audio_format = Some(fmt);
    } else if let Some(audio_format) = args.audio_format.as_deref() {
        if audio_format != "interactive" {
            config.audio_format = Some(
                audio_format
                    .parse::<AudioFormat>()
                    .map_err(|e| anyhow::anyhow!(e))?,
            );
        }
    }

    if let Some(ref audio_quality) = args.audio_quality {
        config.audio_quality = Some(audio_quality.clone());
    }
    if args.embed_metadata {
        config.embed_metadata = true;
    }
    if args.no_thumbnail {
        config.embed_thumbnail = false;
    }
    if args.write_thumbnail {
        config.write_thumbnail = true;
    }

    // Subtitles
    if args.write_subtitles {
        config.write_subtitles = true;
    }
    if args.write_auto_subtitles {
        config.write_auto_subtitles = true;
    }
    if let Some(ref langs) = args.sub_langs {
        config.subtitle_langs = langs.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Some(ref format) = args.sub_format {
        config.subtitle_format = Some(
            format
                .parse::<SubtitleFormat>()
                .map_err(|e| anyhow::anyhow!(e))?,
        );
    }
    if args.embed_subtitles {
        config.embed_subtitles = true;
    }
    // --list-subs implies --write-subtitles
    if args.list_subs || args.list_subs_only {
        config.write_subtitles = true;
        config.list_subs = true;
    }
    // --embed-subtitles implies --write-subtitles for the download step
    if config.embed_subtitles && !config.write_subtitles {
        config.write_subtitles = true;
    }
    if args.strict_subs {
        config.strict_subs = true;
    }
    if args.verify_sub_urls {
        config.verify_sub_urls = true;
    }
    if args.retry_subs {
        config.retry_subs = true;
    }

    // Recode video: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.recode_video {
        config.recode_video = Some(fmt);
    } else if let Some(recode_video) = args.recode_video.as_deref() {
        if recode_video != "interactive" {
            config.recode_video = Some(
                recode_video
                    .parse::<ContainerFormat>()
                    .map_err(|e| anyhow::anyhow!(e))?,
            );
        }
    }

    // Audio normalization: --normalize-boost / --normalize-boost-db implies --loudnorm
    // implies --normalize-audio
    let boost_enabled = args.normalize_boost || args.normalize_boost_db.is_some();
    if args.normalize_audio || args.loudnorm || boost_enabled {
        config.normalize_audio = true;
    }
    if args.loudnorm || boost_enabled {
        config.loudnorm = true;
    }
    if boost_enabled {
        config.normalize_boost = true;
    }
    if let Some(db) = args.normalize_boost_db {
        config.normalize_boost_db = Some(db);
    }
    if let Some(target) = args.audio_gain_target {
        config.audio_gain_target = Some(target);
    }
    if let Some(ref preset) = args.loudnorm_preset {
        config.loudnorm_preset = Some(preset.clone());
    }
    if let Some(i) = args.loudnorm_i {
        config.loudnorm_target_i = Some(i);
    }
    if let Some(tp) = args.loudnorm_tp {
        config.loudnorm_target_tp = Some(tp);
    }
    if let Some(lra) = args.loudnorm_lra {
        config.loudnorm_target_lra = Some(lra);
    }
    if args.loudnorm_dynamic {
        config.loudnorm_dynamic = true;
    }
    if args.loudnorm_precompress {
        config.loudnorm_precompress = true;
    }

    if args.keep_video {
        config.keep_video = true;
    }
    if let Some(ref ffmpeg_location) = args.ffmpeg_location {
        config.ffmpeg_location = Some(ffmpeg_location.clone());
    }
    if let Some(ref proxy) = args.proxy {
        config.proxy = Some(proxy.clone());
    }
    if let Some(ref rate_str) = args.limit_rate {
        let bps = rdlp_ratelimit::parse_rate_limit(rate_str).map_err(|e| anyhow::anyhow!(e))?;
        config.rate_limit = Some(bps);
    }
    if let Some(ref browser) = args.cookies_from_browser {
        config.cookies_from_browser = Some(
            browser
                .parse::<BrowserType>()
                .map_err(|e| anyhow::anyhow!(e))?,
        );
    }
    if let Some(ref cookies) = args.cookies {
        config.cookies_file = Some(cookies.clone());
    }
    if let Some(ref archive) = args.download_archive {
        config.download_archive = Some(archive.clone());
    }

    // Remux: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.remux_container {
        config.remux_container = Some(fmt);
    } else if let Some(container) = args.remux.as_deref() {
        if container != "interactive" {
            config.remux_container = Some(
                container
                    .parse::<ContainerFormat>()
                    .map_err(|e| anyhow::anyhow!(e))?,
            );
        }
    }

    // Derived settings
    config.progress = !config.quiet;

    if config.extract_audio && config.audio_format.is_none() {
        config.audio_format = Some(AudioFormat::Mp3);
    }

    // Validate final config
    config
        .validate()
        .map_err(|e| anyhow::anyhow!("Invalid configuration: {e}"))?;

    Ok(config)
}

/// Build Config by: resolve interactive prompts -> load file -> merge.
fn build_config(args: &Args) -> Result<Config> {
    // Step 1: Resolve interactive values (side effects isolated here)
    let interactive_values = resolve_interactive_values(args)?;

    // Step 2: Load config file (or use defaults)
    let file_config = if args.ignore_config {
        Config::default()
    } else {
        match config_io::load_config(args.config_location.as_deref()) {
            Ok(Some((file_config, path))) => {
                eprintln!("Loaded config from {}", path.display());
                file_config
            }
            Ok(None) => Config::default(),
            Err(e) => {
                if args.config_location.is_some() {
                    return Err(e.into());
                }
                eprintln!("Warning: Failed to load config file: {e}");
                Config::default()
            }
        }
    };

    // Step 3: Pure merge (no side effects, testable)
    merge_config(args, file_config, interactive_values)
}

/// Map RdlpApiError to a structured process exit code.
///
/// Exit codes:
///   0 — success (handled by Ok paths)
///   1 — general/unknown error (I/O, processing, platform)
///   2 — user cancelled (Ctrl+C, ESC)
///   3 — extraction failed (unsupported URL, extraction error)
///   4 — download/network failed (network error)
///   5 — configuration/format error (invalid input, builder error)
fn exit_code_for(e: &RdlpApiError) -> i32 {
    match e {
        RdlpApiError::UserCancelled => 2,
        RdlpApiError::UnsupportedUrl { .. } | RdlpApiError::ExtractError { .. } => 3,
        RdlpApiError::NetworkError { .. } => 4,
        RdlpApiError::InvalidInput { .. } | RdlpApiError::BuilderError { .. } => 5,
        RdlpApiError::IoError { .. }
        | RdlpApiError::FfmpegError { .. }
        | RdlpApiError::UnsupportedPlatform { .. }
        | RdlpApiError::Soft { .. } => 1,
    }
}

/// Log an RdlpApiError and exit with the appropriate structured code.
fn fail_with(e: RdlpApiError, verbose: bool) -> ! {
    error!("Error: {e}");
    if verbose {
        error!("Debug info: {e:?}");
    }
    std::process::exit(exit_code_for(&e))
}

async fn async_main() -> Result<()> {
    let args = Args::parse();

    // Build config with precedence: CLI > config file > defaults
    let config = build_config(&args)?;

    // Create shared MultiProgress for managing progress bars with log output
    let multi_progress = Arc::new(MultiProgress::new());

    if !config.quiet {
        let filter = if config.verbose {
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug"))
        } else {
            EnvFilter::new("info")
        };

        let writer = SuspendingWriter {
            multi_progress: Arc::clone(&multi_progress),
        };

        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_writer(writer))
            .init();
    }

    if let Some(rate) = config.rate_limit {
        info!("Rate limit: {rate} bytes/s");
    }

    if config.verify_sub_urls && !config.strict_subs {
        warn!(
            "--verify-sub-urls validates URLs but missing subs \
             won't fail the download without --strict-subs"
        );
    }

    let interactive = args.interactive;
    let verbose = args.verbose;
    let quiet = config.quiet;

    // Create RdlpClient with interactive callback
    let client = RdlpClient::builder()
        .config(config)
        .interactive(Arc::new(DialoguerCallback))
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // List extractors if requested
    if args.list_extractors {
        info!("Available extractors:");
        for extractor in client.list_extractors() {
            info!("  - {extractor}");
        }
        return Ok(());
    }

    if args.list_downloaders {
        info!("Available download protocols:");
        for downloader in client.list_downloaders() {
            info!("  - {downloader}");
        }
        return Ok(());
    }

    if args.list_codecs {
        print_codecs();
        return Ok(());
    }

    // Check if URL is provided
    let url = args
        .url
        .ok_or_else(|| anyhow::anyhow!("No URL provided. Use --help for usage information."))?;

    // --list-subs-only: show subtitle menu, download subs, exit (no video)
    if args.list_subs_only {
        let infos = match client.extract_info(&url).await {
            Ok(infos) => infos,
            Err(e) => fail_with(e, verbose),
        };

        // Use the first video's metadata for subtitle selection
        if let Some(info) = infos.first() {
            match client.download_subtitles_only(info).await {
                Ok(Some(paths)) => {
                    if paths.is_empty() {
                        info!("No subtitles downloaded");
                    } else {
                        for path in &paths {
                            info!("Subtitle saved: {}", path.display());
                        }
                    }
                }
                Ok(None) => {
                    info!("Subtitle selection cancelled");
                }
                Err(e) => fail_with(e, verbose),
            }
        }

        return Ok(());
    }

    // Metadata-only modes: --dump-json, --print, --simulate
    if args.dump_json || args.print.is_some() || args.simulate {
        let infos = match client.extract_info(&url).await {
            Ok(infos) => infos,
            Err(e) => fail_with(e, verbose),
        };

        if args.dump_json {
            for info in &infos {
                let json = serde_json::to_string_pretty(info)?;
                println!("{json}");
            }
        }

        if let Some(ref fields) = args.print {
            for info in &infos {
                print_fields(info, fields)?;
            }
        }

        if args.simulate && !args.dump_json && args.print.is_none() {
            for info in &infos {
                info!(
                    "[Simulate] {} | id={} | extractor={} | {} format(s)",
                    info.title,
                    info.id,
                    info.extractor,
                    info.formats.len()
                );
            }
        }

        return Ok(());
    }

    // Build download request from config
    let request = rdlp_api::DownloadRequest {
        url: url.clone(),
        format: rdlp_api::request::FormatOptions {
            interactive,
            ..Default::default()
        },
        ..Default::default()
    };

    // Start the download
    let mut handle = client.download(request);
    let mut event_handler = CliEventHandler::new(Arc::clone(&multi_progress), quiet);

    // Drain all events (progress, status, etc.)
    while let Some(event) = handle.events().recv().await {
        event_handler.handle_event(&event);
    }

    // Wait for the final result
    match handle.wait().await {
        Ok(result) => {
            if let Some(path) = result.output_files.first() {
                info!("Success! Video saved to: {}", path.display());
            }
            Ok(())
        }
        Err(RdlpApiError::UserCancelled) => {
            // User cancelled - already printed message via events
            Ok(())
        }
        Err(e) => fail_with(e, verbose),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create default Args for testing (all fields at defaults).
    fn default_args() -> Args {
        Args {
            url: None,
            output: None,
            output_dir: None,
            format: None,
            quiet: false,
            verbose: false,
            list_extractors: false,
            list_downloaders: false,
            list_codecs: false,
            simulate: false,
            dump_json: false,
            print: None,
            interactive: false,
            extract_audio: false,
            audio_format: None,
            audio_quality: None,
            embed_metadata: false,
            no_thumbnail: false,
            write_thumbnail: false,
            write_subtitles: false,
            write_auto_subtitles: false,
            sub_langs: None,
            sub_format: None,
            embed_subtitles: false,
            list_subs: false,
            list_subs_only: false,
            strict_subs: false,
            verify_sub_urls: false,
            retry_subs: false,
            recode_video: None,
            remux: None,
            normalize_audio: false,
            loudnorm: false,
            audio_gain_target: None,
            loudnorm_preset: None,
            loudnorm_i: None,
            loudnorm_tp: None,
            loudnorm_lra: None,
            loudnorm_dynamic: false,
            loudnorm_precompress: false,
            normalize_boost: false,
            normalize_boost_db: None,
            keep_video: false,
            ffmpeg_location: None,
            proxy: None,
            limit_rate: None,
            cookies_from_browser: None,
            cookies: None,
            download_archive: None,
            ignore_config: false,
            config_location: None,
        }
    }

    /// Helper: no-op interactive values (nothing interactive selected).
    fn no_interactive() -> ResolvedInteractiveValues {
        ResolvedInteractiveValues {
            audio_format: None,
            recode_video: None,
            remux_container: None,
        }
    }

    // === Subtitle config merge tests ===

    #[test]
    fn test_merge_config_write_subtitles() {
        let mut args = default_args();
        args.write_subtitles = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.write_subtitles);
        assert!(!config.write_auto_subtitles);
        assert!(!config.embed_subtitles);
    }

    #[test]
    fn test_merge_config_write_auto_subtitles() {
        let mut args = default_args();
        args.write_auto_subtitles = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.write_auto_subtitles);
    }

    #[test]
    fn test_merge_config_sub_langs_parsing() {
        let mut args = default_args();
        args.sub_langs = Some("en, es , fr".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(
            config.subtitle_langs,
            vec!["en".to_string(), "es".to_string(), "fr".to_string()]
        );
    }

    #[test]
    fn test_merge_config_sub_langs_single() {
        let mut args = default_args();
        args.sub_langs = Some("en".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.subtitle_langs, vec!["en".to_string()]);
    }

    #[test]
    fn test_merge_config_sub_langs_all() {
        let mut args = default_args();
        args.sub_langs = Some("all".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.subtitle_langs, vec!["all".to_string()]);
    }

    #[test]
    fn test_merge_config_sub_format_parsing() {
        let mut args = default_args();
        args.sub_format = Some("srt".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.subtitle_format, Some(SubtitleFormat::Srt));
    }

    #[test]
    fn test_merge_config_sub_format_vtt() {
        let mut args = default_args();
        args.sub_format = Some("vtt".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.subtitle_format, Some(SubtitleFormat::Vtt));
    }

    #[test]
    fn test_merge_config_sub_format_ass() {
        let mut args = default_args();
        args.sub_format = Some("ass".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.subtitle_format, Some(SubtitleFormat::Ass));
    }

    #[test]
    fn test_merge_config_sub_format_invalid() {
        let mut args = default_args();
        args.sub_format = Some("invalid_format".to_string());

        let result = merge_config(&args, Config::default(), no_interactive());
        assert!(result.is_err(), "Invalid subtitle format should fail");
    }

    #[test]
    fn test_merge_config_embed_implies_write() {
        let mut args = default_args();
        args.embed_subtitles = true;
        // write_subtitles is NOT explicitly set

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.embed_subtitles);
        assert!(
            config.write_subtitles,
            "--embed-subtitles should imply --write-subtitles"
        );
    }

    #[test]
    fn test_merge_config_embed_with_write_already_set() {
        let mut args = default_args();
        args.embed_subtitles = true;
        args.write_subtitles = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.embed_subtitles);
        assert!(config.write_subtitles);
    }

    #[test]
    fn test_merge_config_subtitle_defaults_are_off() {
        let args = default_args();

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(!config.write_subtitles);
        assert!(!config.write_auto_subtitles);
        assert!(!config.embed_subtitles);
        assert!(config.subtitle_langs.is_empty());
        assert!(config.subtitle_format.is_none());
    }

    #[test]
    fn test_merge_config_subtitle_combined_options() {
        let mut args = default_args();
        args.write_subtitles = true;
        args.write_auto_subtitles = true;
        args.sub_langs = Some("en,es".to_string());
        args.sub_format = Some("vtt".to_string());
        args.embed_subtitles = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.write_subtitles);
        assert!(config.write_auto_subtitles);
        assert!(config.embed_subtitles);
        assert_eq!(
            config.subtitle_langs,
            vec!["en".to_string(), "es".to_string()]
        );
        assert_eq!(config.subtitle_format, Some(SubtitleFormat::Vtt));
    }

    #[test]
    fn test_merge_config_list_subs() {
        let mut args = default_args();
        args.list_subs = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(
            config.write_subtitles,
            "--list-subs should imply --write-subtitles"
        );
        assert!(config.list_subs, "--list-subs should set list_subs");
    }

    #[test]
    fn test_merge_config_list_subs_only() {
        let mut args = default_args();
        args.list_subs_only = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(
            config.write_subtitles,
            "--list-subs-only should imply --write-subtitles"
        );
        assert!(config.list_subs, "--list-subs-only should set list_subs");
    }

    // === Existing config merge tests for non-subtitle features ===

    #[test]
    fn test_merge_config_defaults() {
        let args = default_args();
        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.output_template, "%(title)s.%(ext)s");
        assert!(!config.quiet);
        assert!(!config.verbose);
    }

    #[test]
    fn test_merge_config_output_template() {
        let mut args = default_args();
        args.output = Some("%(id)s.%(ext)s".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.output_template, "%(id)s.%(ext)s");
    }

    #[test]
    fn test_merge_config_extract_audio() {
        let mut args = default_args();
        args.extract_audio = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.extract_audio);
        // extract_audio without format defaults to mp3
        assert_eq!(config.audio_format, Some(AudioFormat::Mp3));
    }

    #[test]
    fn test_merge_config_audio_format_direct() {
        let mut args = default_args();
        args.audio_format = Some("flac".to_string());

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert_eq!(config.audio_format, Some(AudioFormat::Flac));
    }

    #[test]
    fn test_merge_config_normalize_boost_implies_loudnorm() {
        let mut args = default_args();
        args.normalize_boost = true;

        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

        assert!(config.normalize_boost);
        assert!(config.loudnorm, "normalize_boost should imply loudnorm");
        assert!(
            config.normalize_audio,
            "normalize_boost should imply normalize_audio"
        );
    }
}
