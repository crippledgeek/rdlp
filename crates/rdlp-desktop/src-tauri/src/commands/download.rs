//! Download lifecycle IPC commands.
//!
//! Provides commands to start, cancel, query, and remove downloads
//! from the managed download queue.

use tauri::State;

use crate::state::AppState;

/// Start a new download for the given URL.
///
/// The download is added to the queue and begins processing
/// asynchronously.
#[tauri::command]
pub async fn start_download(
    url: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = url;
    Ok(serde_json::json!({ "stub": true }))
}

/// Cancel an in-progress download by its identifier.
#[tauri::command]
pub async fn cancel_download(
    download_id: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = download_id;
    Ok(serde_json::json!({ "stub": true }))
}

/// Retrieve the current download queue.
///
/// Returns all queued, active, and completed downloads.
#[tauri::command]
pub async fn get_queue(_state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "queue": [] }))
}

/// Remove a completed or failed download from the queue.
#[tauri::command]
pub async fn remove_from_queue(
    download_id: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = download_id;
    Ok(serde_json::json!({ "stub": true }))
}
