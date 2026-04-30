//! `host:extract-helpers` capability — Slice-2.5 host-side helpers
//! delegating to rdlp's existing extractor primitives. Plugins call
//! these via WIT bindings; the Python compat shim's I/O methods become
//! 2-line passthroughs over them.
//!
//! All functions sync (pure CPU) except `extract_m3u8` which fetches
//! via the existing `host:fetch` wreq client.

use crate::instance::PluginStoreData;
use wasmtime::component::Linker;

/// Wire `host:extract-helpers` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_extract_helpers::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_extract_helpers::Host for PluginStoreData {
    async fn search_regex(
        &mut self,
        _pattern: String,
        _haystack: String,
        _re_flags: crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags,
    ) -> Option<String> {
        unimplemented!("Task 4")
    }

    async fn html_search_regex(
        &mut self,
        _pattern: String,
        _haystack: String,
        _re_flags: crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags,
    ) -> Option<String> {
        unimplemented!("Task 4")
    }

    async fn html_search_meta(&mut self, _name: String, _html: String) -> Option<String> {
        unimplemented!("Task 5")
    }

    async fn og_search_property(&mut self, _prop: String, _html: String) -> Option<String> {
        unimplemented!("Task 6")
    }

    async fn rta_search(&mut self, _html: String) -> Option<u8> {
        unimplemented!("Task 7")
    }

    async fn search_json(
        &mut self,
        _start_pattern: String,
        _end_pattern: String,
        _haystack: String,
    ) -> Option<String> {
        unimplemented!("Task 8")
    }

    async fn extract_m3u8(
        &mut self,
        _url: String,
        _video_id: String,
        _opts: crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options,
    ) -> Result<
        crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Extraction,
        crate::bindings::rdlp::plugin::host_fetch::FetchError,
    > {
        unimplemented!("Task 10")
    }

    async fn extract_json_ld(
        &mut self,
        _html: String,
    ) -> Option<crate::bindings::rdlp::plugin::host_extract_helpers::JsonLdVideo> {
        unimplemented!("Task 11")
    }
}
