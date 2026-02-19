//! Search-related IPC commands.
//!
//! Provides content search, provider listing, and filter discovery
//! exposed to the frontend via Tauri's command system.

use serde::Serialize;
use tauri::State;

use rdlp_api::{
    SearchFilter, SearchFilterDescriptor, SearchQuery, SearchResultPreview, SearchSiteInfo,
};

use crate::error::AppError;
use crate::state::AppState;

/// Response wrapper for search results.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    /// The search result items.
    results: Vec<SearchResultPreview>,
    /// Optional estimate of total matching results.
    total_estimate: Option<u64>,
}

/// Sanitize a user-provided query string.
///
/// Trims whitespace, strips control characters, and limits length to
/// 500 characters to prevent oversized or malformed queries from
/// reaching the backend.
fn sanitize_query(input: &str) -> String {
    input
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect()
}

/// Search a supported site by query string.
///
/// Sanitizes the query, builds a [`SearchQuery`] with up to 40 results,
/// and delegates to the rdlp-api search engine.
///
/// # Arguments
///
/// * `query` - Raw search query from the frontend.
/// * `site` - Site name as returned by [`get_search_providers`].
/// * `filters` - Optional search filters for the chosen site.
/// * `state` - Managed application state containing the API client.
///
/// # Returns
///
/// A [`SearchResponse`] with results and an optional total estimate.
///
/// # Errors
///
/// * [`AppError::InvalidInput`] if the sanitized query is empty.
/// * [`AppError::SearchFailed`] if the backend search call fails.
#[tauri::command]
pub async fn search_content(
    query: String,
    site: String,
    filters: Vec<SearchFilter>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, AppError> {
    let sanitized = sanitize_query(&query);
    if sanitized.is_empty() {
        return Err(AppError::InvalidInput {
            field: "query".to_owned(),
            message: "Search query must not be empty".to_owned(),
        });
    }

    let search_query = SearchQuery {
        query: sanitized,
        filters,
        max_results: Some(40),
    };

    let results = state
        .client
        .search(&site, &search_query)
        .await
        .map_err(|e| AppError::SearchFailed {
            message: e.to_string(),
            retryable: e.is_retryable(),
        })?;

    Ok(SearchResponse {
        total_estimate: None,
        results,
    })
}

/// List all sites that support search.
///
/// # Arguments
///
/// * `state` - Managed application state containing the API client.
///
/// # Returns
///
/// A list of [`SearchSiteInfo`] with name and display name.
///
/// # Errors
///
/// Returns [`AppError::Internal`] on unexpected failures (none expected
/// from the synchronous underlying call).
#[tauri::command]
pub async fn get_search_providers(
    state: State<'_, AppState>,
) -> Result<Vec<SearchSiteInfo>, AppError> {
    Ok(state.client.list_search_sites())
}

/// Retrieve available search filters for a given site.
///
/// # Arguments
///
/// * `site` - Site name as returned by [`get_search_providers`].
/// * `state` - Managed application state containing the API client.
///
/// # Returns
///
/// A list of [`SearchFilterDescriptor`] describing the available
/// filter keys, allowed values, and defaults.
///
/// # Errors
///
/// * [`AppError::InvalidInput`] if the site name is unknown.
#[tauri::command]
pub async fn get_search_filters(
    site: String,
    state: State<'_, AppState>,
) -> Result<Vec<SearchFilterDescriptor>, AppError> {
    state
        .client
        .search_filters(&site)
        .map_err(|e| AppError::InvalidInput {
            field: "site".to_owned(),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_query_trims() {
        assert_eq!(sanitize_query("  hello world  "), "hello world");
    }

    #[test]
    fn test_sanitize_query_strips_control_chars() {
        assert_eq!(sanitize_query("hello\x00\x01world"), "helloworld");
        assert_eq!(sanitize_query("tab\there"), "tabhere");
    }

    #[test]
    fn test_sanitize_query_length_limit() {
        let long = "a".repeat(1000);
        let result = sanitize_query(&long);
        assert_eq!(result.len(), 500);
    }

    #[test]
    fn test_sanitize_query_empty() {
        assert_eq!(sanitize_query(""), "");
        assert_eq!(sanitize_query("   "), "");
        assert_eq!(sanitize_query("\x00\x01\x02"), "");
    }
}
