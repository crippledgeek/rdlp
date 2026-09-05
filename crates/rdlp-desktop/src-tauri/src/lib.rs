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
/// `Info` even in a debug build: the workspace's ~370 `debug!` sites (172 in
/// rdlp-extractor) are per-fragment detail that drowns the record you are
/// actually reading. Raise a single noisy module with `.level_for(...)` while
/// debugging it rather than turning the whole tree up.
const LOG_LEVEL: log::LevelFilter = log::LevelFilter::Info;

/// Bytes per log file before rotation. The plugin's own default is `40_000`,
/// which this size of workspace fills in seconds.
const LOG_MAX_FILE_SIZE: u128 = 5 * 1024 * 1024;

/// How many rotated files to keep, so a bug report can include the run BEFORE
/// the one that crashed. `KeepOne` (the plugin default) cannot.
const LOG_FILES_KEPT: usize = 3;

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
    log_panics::init();

    // SAFETY (expect): startup-time fatal; tauri.conf.json is validated at
    // build time and the application cannot function without the webview.
    #[allow(clippy::expect_used)]
    let app = tauri::Builder::default()
        // Registered first: it installs the global `log` backend, and every
        // other plugin and command logs through the `log` facade. The crate
        // depended on `log` with no backend at all, so every `info!`/`warn!`
        // in the desktop crate was discarded.
        //
        // `Stdout` covers `pnpm tauri dev`; `LogDir` gives a file to attach to
        // a bug report, at `$XDG_DATA_HOME/com.rdlp.desktop/logs` on Linux,
        // rotated by the plugin's own `max_file_size` handling. `Webview`
        // forwards records to the devtools console, where a frontend bug is
        // usually being read anyway — `main.tsx` calls `attachConsole()` to
        // receive them. That is an event listener, not a plugin command, so it
        // needs no `log:` capability entry.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(LOG_LEVEL)
                // The plugin's defaults are 40 KB with `KeepOne` — which
                // DELETES the previous file on every rotation. The workspace
                // has ~370 `debug!` sites (172 in rdlp-extractor alone), so a
                // single download would rotate several times over and leave a
                // file holding only its last seconds: useless for the
                // post-mortem this target exists to serve.
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
