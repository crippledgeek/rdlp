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

use bindings::rdlp::plugin::types::{
    ExtractError, Format, InfoDict, PluginInfo, SearchError, SearchFilterDescriptor, SearchPage,
    SearchQuery,
};
use bindings::Guest;

const URL_PREFIX: &str = "https://example.com/video/";

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

    fn search(_q: SearchQuery) -> Result<SearchPage, SearchError> {
        Err(SearchError::Unsupported)
    }

    fn search_filters() -> Vec<SearchFilterDescriptor> {
        Vec::new()
    }
}

bindings::export!(Component with_types_in bindings);
