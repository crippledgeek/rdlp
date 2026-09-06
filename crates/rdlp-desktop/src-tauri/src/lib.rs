//! Tauri v2 desktop frontend for the rdlp download engine.
//!
//! This crate provides a native desktop UI built with Tauri, exposing
//! the rdlp-api functionality through IPC commands for a React
//! frontend.

#![warn(missing_docs)]
#![warn(clippy::pedantic, clippy::nursery, clippy::indexing_slicing)]

// No outer `///` on the modules below: each already carries its own `//!`
// doc, and an outer doc merged with it makes rustdoc resolve the inner
// doc's intra-doc links against THIS module's scope instead of the
// submodule's own — "no item named `AppError` in scope" (#661).
pub mod commands;
pub mod error;
pub mod events;
pub mod state;

use rdlp_postprocess::TempRegistry;
use state::AppState;
use tauri::Manager;

/// Level for the desktop's log targets.
///
/// `Info` even in a debug build: the workspace's ~436 `debug!` sites (172 in
/// rdlp-extractor) are per-fragment detail that drowns the record you are
/// actually reading. Raise a single noisy module with `.level_for(...)` while
/// debugging it rather than turning the whole tree up.
///
/// That volume is also why rotation is sized the way it is below; the figure
/// is stated here only, since two copies of it drifted independently once
/// already.
const LOG_LEVEL: log::LevelFilter = log::LevelFilter::Info;

/// Bytes per log file before rotation. The plugin's own default is `40_000`,
/// which this size of workspace fills in seconds.
const LOG_MAX_FILE_SIZE: u128 = 5 * 1024 * 1024;

/// How many rotated files to keep, so a bug report can include the run BEFORE
/// the one that crashed. `KeepOne` (the plugin default) cannot.
const LOG_FILES_KEPT: usize = 3;

/// Restrict the log directory to the owning user.
///
/// The plugin opens log files with a bare `OpenOptions::create(true)`, so they
/// land at the process umask (typically 0644) and are world-readable on a
/// shared machine. Nothing logged today is secret — but the whole point of
/// this branch is that records now PERSIST, and the blast radius of a future
/// mistake should not be "every local user".
///
/// The DIRECTORY is restricted rather than the file: `RotationStrategy`
/// re-creates the file through that same unmoded `OpenOptions`, so a chmod on
/// the file would be silently undone at the first rotation, while 0700 on the
/// directory covers every rotated file by construction.
///
/// Best-effort and non-fatal: a logger that cannot be locked down is still
/// better than no logger, and this must not stop the app from starting.
///
/// One window remains, and is accepted rather than unnoticed: the plugin
/// creates the directory and opens the first file during `build()`, at the
/// process umask, and this runs after `build()` returns. For that sub-
/// millisecond gap, once per launch, both are world-readable. Closing it would
/// mean pre-creating the directory before the builder exists — which means
/// reimplementing Tauri's `app_log_dir()` path resolution by hand, a worse
/// trade against an attacker who must poll that exact path at that exact
/// moment.
#[cfg(unix)]
fn restrict_log_dir(app: &tauri::App) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(dir) = app.path().app_log_dir() else {
        return;
    };
    // The plugin creates this directory while initializing its `LogDir`
    // target, which has already run by the time `build()` returns — so this
    // only tightens what exists rather than creating it. If it is somehow
    // absent there is nothing to protect and nothing to warn about yet.
    if !dir.is_dir() {
        return;
    }
    if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
        log::warn!("could not restrict the log directory to this user: {e}");
    }
}

/// No-op off Unix: permission bits are a POSIX concept, and Windows/macOS
/// place the log directory inside the user's own profile already.
#[cfg(not(unix))]
fn restrict_log_dir(_app: &tauri::App) {}

