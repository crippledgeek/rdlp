//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

mod orchestrator;

use anyhow::Result;
use clap::Parser;
use rdlp_core::Config;
use std::path::PathBuf;

use orchestrator::Orchestrator;

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

    /// Simulate (don't actually download)
    #[arg(short = 's', long)]
    simulate: bool,

    /// Interactive format selection
    #[arg(short = 'i', long)]
    interactive: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
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
