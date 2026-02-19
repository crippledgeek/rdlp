//! Application settings IPC commands.
//!
//! Provides commands to read, update, and interact with persistent
//! application settings such as the download directory.

use tauri::{AppHandle, State};

use crate::state::AppState;

/// Retrieve the current application settings.
#[tauri::command]
pub async fn get_settings(_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "stub": true }))
}

/// Update application settings with new values.
#[tauri::command]
pub async fn update_settings(
    settings: serde_json::Value,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = settings;
    Ok(serde_json::json!({ "stub": true }))
}

/// Open a native directory picker dialog and return the selected
/// path.
#[tauri::command]
pub async fn pick_directory(_app: AppHandle) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "stub": true }))
}