/// Run the Tauri application.
///
/// Initialises plugins, registers managed state, binds all IPC
/// command handlers, and starts the event loop.
///
/// # Panics
///
/// Panics if the Tauri application cannot be built (e.g. invalid
/// `tauri.conf.json`) or if the main webview window cannot be opened.
/// These are unrecoverable startup failures.
pub fn run() {
    // Route panics through `log` so they reach the same targets as every other
    // record. Panics in async command handlers are the reason this matters: a
    // panicking Tauri command never sends an IPC response, so the caller's
    // promise hangs with no error anywhere — how #693 stayed invisible.
    //
    // Chained onto the default hook rather than replacing it. `log_panics`
    // calls `set_hook` without `take_hook`, so on its own it would DROP the
    // default stderr printer — and the log backend is not installed until the
    // plugin below initializes. A panic in the builder chain or another
    // plugin's setup would then reach a facade with no logger and no stderr
    // fallback: silent, which is the exact failure this work exists to remove.
    let default_hook = std::panic::take_hook();
    log_panics::init();
    let log_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_hook(info);
        default_hook(info);
    }));

    // SAFETY (expect): startup-time fatal; tauri.conf.json is validated at
    // build time and the application cannot function without the webview.
    #[allow(clippy::expect_used)]
    let app = tauri::Builder::default()
        // Registered first: it installs the global `log` backend, and every
        // other plugin and command logs through the `log` facade. The crate
        // depended on `log` with no backend at all, so every `info!`/`warn!`
        // in the desktop crate was discarded.
        //
        // Two sinks, and deliberately no stdout one. `LogDir` gives a file to
        // attach to a bug report, at `$XDG_DATA_HOME/com.rdlp.desktop/logs` on
        // Linux, rotated by the plugin's own `max_file_size` handling.
        // `Webview` emits each record as the `log://log` event, which has two
        // subscribers: `events/registerLogEvents.ts` feeds the in-app Log
        // Viewer, and `main.tsx` calls `attachConsole()` to mirror records into
        // the devtools console. Both are event listeners rather than plugin
        // commands, so neither needs a `log:` capability entry.
        //
        // A terminal is not one of the places a desktop user reads logs: under
        // a bundled launch there is no attached console at all, so the records
        // a `Stdout` target wrote were discarded everywhere except
        // `pnpm tauri dev`. The Log Viewer is the surface that replaces it, and
        // until it subscribed to `log://log` it showed only per-job
        // `download-log` messages — no facade record had ever reached it.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    // Formatted for a pane that renders its own timestamp and
                    // level badge: emitting the builder's default
                    // `[date][time][target][LEVEL] msg` would print both twice
                    // in every row. The target survives because it is the one
                    // part the pane cannot recover — which crate spoke.
                    //
                    // Applied per-target, so `LogDir` below keeps the full
                    // default prefix. A file has no surrounding UI to carry
                    // that context.
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview).format(
                        |out, message, record| {
                            out.finish(format_args!("[{}] {message}", record.target()));
                        },
                    ),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(LOG_LEVEL)
                // Third-party chatter, measured rather than guessed: in a
                // 287-line real session zbus's D-Bus handshake produced 140
                // lines at INFO and its span lifecycle another 28 — together
                // 58% of the file, against 23 lines of actual signal. They
                // became visible when the workspace enabled `tracing`'s `log`
                // feature so rdlp-http would stop being invisible; that bridge
                // carries third-party `tracing` events too.
                //
                // Filtered at this sink rather than at the source because
                // these are not our crates. Our own records reach the facade
                // as `log` records under their module targets and are
                // unaffected. The span filter is the one to be careful with:
                // it is not zbus-specific and cannot be narrowed — see the
                // constant's own documentation for why, before widening or
                // trusting it.
                //
                // `rdlp-cli` filters the same zbus target through its own
                // `EnvFilter`; the shared constants are what keep the two
                // sinks from drifting apart.
                .level_for(rdlp_types::log_targets::ZBUS, log::LevelFilter::Warn)
                .level_for(
                    rdlp_types::log_targets::TRACING_SPAN_LIFECYCLE,
                    log::LevelFilter::Warn,
                )
                // The plugin's defaults are 40 KB with `KeepOne` — which
                // DELETES the previous file on every rotation. At the `debug!`
                // volume noted on `LOG_LEVEL`, a single download would rotate
                // several times over and leave a file holding only its last
                // seconds: useless for the post-mortem this target exists to
                // serve.
                .max_file_size(LOG_MAX_FILE_SIZE)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(LOG_FILES_KEPT))
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::codecs::available_codecs,
            commands::codecs::available_audio_codecs,
            commands::search::search_content,
            commands::search::enrich_search_result,
            commands::search::search_providers,
            commands::search::search_filters,
            commands::download::start_download,
            commands::download::cancel_download,
            commands::download::queue,
            commands::download::remove_job,
            commands::download::clear_completed_jobs,
            commands::download::job_options,
            commands::formats::formats,
            commands::formats::validate_format_expression,
            commands::settings::settings,
            commands::settings::update_settings,
            commands::settings::pick_directory,
            commands::settings::reveal_in_folder,
            commands::thumbnail::proxy_thumbnail,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    restrict_log_dir(&app);

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            let state = app.state::<AppState>();
            let output_dir = state
                .settings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .output_dir
                .clone();
            // Clean up temp files created by active downloads in this session.
            state.temp_registry.cleanup_all();
            // Also sweep for orphans left by a prior crash (SIGKILL, etc.).
            TempRegistry::cleanup_stale(&output_dir);
        }
    });
}
