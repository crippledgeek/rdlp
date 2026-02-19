//! Tauri v2 desktop frontend for the rdlp download engine.
//!
//! This crate provides a native desktop UI built with Tauri, exposing
//! the rdlp-api functionality through IPC commands for a React
//! frontend.

#![warn(missing_docs)]

/// Tauri IPC command handlers.
pub mod commands;
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
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::search::search_url,
            commands::search::search_site,
            commands::search::get_search_filters,
            commands::download::start_download,
            commands::download::cancel_download,
            commands::download::get_queue,
            commands::download::remove_from_queue,
            commands::formats::get_formats,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::settings::pick_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
