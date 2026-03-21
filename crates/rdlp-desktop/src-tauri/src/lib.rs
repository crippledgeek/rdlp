//! Tauri v2 desktop frontend for the rdlp download engine.
//!
//! This crate provides a native desktop UI built with Tauri, exposing
//! the rdlp-api functionality through IPC commands for a React
//! frontend.

#![warn(missing_docs)]

/// Tauri IPC command handlers.
pub mod commands;
/// Frontend-facing error types for IPC responses.
pub mod error;
/// Tauri event types for frontend notifications.
pub mod events;
/// Application state managed by Tauri.
pub mod state;

use rdlp_postprocess::TempRegistry;
use state::AppState;
use tauri::Manager;

/// Run the Tauri application.
///
/// Initialises plugins, registers managed state, binds all IPC
/// command handlers, and starts the event loop.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::codecs::get_available_codecs,
            commands::search::search_content,
            commands::search::get_search_providers,
            commands::search::get_search_filters,
            commands::download::start_download,
            commands::download::cancel_download,
            commands::download::get_queue,
            commands::download::remove_job,
            commands::download::clear_completed_jobs,
            commands::download::get_job_options,
            commands::formats::get_formats,
            commands::formats::validate_format_expression,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::pick_directory,
            commands::settings::reveal_in_folder,
            commands::thumbnail::proxy_thumbnail,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                let output_dir = state
                    .settings
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .output_dir
                    .clone();
                // Clean up temp files created by active downloads in this session.
                state.temp_registry.cleanup_all();
                // Also sweep for orphans left by a prior crash (SIGKILL, etc.).
                TempRegistry::cleanup_stale(&output_dir);
            }
        });
}
