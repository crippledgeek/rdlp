//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

use anyhow::Result;
use clap::Parser;
use rdlp_cli::Orchestrator;
use rdlp_core::Config;
use std::path::PathBuf;

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

async fn async_main() -> Result<()> {
    let args = Args::parse();

    // Set up tracing
    if !args.quiet {
        tracing_subscriber::fmt()
            .with_env_filter(if args.verbose {
                "rdlp=debug"
            } else {
                "rdlp=info"
            })
            .init();
    }

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
        audio_format: if args.extract_audio { Some(args.audio_format) } else { None },
        audio_quality: args.audio_quality,
        embed_metadata: args.embed_metadata,
        embed_thumbnail: args.embed_thumbnail,
        recode_video: args.recode_video,
        keep_video: args.keep_video,
        ffmpeg_location: args.ffmpeg_location,
        ..Default::default()
    };

    let interactive = args.interactive;

    // Create orchestrator
    let orchestrator = Orchestrator::new(config);

    // List extractors if requested
    if args.list_extractors {
        println!("Available extractors:");
        for extractor in orchestrator.list_extractors() {
            println!("  - {extractor}");
        }
        return Ok(());
    }

    if args.list_downloaders {
        println!("Available download protocols:");
        for downloader in orchestrator.list_downloaders() {
            println!("  - {downloader}");
        }
        return Ok(());
    }

    // Check if URL is provided
    let url = args.url.ok_or_else(|| {
        anyhow::anyhow!("No URL provided. Use --help for usage information.")
    })?;

    // Download the video
    if args.simulate {
        println!("🔍 Simulating download (no actual download will occur)");
        println!("URL: {url}");
        return Ok(());
    }

    match orchestrator.download(&url, interactive).await {
        Ok(Some(path)) => {
            if !args.quiet {
                println!("\n🎉 Success! Video saved to: {}", path.display());
            }
            Ok(())
        }
        Ok(None) => {
            // User cancelled - already printed message in orchestrator
            Ok(())
        }
        Err(e) => {
            eprintln!("\n❌ Error: {e}");
            if args.verbose {
                eprintln!("\nDebug info:");
                eprintln!("{e:?}");
            }
            std::process::exit(1);
        }
    }
}
