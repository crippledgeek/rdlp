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
    ExtractError, Extraction, Format, InfoDict, InfoDictExtra, MetaValue, PlaylistEntry,
    PlaylistError, PlaylistPage, PluginInfo, SearchError, SearchFilterDescriptor, SearchPage,
    SearchQuery, SearchResult,
};

const URL_PREFIX: &str = "https://example.com/video/";

/// The one video URL `extract-with-metadata` declines with
/// `unsupported-url` (a domain outcome, never a strike) — so a host test
/// can watch that variant cross the 0.5.2 export end-to-end. Every other
/// `URL_PREFIX` URL is answered.
const UNSUPPORTED_VIDEO_URL: &str = "https://example.com/video/unsupported";

/// Playlist URL answered with `internal`: the one `playlist-error` case
/// the host counts as a strike.
const PLAYLIST_URL_INTERNAL: &str = "https://example.com/internal-error-playlist";

/// Playlist URL answered with `not-found`: a domain error the host
/// propagates instead of falling back to `extract`.
const PLAYLIST_URL_GONE: &str = "https://example.com/gone-playlist";

/// The one real listing: `PLAYLIST_PAGES` pages of `URL_PREFIX` entries
/// the plugin's own `extract` accepts. Every URL outside this table is
/// `unsupported-url`, which makes the host fall back to a single `extract`.
const PLAYLIST_URL_REAL: &str = "https://example.com/a-real-playlist";

/// A URL `extract-playlist` declines (`unsupported-url`) but `extract`
/// accepts as a single video — the shape the host's fallback exists for.
/// Without a `URL_PREFIX` numeric id, so it carries this literal id.
///
/// Deliberately OUTSIDE this plugin's declared `matches` / `url-regex`
/// (`https://example.com/video/*`): it is reachable only when a host test
/// calls the adapter directly, never through registry routing. Do not
/// widen the manifest to cover it — the routing surface is the video
/// prefix, this is a test seam for the fallback path.
const SINGLE_VIDEO_NOT_A_PLAYLIST: &str = "https://example.com/not-a-playlist";
const SINGLE_VIDEO_NOT_A_PLAYLIST_ID: &str = "not-a-playlist";

/// Pages the real playlist has; page 1 lists videos 1..=2, page 2 lists
/// video 3, so the host must fetch both to see every entry.
const PLAYLIST_PAGES: u32 = 2;

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
            wit_version: "0.5.2".into(),
            matches: vec!["https://example.com/video/*".into()],
            url_regex: Some(r"^https://example\.com/video/(?P<id>\d+)".into()),
            priority: 150,
            claims_override: vec![],
            supports_search: true,
        }
    }

    /// The frozen 0.5.0 `extract`: exactly `extract-with-metadata(url).core`,
    /// as the WIT contract asks of a plugin exporting both.
    fn extract(url: String) -> Result<InfoDict, ExtractError> {
        Self::extract_with_metadata(url).map(|extraction| extraction.core)
    }

    /// The 0.5.2 export: the core `extract` always produced, plus a
    /// non-empty extra — typed fields AND two `extras` entries, so a host
    /// can prove both the lift and its `Config`-driven entry cap.
    /// `UNSUPPORTED_VIDEO_URL` is declined with `unsupported-url`.
    fn extract_with_metadata(url: String) -> Result<Extraction, ExtractError> {
        if url == UNSUPPORTED_VIDEO_URL {
            return Err(ExtractError::UnsupportedUrl(url));
        }
        let core = extract_core(url)?;
        Ok(Extraction {
            core,
            extra: InfoDictExtra {
                actors: vec!["Example Actor".into()],
                channel: None,
                channel_url: None,
                age_limit: Some(18),
                thumbnails: vec![],
                extras: vec![
                    ("studio".into(), MetaValue::Text("Example Studio".into())),
                    ("series".into(), MetaValue::Text("Example Series".into())),
                ],
            },
        })
    }

    /// Canned playlist listing keyed by URL — one URL per `playlist-error`
    /// outcome the host handles differently, plus one real listing. A URL
    /// outside that table is `unsupported-url`, the same answer a plugin
    /// without playlists gives.
    fn extract_playlist(url: String, page: u32) -> Result<PlaylistPage, PlaylistError> {
        match url.as_str() {
            PLAYLIST_URL_INTERNAL => Err(PlaylistError::Internal(format!(
                "example: page {page} of {url} cannot be listed"
            ))),
            PLAYLIST_URL_GONE => Err(PlaylistError::NotFound(url)),
            PLAYLIST_URL_REAL => Ok(real_playlist_page(page)),
            _ => Err(PlaylistError::UnsupportedUrl(url)),
        }
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

/// The video id `extract` answers for: the numeric tail of a `URL_PREFIX`
/// URL (anything else there is a `parse` error), or the literal id of
/// `SINGLE_VIDEO_NOT_A_PLAYLIST`. Every other URL is `unsupported-url`.
fn video_id(url: &str) -> Result<String, ExtractError> {
    if url == SINGLE_VIDEO_NOT_A_PLAYLIST {
        return Ok(SINGLE_VIDEO_NOT_A_PLAYLIST_ID.into());
    }
    let id = url
        .strip_prefix(URL_PREFIX)
        .ok_or_else(|| ExtractError::UnsupportedUrl(url.to_string()))?
        .trim_end_matches('/');
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(ExtractError::Parse(format!("non-numeric id: {id}")));
    }
    Ok(id.to_string())
}

/// The synthetic `info-dict` for a URL `video_id` accepts.
fn extract_core(url: String) -> Result<InfoDict, ExtractError> {
    let id = video_id(&url)?;
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

/// One page of `PLAYLIST_URL_REAL`: page 1 lists videos 1 and 2 and claims
/// more; page 2 lists video 3 and is the last. Any later page is empty and
/// final, so a host that ignores `has-more` still terminates.
fn real_playlist_page(page: u32) -> PlaylistPage {
    let ids: &[u32] = match page {
        1 => &[1, 2],
        2 => &[3],
        _ => &[],
    };
    PlaylistPage {
        entries: ids
            .iter()
            .map(|i| PlaylistEntry {
                url: format!("{URL_PREFIX}{i}"),
                id: Some(i.to_string()),
                title: Some(format!("Example Video {i}")),
            })
            .collect(),
        page,
        has_more: page < PLAYLIST_PAGES,
        playlist_id: Some("a-real-playlist".into()),
        playlist_title: Some("A Real Playlist".into()),
        total_estimate: Some(3),
    }
}

bindings::export!(Component with_types_in bindings);
