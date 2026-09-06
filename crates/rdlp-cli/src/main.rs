// Lint-tightening for the binary entrypoint. `pedantic` / `nursery` are
// stylistic; `indexing_slicing` prevents silent out-of-bounds panics.
// See `Cargo.toml` `[lints.clippy]` for crate-level baseline.
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]
//! # rdlp CLI
//!
//! Command-line interface for rdlp (Rust Download Program).

mod args;
mod commands;
// Must stay private and absent from `lib.rs`. `merge_config` trusts that its
// `Args` came from clap, which is where blank-value rejection now lives (#540);
// the post-parse guard was removed as redundant. Re-declaring this as
// `pub mod config` in lib.rs is a one-line change that would hand any dependent
// crate an unvalidated `merge_config`.
mod config;
mod plugin_cmd;
mod selection;

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::MultiProgress;
use rdlp_api::TempRegistry;
use rdlp_api::{RdlpApiError, RdlpClient};
use rdlp_cli::event_handler::CliEventHandler;
use rdlp_cli::interactive::DialoguerCallback;
use rdlp_cli::sanitize::sanitize_for_terminal;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use args::{Args, PluginCmd, PluginSubcommand};
use commands::{fail_with, print_codecs, print_fields};
use config::build_config;

/// #413/#414: spawn the OS-signal → graceful-cancel task for an in-flight
/// download or local-file job. First interrupt → graceful (record the
/// conventional exit code + `interrupt()`, keeping any resumable partial or the
/// user's source); second SIGINT → force-exit. Detached: aborts with the runtime
/// when the job finishes normally.
fn spawn_signal_task(interrupt: rdlp_api::InterruptHandle, exit_signal: Arc<AtomicU8>) {
    use rdlp_cli::signal::{SignalAction, next_action, wait_for_signal};
    tokio::spawn(async move {
        let mut graceful_started = false;
        loop {
            let sig = wait_for_signal().await;
            match next_action(graceful_started, sig) {
                SignalAction::GracefulCancel(code) => {
                    graceful_started = true;
                    exit_signal.store(code, Ordering::SeqCst);
                    interrupt.interrupt();
                }
                SignalAction::ForceExit(code) => std::process::exit(i32::from(code)),
                SignalAction::Ignore => {}
            }
        }
    });
}

/// Get optimal number of worker threads for I/O-heavy workloads
fn optimal_worker_threads() -> usize {
    // For I/O-bound work (downloads), use 2x CPU cores
    // This allows more concurrent I/O operations
    let cpu_count = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);

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
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Build the default filter at `level`, quieting the third-party targets that
/// would otherwise bury it.
///
/// Only [`rdlp_types::log_targets::ZBUS`] applies here. The desktop
/// additionally filters [`rdlp_types::log_targets::TRACING_SPAN_LIFECYCLE`],
/// which is a `log`-bridge artifact:
/// this binary installs a `tracing` subscriber, so zbus's instrumentation
/// arrives as real spans under their own `zbus::…` target and the directive
/// below already covers them.
fn default_filter(level: tracing::Level) -> EnvFilter {
    // `EnvFilter::new` is lossy, NOT panicking: it routes through
    // `parse_lossy`, which prints "ignoring `X`" to stderr and DROPS the bad
    // directive. Whatever parsed is all that remains — the builder's `ERROR`
    // default is added only when NOTHING parsed (`filter/env/builder.rs`,
    // `from_directives`). So a malformed directive fails silently and in the
    // worst direction: a bad level half leaves just `zbus=warn` standing and
    // silences the rest of the tree outright, and a bad zbus half restores
    // exactly the noise this filter exists to remove.
    //
    // That is why the parameter is a `tracing::Level` rather than a `&str`:
    // no value of it can produce an invalid directive, so a config- or
    // CLI-derived string cannot reintroduce the silent-degradation class.
    // Typing the level cannot catch a mistake in the format string itself,
    // which is what `filter_directive_parses` is for.
    EnvFilter::new(format!("{level},{}=warn", rdlp_types::log_targets::ZBUS))
}

