//! ABXXX-specific URL helpers that compose the shared KVS API helpers.
//!
//! All KVS-generic logic (URL obfuscation reversal, file-formats blob
//! parsing, per-video lookup endpoint shape) lives in
//! [`crate::base::kvs`]; this module owns only the bits that are
//! genuinely site-specific:
//!
//! - the deployment's base URL (`https://abxxx.com`);
//! - the slug-driven search URL — sort/section choices match the live
//!   `app.js` `videos()` helper for ABXXX and may differ across sibling
//!   deployments.

use super::ABXXX_BASE_URL;

const SEARCH_RESULTS_PER_PAGE: u32 = 60;

/// Build the slug-driven search URL used to look up file format details
/// for a single video page on ABXXX.
///
/// The search query is the slug with hyphens replaced by spaces. The
/// canonical row is then identified by matching `video_id` in the result
/// set, not by relying on positional order — see
/// [`crate::base::kvs::file_formats::parse_search_for_file_formats`].
#[must_use]
pub(crate) fn slug_search_endpoint(slug: &str) -> String {
    let query = slug.replace('-', " ");
    let q = urlencoding::encode(&query);
    format!(
        "{ABXXX_BASE_URL}/api/videos2.php?params=0/str/relevance/{count}/search..1.all..&s={q}",
        count = SEARCH_RESULTS_PER_PAGE,
        q = q,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_search_endpoint_replaces_hyphens_with_spaces() {
        let url = slug_search_endpoint("excogi-katie-carmine-in-hd");
        assert!(url.contains("&s=excogi%20katie%20carmine%20in%20hd"));
        assert!(url.contains("/search..1.all.."));
    }
}
