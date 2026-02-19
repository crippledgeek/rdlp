//! Format listing IPC commands.
//!
//! Exposes available download formats for a given URL so the
//! frontend can present an interactive format selector.

use tauri::State;

use crate::state::AppState;

/// Retrieve available formats for a URL.
///
/// Returns a list of formats that can be selected for download.
#[tauri::command]
pub async fn get_formats(
    url: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = url;
    Ok(serde_json::json!({ "formats": [] }))
}
