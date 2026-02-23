//! Search-related IPC commands.
//!
//! Provides content search, provider listing, and filter discovery
//! exposed to the frontend via Tauri's command system.

use tauri::State;

use rdlp_api::{
    SearchFilter, SearchFilterDescriptor, SearchPageResponse, SearchQuery, SearchSiteInfo,
};

use crate::error::AppError;
use crate::state::AppState;

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
/// Sanitizes the query and delegates to the rdlp-api search engine.
/// When `page` is `Some`, fetches a single page via `search_page()`.
/// When `page` is `None`, collects all pages via `search()` (capped at 40).
///
/// # Arguments
///
/// * `query` - Raw search query from the frontend.
/// * `site` - Site name as returned by [`get_search_providers`].
/// * `filters` - Optional search filters for the chosen site.
/// * `page` - Optional page number for paginated fetching.
/// * `state` - Managed application state containing the API client.
///
/// # Returns
///
/// A [`SearchPageResponse`] with results and pagination metadata.
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
    page: Option<u32>,
    state: State<'_, AppState>,
) -> Result<SearchPageResponse, AppError> {
    let sanitized = sanitize_query(&query);
    if sanitized.is_empty() {
        return Err(AppError::InvalidInput {
            field: "query".to_owned(),
            message: "Search query must not be empty".to_owned(),
        });
    }

    if let Some(p) = page {
        // Paginated mode: fetch a single page
        let search_query = SearchQuery {
            query: sanitized,
            filters,
            max_results: None,
            page: Some(p),
        };

        state
            .client
            .search_page(&site, &search_query)
            .await
            .map_err(|e| AppError::SearchFailed {
                message: e.to_string(),
                retryable: e.is_retryable(),
            })
    } else {
        // Collect-all mode: existing behavior, capped at 40
        let search_query = SearchQuery {
            query: sanitized,
            filters,
            max_results: Some(40),
            page: None,
        };

        let results = state
            .client
            .search(&site, &search_query)
            .await
            .map_err(|e| AppError::SearchFailed {
                message: e.to_string(),
                retryable: e.is_retryable(),
            })?;

        Ok(SearchPageResponse {
            results,
            page: 1,
            has_more: false,
            total_estimate: None,
        })
    }
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
