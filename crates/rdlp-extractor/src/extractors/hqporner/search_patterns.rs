//! URL builders for HQPorner search and listing pages.

/// Build the next listing page URL from the current URL and page HTML.
///
/// Parses the pagination links to find the "Next" page URL.
///
/// # Arguments
/// * `current_url` - The current listing page URL
/// * `_webpage` - The current page HTML (used for pagination link extraction)
///
/// # Returns
/// The next page URL, or an empty string if no next page exists.
pub(crate) fn next_listing_page_url(current_url: &str, _webpage: &str) -> String {
    // Placeholder — implemented in Task 5
    let _ = current_url;
    String::new()
}
