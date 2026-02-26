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

use state::AppState;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
