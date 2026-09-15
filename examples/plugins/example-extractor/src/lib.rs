//! Reference rdlp plugin: example.com/video/{id} extractor.
//!
//! Pure deterministic plugin with no host capabilities — used by the manual
//! smoke test to verify dispatch end-to-end.
//!
//! ## Building a component the production host can actually load
//!
//! `cargo component build --release` (the usual guest-toolchain path) proves
//! the WIT contract compiles, but its `wasm32-wasip1` output always carries
//! WASI 0.2 imports (the wasip1→preview2 adapter, fused in regardless of
//! whether this crate calls anything WASI-backed) — and `rdlp-plugin`'s host
//! wires only its six `rdlp:plugin/host-*` capability interfaces, never WASI
//! (see `crates/rdlp-plugin/src/lib.rs`, "Known limitations"). Such a
//! component traps at instantiation with `component imports instance
//! 'wasi:cli/environment@0.2.3' ... not found in the linker`. The
//! WASI-free, host-loadable recipe (measured rdlp#762 slice B, Task 1):
//!
//! ```sh
//! rustup target add wasm32-unknown-unknown   # once
//! cargo build --release --target wasm32-unknown-unknown
//! wasm-tools component new \
//!     target/wasm32-unknown-unknown/release/example_extractor.wasm \
//!     -o plugin.wasm
//! ```
//!
//! Plain `cargo build` against `wasm32-unknown-unknown` already emits the
//! component-type custom section `wit-bindgen-rt` needs, so no
//! `cargo-component` fallback or `--adapt` step is required; `wasm-tools
//! component wit plugin.wasm` on the result shows only
//! `rdlp:plugin/types@…` imported.

#[allow(warnings)]
mod bindings;

use bindings::Guest;
use bindings::rdlp::plugin::types::{
    ExtractError, Format, InfoDict, PluginInfo, SearchError, SearchFilterDescriptor, SearchPage,
    SearchQuery, SearchResult,
};

const URL_PREFIX: &str = "https://example.com/video/";

/// Results on every canned search page (videos `1..=CANNED_RESULTS`).
const CANNED_RESULTS: u32 = 2;

/// Pages the canned search claims to have; the last one repeats the first.
const CANNED_PAGES: u32 = 2;

/// The `ordering` value under which page 1 still claims a further page
/// but page 2 fails with `internal`. A host that fetches a page it should
/// not have (past `max-results`) turns that into a counted fault, which is
/// how a test observes the fetch: the host instantiates a fresh store per
/// call, so the plugin itself cannot count them.
const ORDERING_TRAPS_PAST_PAGE_ONE: &str = "views";

struct Component;

impl Guest for Component {
    fn metadata() -> PluginInfo {
        PluginInfo {
            name: "example".into(),
            version: "0.1.0".into(),
            wit_version: "0.5.1".into(),
            matches: vec!["https://example.com/video/*".into()],
            url_regex: Some(r"^https://example\.com/video/(?P<id>\d+)".into()),
            priority: 150,
            claims_override: vec![],
            supports_search: false,
        }
    }

    fn extract(url: String) -> Result<InfoDict, ExtractError> {
        let id = url
            .strip_prefix(URL_PREFIX)
            .ok_or_else(|| ExtractError::UnsupportedUrl(url.clone()))?
            .trim_end_matches('/')
            .to_string();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(ExtractError::Parse(format!("non-numeric id: {id}")));
        }
        Ok(InfoDict {
            id: id.clone(),
            title: format!("Example Video {id}"),
            url: Some(url),
            formats: vec![Format {
                format_id: "0".into(),
                url: format!("https://example.com/video/{id}.mp4"),
                ext: "mp4".into(),
                protocol: "https".into(),
                width: Some(1280),
                height: Some(720),
                fps: Some(30.0),
                tbr: None,
                vbr: None,
                abr: None,
                vcodec: Some("h264".into()),
                acodec: Some("aac".into()),
                container: Some("mp4".into()),
                filesize: None,
                format_note: Some("synthetic".into()),
            }],
            subtitles: vec![],
            thumbnail: None,
            description: Some("Synthetic InfoDict from the rdlp example plugin.".into()),
            uploader: None,
            uploader_id: None,
            upload_date: None,
            duration: Some(60),
            view_count: None,
            like_count: None,
            tags: vec![],
            categories: vec![],
        })
    }

    /// Canned two-result page so the host's search path can be exercised
    /// end-to-end without a network; echoes the requested page number.
    /// Page 1 claims a further page exists and page 2 repeats the same two
    /// results, so a host that terminates on a duplicate page stops at two
    /// results while one that trusts `has-more` alone would loop. Under
    /// `ordering=views` page 2 is an `internal` error instead (see
    /// `ORDERING_TRAPS_PAST_PAGE_ONE`).
    fn search(q: SearchQuery) -> Result<SearchPage, SearchError> {
        let page = q.page.unwrap_or(1);
        let traps_past_page_one = q
            .filters
            .iter()
            .any(|(k, v)| k == "ordering" && v == ORDERING_TRAPS_PAST_PAGE_ONE);
        if traps_past_page_one && page > 1 {
            return Err(SearchError::Internal(format!(
                "page {page} requested under ordering={ORDERING_TRAPS_PAST_PAGE_ONE}"
            )));
        }
        let results = (1..=CANNED_RESULTS)
            .map(|i| SearchResult {
                url: format!("{URL_PREFIX}{i}"),
                title: format!("Example result {i} for {}", q.query),
                thumbnail: None,
                duration: Some(60),
                uploader: None,
            })
            .collect();
        Ok(SearchPage {
            results,
            page,
            has_more: page < CANNED_PAGES,
            total_estimate: Some(CANNED_RESULTS.into()),
        })
    }

    /// One descriptor, so a host that resolves `search-filters` by name has
    /// something non-empty to observe (a 0.5.0 build has no such export).
    fn search_filters() -> Vec<SearchFilterDescriptor> {
        vec![SearchFilterDescriptor {
            key: "ordering".into(),
            display_name: "Ordering".into(),
            allowed_values: vec!["newest".into(), "views".into()],
            default: Some("newest".into()),
        }]
    }
}

bindings::export!(Component with_types_in bindings);
