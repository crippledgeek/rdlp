//! `host:extract-helpers` capability — Slice-2.5 host-side helpers
//! delegating to rdlp's existing extractor primitives. Plugins call
//! these via WIT bindings; the Python compat shim's I/O methods become
//! 2-line passthroughs over them.
//!
//! All functions sync (pure CPU) except `extract_m3u8` which fetches
//! via the existing `host:fetch` wreq client.

use crate::instance::PluginStoreData;
use wasmtime::component::Linker;

fn build_regex(
    pattern: &str,
    flags: crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags,
) -> Result<regex::Regex, regex::Error> {
    use crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags;
    let mut builder = regex::RegexBuilder::new(pattern);
    builder.case_insensitive(flags.contains(RegexFlags::IGNORE_CASE));
    builder.multi_line(flags.contains(RegexFlags::MULTILINE));
    builder.dot_matches_new_line(flags.contains(RegexFlags::DOTALL));
    builder.ignore_whitespace(flags.contains(RegexFlags::VERBOSE));
    builder.build()
}

/// Strip HTML tags + collapse whitespace + unescape entities.
/// Mirrors yt-dlp's `clean_html` (`utils/_utils.py:527-540`).
fn clean_html(html: &str) -> String {
    let collapsed = regex::Regex::new(r"\s+").unwrap().replace_all(html, " ");
    let no_br = regex::Regex::new(r"\s?<\s?br\s?/?\s?>\s?")
        .unwrap()
        .replace_all(&collapsed, "\n");
    let no_p = regex::Regex::new(r"<\s?/\s?p\s?>\s?<\s?p[^>]*>")
        .unwrap()
        .replace_all(&no_br, "\n");
    let no_tags = regex::Regex::new(r"<.*?>").unwrap().replace_all(&no_p, "");
    html_escape::decode_html_entities(&no_tags).trim().to_string()
}

/// Wire `host:extract-helpers` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_extract_helpers::add_to_linker(linker, |s| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::rdlp::plugin::host_extract_helpers::Host as _;

    fn ctx() -> PluginStoreData {
        PluginStoreData::new("test", tokio_util::sync::CancellationToken::new())
    }

    #[tokio::test]
    async fn search_regex_finds_first_match() {
        let mut c = ctx();
        let r = c
            .search_regex(
                r"(\d+)".to_string(),
                "id=42 ts=99".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::empty(),
            )
            .await;
        assert_eq!(r, Some("42".to_string()));
    }

    #[tokio::test]
    async fn search_regex_no_match_returns_none() {
        let mut c = ctx();
        let r = c
            .search_regex(
                r"NOPE".to_string(),
                "irrelevant".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::empty(),
            )
            .await;
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn search_regex_ignore_case_flag() {
        let mut c = ctx();
        let r = c
            .search_regex(
                r"(foo)".to_string(),
                "FOO bar".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::IGNORE_CASE,
            )
            .await;
        assert_eq!(r, Some("FOO".to_string()));
    }

    #[tokio::test]
    async fn html_search_regex_strips_tags() {
        let mut c = ctx();
        let r = c
            .html_search_regex(
                r"<title>(.+?)</title>".to_string(),
                "<title>Hello <b>World</b></title>".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::empty(),
            )
            .await;
        assert_eq!(r, Some("Hello World".to_string()));
    }

    #[tokio::test]
    async fn html_search_regex_unescapes_entities() {
        let mut c = ctx();
        let r = c
            .html_search_regex(
                r"<title>(.+?)</title>".to_string(),
                "<title>Tom &amp; Jerry</title>".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::empty(),
            )
            .await;
        assert_eq!(r, Some("Tom & Jerry".to_string()));
    }
}

impl crate::bindings::rdlp::plugin::host_extract_helpers::Host for PluginStoreData {
    async fn search_regex(
        &mut self,
        pattern: String,
        haystack: String,
        re_flags: crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags,
    ) -> Option<String> {
        let pat = build_regex(&pattern, re_flags).ok()?;
        let m = pat.captures(&haystack)?;
        // Return the first non-None capture group, falling back to group 0.
        for i in 1..m.len() {
            if let Some(g) = m.get(i) {
                return Some(g.as_str().to_string());
            }
        }
        Some(m.get(0)?.as_str().to_string())
    }

    async fn html_search_regex(
        &mut self,
        pattern: String,
        haystack: String,
        re_flags: crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags,
    ) -> Option<String> {
        let raw = self.search_regex(pattern, haystack, re_flags).await?;
        Some(clean_html(&raw))
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
