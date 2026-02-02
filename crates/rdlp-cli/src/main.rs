//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

use anyhow::Result;
use clap::Parser;
use dialoguer::{Select, theme::ColorfulTheme};
use indicatif::MultiProgress;
use rdlp_cli::Orchestrator;
use rdlp_core::{AudioFormat, BrowserType, Config, ContainerFormat, InfoDict, config_io};
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

    /// Output directory
    #[arg(short, long)]
    output: Option<PathBuf>,

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

    /// Embed thumbnail in the file (requires FFmpeg)
    #[arg(long)]
    embed_thumbnail: bool,

    /// Convert video to specified format
    /// Use --recode-video for interactive, --recode-video=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    recode_video: Option<String>,

    /// Remux to container for better seeking - no re-encoding
    /// Use --remux for interactive, --remux=mp4 for direct
    #[arg(long, num_args = 0..=1, default_missing_value = "interactive", require_equals = true)]
    remux: Option<String>,

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

/// Build Config by merging: defaults < config file < CLI args
fn build_config(args: &Args) -> Result<Config> {
    // Step 1: Load config file (or use defaults)
    let mut config = if args.ignore_config {
        Config::default()
    } else {
        match config_io::load_config(args.config_location.as_deref()) {
            Ok(Some((file_config, path))) => {
                // Can't use tracing yet (not initialized), so use eprintln for early feedback
                // Tracing will be set up after config is built (need quiet/verbose from config)
                eprintln!("Loaded config from {}", path.display());
                file_config
            }
            Ok(None) => Config::default(),
            Err(e) => {
                if args.config_location.is_some() {
                    // Explicit config path — error is fatal
                    return Err(e.into());
                }
                // Default path — warn and continue with defaults
                eprintln!("Warning: Failed to load config file: {e}");
                Config::default()
            }
        }
    };

    // Step 2: Overlay CLI args (only when explicitly provided)
    if let Some(ref output) = args.output {
        config.output_directory = output.clone();
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
    match args.audio_format.as_deref() {
        Some("interactive") => {
            config.audio_format = select_audio_format()?;
        }
        Some(audio_format) => {
            config.audio_format = Some(
                audio_format
                    .parse::<AudioFormat>()
                    .map_err(|e| anyhow::anyhow!(e))?,
            );
        }
        None => {}
    }
    if let Some(ref audio_quality) = args.audio_quality {
        config.audio_quality = Some(audio_quality.clone());
    }
    if args.embed_metadata {
        config.embed_metadata = true;
    }
    if args.embed_thumbnail {
        config.embed_thumbnail = true;
    }
    match args.recode_video.as_deref() {
        Some("interactive") => {
            config.recode_video = select_recode_video()?;
        }
        Some(recode_video) => {
            config.recode_video = Some(
                recode_video
                    .parse::<ContainerFormat>()
                    .map_err(|e| anyhow::anyhow!(e))?,
            );
        }
        None => {}
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

    // Handle interactive remux selection
    match args.remux.as_deref() {
        Some("interactive") => {
            config.remux_container = select_remux_container()?;
        }
        Some(container) => {
            config.remux_container = Some(
                container
                    .parse::<ContainerFormat>()
                    .map_err(|e| anyhow::anyhow!(e))?,
            );
        }
        None => {}
    }

    // Set progress based on quiet
    config.progress = !config.quiet;

    // Set audio_format when extract_audio is set but no explicit format
    if config.extract_audio && config.audio_format.is_none() {
        config.audio_format = Some(AudioFormat::Mp3);
    }

    Ok(config)
}

async fn async_main() -> Result<()> {
    let args = Args::parse();

    // Build config with precedence: CLI > config file > defaults
    let config = build_config(&args)?;

    // Create shared MultiProgress for managing progress bars with log output
    let multi_progress = Arc::new(MultiProgress::new());

    if !config.quiet {
        let default_level = if config.verbose { "debug" } else { "info" };

        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

        // Use SuspendingWriter to properly handle logs while progress bars are active
        // This prevents progress bar duplication caused by log messages
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

    let interactive = args.interactive;

    // Create orchestrator with shared MultiProgress
    let orchestrator = Orchestrator::new(config, (*multi_progress).clone());

    // Load cookies from file or browser if configured
    orchestrator.load_cookies().await?;

    // List extractors if requested
    if args.list_extractors {
        info!("Available extractors:");
        for extractor in orchestrator.list_extractors() {
            info!("  - {extractor}");
        }
        return Ok(());
    }

    if args.list_downloaders {
        info!("Available download protocols:");
        for downloader in orchestrator.list_downloaders() {
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

    // Metadata-only modes: --dump-json, --print, --simulate
    if args.dump_json || args.print.is_some() || args.simulate {
        let infos = orchestrator.extract_info(&url).await?;

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

    match orchestrator.download(&url, interactive).await {
        Ok(Some(path)) => {
            info!("Success! Video saved to: {}", path.display());
            Ok(())
        }
        Ok(None) => {
            // User cancelled - already printed message in orchestrator
            Ok(())
        }
        Err(e) => {
            error!("Error: {e}");
            if args.verbose {
                error!("Debug info: {e:?}");
            }
            std::process::exit(1);
        }
    }
}