fn main() -> Result<()> {
    // Create optimized multi-threaded runtime for I/O-heavy workloads
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(optimal_worker_threads())
        .enable_all()
        .build()
        .context("failed to build Tokio runtime")?;

    let exit_signal = Arc::new(AtomicU8::new(0));
    let result = runtime.block_on(async_main(Arc::clone(&exit_signal)));

    // #413: a graceful signal-cancel set the conventional exit code; emit it.
    let code = exit_signal.load(Ordering::SeqCst);
    if code != 0 {
        std::process::exit(i32::from(code)); // 130 = SIGINT, 143 = SIGTERM
    }
    result
}

#[allow(clippy::too_many_lines)] // top-level CLI dispatch; extracting sub-functions would add indirection without clarity
async fn async_main(exit_signal: Arc<AtomicU8>) -> Result<()> {
    let args = Args::parse();

    // Build config with precedence: CLI > config file > defaults
    let mut config = build_config(&args).context("failed to build configuration")?;

    // Extend trusted publishers from --trust-publisher flags
    if !args.trust_publisher.is_empty() {
        config
            .plugin_trusted_publishers
            .extend(args.trust_publisher.iter().cloned());
    }

    // Plugin management subcommands — handle before any download logic.
    if let Some(PluginSubcommand::Plugin(plugin_args)) = args.plugin {
        match plugin_args.cmd {
            PluginCmd::List => plugin_cmd::run_list(&config)?,
            PluginCmd::Info { name } => plugin_cmd::run_info(&name, &config)?,
            PluginCmd::Retrust { name } => plugin_cmd::run_retrust(&name)?,
            PluginCmd::Disable { name } => plugin_cmd::run_disable(&name)?,
            PluginCmd::Enable { name } => plugin_cmd::run_enable(&name)?,
            PluginCmd::Uninstall { name } => plugin_cmd::run_uninstall(&name, &config)?,
            PluginCmd::BuildFromYtdlp {
                plugin_py,
                output_dir,
            } => plugin_cmd::run_build_from_ytdlp(plugin_py, output_dir).await?,
        }
        return Ok(());
    }

    // Create shared MultiProgress for managing progress bars with log output
    let multi_progress = Arc::new(MultiProgress::new());

    if !config.quiet {
        // Only `-v` consults `RUST_LOG`; without it the filter is fixed at
        // INFO. That asymmetry is pre-existing. Where `RUST_LOG` IS consulted
        // and parses, it is used verbatim — including if it turns the noisy
        // targets back up — so the quieting applies only to the filters we
        // synthesize ourselves.
        let filter = if config.verbose {
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter(tracing::Level::DEBUG))
        } else {
            default_filter(tracing::Level::INFO)
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
    // cleanup_stale performs a blocking directory walk with per-entry metadata
    // syscalls — move to a blocking worker so async_main's runtime thread
    // isn't stalled during startup on slow / network filesystems.
    {
        let output_dir = config.output_directory.clone();
        let _ = tokio::task::spawn_blocking(move || TempRegistry::cleanup_stale(&output_dir)).await;
    }

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
            if let [only] = sites.as_slice() {
                // Leak is fine for a single CLI run
                return Box::leak(only.name.clone().into_boxed_str());
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
            let (key, value) = if let Some((k, v)) = raw.split_once('=') {
                (k.to_string(), v.to_string())
            } else {
                eprintln!("Error: Invalid filter format '{raw}'. Expected key=value.");
                std::process::exit(1);
            };
            filters.push(rdlp_api::SearchFilter { key, value });
        }

        let search_query = rdlp_api::SearchQuery {
            query: query_text.clone(),
            filters,
            max_results: None,
            page: Some(1),
        };

        // The CLI's client is built from the fully-merged config (flags
        // included), so it already carries the cookie source: no per-call
        // override is needed here.
        match client
            .search_page(
                site,
                &search_query,
                &rdlp_api::request::NetworkOptions::default(),
            )
            .await
        {
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
                        eprintln!("{:>3}. {}", i + 1, sanitize_for_terminal(&r.title));
                        eprintln!("     {}", sanitize_for_terminal(&r.video_url));
                        if let Some(d) = r.duration {
                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            // d is a non-negative duration in seconds; values up to ~136 years fit u32
                            let mins = d as u32 / 60;
                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            let secs = d as u32 % 60;
                            eprint!("     Duration: {mins}:{secs:02}");
                        }
                        if let Some(views) = r.view_count {
                            eprint!("  Views: {views}");
                        }
                        if let Some(uploader) = &r.uploader {
                            eprint!("  Uploader: {}", sanitize_for_terminal(uploader));
                        }
                        eprintln!();
                    }
                }
            }
            Err(e) => fail_with("search", &e, verbose),
        }

        return Ok(());
    }

    // Check if URL is provided
    let url = args
        .url
        .ok_or_else(|| anyhow::anyhow!("No URL provided. Use --help for usage information."))?;

    // --list-subs-only: show subtitle menu, download subs, exit (no video)
    if args.list_subs_only {
        let infos = match client
            .extract_info(&url, &rdlp_api::request::NetworkOptions::default())
            .await
        {
            Ok(infos) => infos,
            Err(e) => fail_with("list_subs", &e, verbose),
        };

        // Use the first video's metadata for subtitle selection
        if let Some(info) = infos.first() {
            match client
                .download_subtitles_only(info, &rdlp_api::request::NetworkOptions::default())
                .await
            {
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
                Err(e) => fail_with("list_subs", &e, verbose),
            }
        }

        return Ok(());
    }

    // Metadata-only modes: --dump-json, --print, --list-formats, --simulate
    if args.dump_json || args.print.is_some() || args.list_formats || args.simulate {
        let infos = match client
            .extract_info(&url, &rdlp_api::request::NetworkOptions::default())
            .await
        {
            Ok(infos) => infos,
            Err(e) => fail_with("analyze", &e, verbose),
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
                let table =
                    rdlp_table::render_formats_table(&refs, &rdlp_table::TableOpts::default());
                println!("{}", sanitize_for_terminal(&info.title));
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
                    sanitize_for_terminal(&info.title),
                    sanitize_for_terminal(&info.id),
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

        // #414: a long local transcode must cancel gracefully on Ctrl+C/SIGTERM
        // (cooperative FFmpeg abort + the user's source preserved), not an
        // abrupt kill.
        spawn_signal_task(handle.interrupt_handle(), Arc::clone(&exit_signal));

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
            Err(RdlpApiError::UserCancelled) => {
                // User cancelled - already printed message via events
                Ok(())
            }
            Err(e) => fail_with("process_local", &e, verbose),
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

    // #413: drive cancellation from OS signals (first interrupt → graceful,
    // keep the resumable partial; second SIGINT → force-exit).
    spawn_signal_task(handle.interrupt_handle(), Arc::clone(&exit_signal));

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
        Err(e) => fail_with("download", &e, verbose),
    };

    // Clean up any temp files created during this session (e.g. aborted
    // mid-pipeline). SIGKILL orphans are handled by cleanup_stale() at next startup.
    temp_registry.cleanup_all();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `default_filter` builds its directive with `Level`'s `Display`, which
    /// is uppercase (`INFO`), while the directive it is concatenated with is
    /// lowercase. A directive `EnvFilter` cannot parse is dropped silently
    /// (`parse_lossy`), not rejected, so a formatting mistake here would
    /// produce a filter that looks fine and logs nothing — the parameter being
    /// typed prevents an invalid *level*, never a mistake in this function.
    ///
    /// Asserts the full `zbus=warn` directive rather than the bare target:
    /// `contains("zbus")` would pass on a bare `zbus` directive, which parses
    /// and means TRACE — the precise noise regression this branch removes — and
    /// would not notice the level half being mangled at all.
    #[test]
    fn filter_directive_parses() {
        for level in [tracing::Level::INFO, tracing::Level::DEBUG] {
            let rendered = default_filter(level).to_string();
            assert!(
                rendered.contains(&format!("{}=warn", rdlp_types::log_targets::ZBUS)),
                "zbus must be pinned to warn, not merely present, got: {rendered}"
            );
            assert!(
                rendered
                    .to_lowercase()
                    .contains(&level.to_string().to_lowercase()),
                "the requested level must survive into the filter, got: {rendered}"
            );
        }
    }
}
