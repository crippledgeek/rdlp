//! Search-related IPC commands.
//!
//! Provides URL extraction and site-search functionality exposed
//! to the frontend via Tauri's command system.

use tauri::State;

use crate::state::AppState;

/// Extract metadata from a URL.
///
/// Returns the extracted info-dict as a JSON value.
#[tauri::command]
pub async fn search_url(
    url: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = url;
    Ok(serde_json::json!({ "stub": true }))
}

/// Search a supported site by query string.
///
/// Returns search results as a JSON value.
#[tauri::command]
pub async fn search_site(
    query: String,
    site: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = (query, site);
    Ok(serde_json::json!({ "stub": true }))
}

/// Retrieve available search filters for a given site.
///
/// Returns filter descriptors as a JSON value.
#[tauri::command]
pub async fn get_search_filters(
    site: String,
    _state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let _ = site;
    Ok(serde_json::json!({ "stub": true }))
}
