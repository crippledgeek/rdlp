//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

use anyhow::Result;
use clap::Parser;
use dialoguer::{Select, theme::ColorfulTheme};
use indicatif::MultiProgress;
use rdlp_cli::Orchestrator;
use rdlp_core::Config;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};
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
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    /// Format selection (e.g., "best", "bestvideo+bestaudio")
    #[arg(short, long, default_value = "best")]
    format: String,

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

    /// Simulate (don't actually download)
    #[arg(short = 's', long)]
    simulate: bool,

    /// Interactive format selection
    #[arg(short = 'i', long)]
    interactive: bool,

    // === Post-processing options ===
    /// Extract audio only (requires FFmpeg)
    #[arg(short = 'x', long)]
    extract_audio: bool,

    /// Audio format for extraction (mp3, m4a, opus, flac, wav)
    #[arg(long, default_value = "mp3")]
    audio_format: String,

    /// Audio quality (VBR level 0-9 or bitrate like "192K")
    #[arg(long)]
    audio_quality: Option<String>,

    /// Embed metadata (title, artist, etc.) in the file
    #[arg(long)]
    embed_metadata: bool,

    /// Embed thumbnail in the file (requires FFmpeg)
    #[arg(long)]
    embed_thumbnail: bool,

    /// Convert video to specified format (mp4, mkv, webm)
    #[arg(long)]
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
fn select_remux_container() -> Result<Option<String>> {
    let containers = [
        ("mp4", "Best compatibility, faststart for streaming"),
        ("mkv", "Supports all codecs, efficient cues index"),
        ("webm", "Web-optimized, VP8/VP9/AV1 + Opus/Vorbis"),
        ("mov", "Apple QuickTime, good for editing"),
        ("avi", "Legacy format, wide support"),
        ("ts", "MPEG-TS, broadcast/streaming"),
    ];

    let items: Vec<String> = containers
        .iter()
        .map(|(name, desc)| format!("{name:<6} {desc}"))
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select remux container (ESC to cancel)")
        .items(&items)
        .default(0)
        .interact_opt()?;

    Ok(selection.map(|idx| containers[idx].0.to_string()))
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

async fn async_main() -> Result<()> {
    let args = Args::parse();

    // Create shared MultiProgress for managing progress bars with log output
    let multi_progress = Arc::new(MultiProgress::new());

    if !args.quiet {
        let default_level = if args.verbose { "debug" } else { "info" };

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(default_level));

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

    // Handle interactive remux selection
    let remux_container = match args.remux.as_deref() {
        Some("interactive") => select_remux_container()?,
        Some(container) => Some(container.to_string()),
        None => None,
    };

    // Create configuration
    let config = Config {
        output_directory: args.output,
        format: args.format,
        quiet: args.quiet,
        verbose: args.verbose,
        simulate: args.simulate,
        progress: !args.quiet,
        // Post-processing options
        extract_audio: args.extract_audio,
        audio_format: if args.extract_audio {
            Some(args.audio_format)
        } else {
            None
        },
        audio_quality: args.audio_quality,
        embed_metadata: args.embed_metadata,
        embed_thumbnail: args.embed_thumbnail,
        recode_video: args.recode_video,
        remux_container,
        keep_video: args.keep_video,
        ffmpeg_location: args.ffmpeg_location,
        ..Default::default()
    };

    let interactive = args.interactive;

    // Create orchestrator with shared MultiProgress
    let orchestrator = Orchestrator::new(config, (*multi_progress).clone());

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

    // Check if URL is provided
    let url = args
        .url
        .ok_or_else(|| anyhow::anyhow!("No URL provided. Use --help for usage information."))?;

    // Download the video
    if args.simulate {
        info!("[Simulate] No actual download will occur");
        info!("URL: {url}");
        return Ok(());
    }

    match orchestrator.download(&url, interactive).await {
        Ok(Some(path)) => {
            if !args.quiet {
                info!("Success! Video saved to: {}", path.display());
            }
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
