//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

mod args;
mod commands;
mod config;
mod selection;

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::MultiProgress;
use rdlp_api::TempRegistry;
use rdlp_api::{RdlpApiError, RdlpClient};
use rdlp_cli::event_handler::CliEventHandler;
use rdlp_cli::interactive::DialoguerCallback;
use std::sync::Arc;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use args::Args;
use commands::{fail_with, print_codecs, print_fields};
use config::build_config;

/// Get optimal number of worker threads for I/O-heavy workloads
fn optimal_worker_threads() -> usize {
    // For I/O-bound work (downloads), use 2x CPU cores
    // This allows more concurrent I/O operations
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    (cpu_count * 2).min(32) // Cap at 32 threads
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

fn main() -> Result<()> {
    // Create optimized multi-threaded runtime for I/O-heavy workloads
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(optimal_worker_threads())
        .enable_all()
        .build()
        .context("failed to build Tokio runtime")?;

    runtime.block_on(async_main())
}

async fn async_main() -> Result<()> {
    let args = Args::parse();

    // Build config with precedence: CLI > config file > defaults
    let config = build_config(&args).context("failed to build configuration")?;

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

    // Remove stale temp files left by a prior crash in the output directory.
    TempRegistry::cleanup_stale(&config.output_directory);

    if let Some(rate) = config.rate_limit {
        debug!("Rate limit: {rate} bytes/s");
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

    // Create a shared TempRegistry for this process lifetime.
    // cleanup_all() is called at the end of async_main() to remove any temp
    // files created during the session. cleanup_stale() (called above) handles
    // orphans from prior crashes.
    let temp_registry = Arc::new(TempRegistry::new());

    // Create RdlpClient with interactive callback, sharing the registry
    // so all pipeline instances register their temp files in the same registry.
    let client = RdlpClient::builder()
        .config(config)
        .interactive(Arc::new(DialoguerCallback))
        .temp_registry(Arc::clone(&temp_registry))
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // List extractors if requested
    if args.list_extractors {
        debug!("Available extractors:");
        for extractor in client.list_extractors() {
            debug!("  - {extractor}");
        }
        return Ok(());
    }

    if args.list_downloaders {
        debug!("Available download protocols:");
        for downloader in client.list_downloaders() {
            debug!("  - {downloader}");
        }
        return Ok(());
    }

    if args.list_codecs {
        print_codecs();
        return Ok(());
    }

    if args.list_encoders {
        rdlp_ffmpeg::ffmpeg::ensure_init()
            .map_err(|e| anyhow::anyhow!("FFmpeg init failed: {e}"))?;
        for codec in rdlp_ffmpeg::ffmpeg::video_codecs::list_available_codecs() {
            eprintln!("{}:", codec.display_name);
            for enc in &codec.encoders {
                eprintln!("  {} — {}", enc.encoder_name, enc.display_name);
            }
        }
        return Ok(());
    }

    // === Search mode ===
    if let Some(ref query_text) = args.search {
        let site = args.search_site.as_deref().unwrap_or_else(|| {
            let sites = client.list_search_sites();
            if sites.len() == 1 {
                // Leak is fine for a single CLI run
                return Box::leak(sites[0].name.clone().into_boxed_str());
            }
            eprintln!(
                "Error: --search-site is required. Available sites: {}",
                sites
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            std::process::exit(1);
        });

        // Parse filters from key=value strings
        let mut filters = Vec::new();
        for raw in &args.search_filter {
            let (key, value) = match raw.split_once('=') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => {
                    eprintln!("Error: Invalid filter format '{raw}'. Expected key=value.");
                    std::process::exit(1);
                }
            };
            filters.push(rdlp_api::SearchFilter { key, value });
        }

        let search_query = rdlp_api::SearchQuery {
            query: query_text.clone(),
            filters,
            max_results: None,
            page: Some(1),
        };

        match client.search_page(site, &search_query).await {
            Ok(response) => {
                if response.results.is_empty() {
                    eprintln!("No results found for '{query_text}'.");
                } else {
                    let page_info = if response.has_more {
                        format!(" (page {}, more available)", response.page)
                    } else {
                        format!(" (page {})", response.page)
                    };
                    eprintln!("Found {} results{}:\n", response.results.len(), page_info);
                    for (i, r) in response.results.iter().enumerate() {
                        eprintln!("{:>3}. {}", i + 1, r.title);
                        eprintln!("     {}", r.video_url);
                        if let Some(d) = r.duration {
                            let mins = d as u32 / 60;
                            let secs = d as u32 % 60;
                            eprint!("     Duration: {mins}:{secs:02}");
                        }
                        if let Some(views) = r.view_count {
                            eprint!("  Views: {views}");
                        }
                        eprintln!();
                    }
                }
            }
            Err(e) => fail_with(e, verbose),
        }

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
                        debug!("No subtitles downloaded");
                    } else {
                        for path in &paths {
                            info!("Subtitle saved: {}", path.display());
                        }
                    }
                }
                Ok(None) => {
                    debug!("Subtitle selection cancelled");
                }
                Err(e) => fail_with(e, verbose),
            }
        }

        return Ok(());
    }

    // Metadata-only modes: --dump-json, --print, --list-formats, --simulate
    if args.dump_json || args.print.is_some() || args.list_formats || args.simulate {
        let infos = match client.extract_info(&url).await {
            Ok(infos) => infos,
            Err(e) => fail_with(e, verbose),
        };

        if args.dump_json {
            for info in &infos {
                let json = serde_json::to_string_pretty(info)
                    .context("failed to serialize metadata to JSON")?;
                println!("{json}");
            }
        }

        if args.list_formats {
            for info in &infos {
                let refs: Vec<&rdlp_api::Format> = info.formats.iter().collect();
                let table = rdlp_table::render_formats_table(
                    &refs,
                    &rdlp_table::TableOpts::default(),
                );
                println!("{}", info.title);
                println!("{table}");
            }
        }

        if let Some(ref fields) = args.print {
            for info in &infos {
                print_fields(info, fields).context("failed to print metadata fields")?;
            }
        }

        if args.simulate && !args.dump_json && args.print.is_none() && !args.list_formats {
            for info in &infos {
                debug!(
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

    // Local file path: skip extraction/download, run post-processing only
    let local_path = std::path::Path::new(&url);
    if !url.contains("://") && local_path.exists() {
        info!("Processing local file: {}", local_path.display());
        let mut handle = client.process_local_file(local_path.to_path_buf());
        let mut event_handler = CliEventHandler::new(Arc::clone(&multi_progress), quiet);

        while let Some(event) = handle.events().recv().await {
            event_handler.handle_event(&event);
        }

        return match handle.wait().await {
            Ok(result) => {
                if let Some(path) = result.output_files.first() {
                    info!("Success! Processed file: {}", path.display());
                }
                Ok(())
            }
            Err(e) => fail_with(e, verbose),
        };
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
    let result = match handle.wait().await {
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
    };

    // Clean up any temp files created during this session (e.g. aborted
    // mid-pipeline). SIGKILL orphans are handled by cleanup_stale() at next startup.
    temp_registry.cleanup_all();

    result
}
