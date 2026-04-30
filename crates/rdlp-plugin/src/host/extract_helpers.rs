//! `host:extract-helpers` capability — Slice-2.5 host-side helpers
//! delegating to rdlp's existing extractor primitives. Plugins call
//! these via WIT bindings; the Python compat shim's I/O methods become
//! 2-line passthroughs over them.
//!
//! All functions sync (pure CPU) except `extract_m3u8` which fetches
//! via the existing `host:fetch` wreq client.

use std::sync::LazyLock;

use crate::instance::PluginStoreData;
use wasmtime::component::Linker;

static RE_WHITESPACE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\s+").expect("valid whitespace pattern"));
static RE_BR: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\s?<\s?br\s?/?\s?>\s?").expect("valid <br> pattern"));
static RE_P: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<\s?/\s?p\s?>\s?<\s?p[^>]*>").expect("valid <p> pattern"));
static RE_TAGS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<.*?>").expect("valid tag-strip pattern"));

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
    let collapsed = RE_WHITESPACE.replace_all(html, " ");
    let no_br = RE_BR.replace_all(&collapsed, "\n");
    let no_p = RE_P.replace_all(&no_br, "\n");
    let no_tags = RE_TAGS.replace_all(&no_p, "");
    html_escape::decode_html_entities(&no_tags)
        .trim()
        .to_string()
}

/// Wire `host:extract-helpers` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_extract_helpers::add_to_linker(linker, |s| s)
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
        // Mirror yt-dlp's `_search_regex` group semantics:
        // if there are any named/unnamed capture groups (`m.len() > 1`),
        // return group 1 unconditionally — even if it captured an empty string.
        // Only fall back to group 0 (the whole match) when there are no groups.
        if m.len() > 1 {
            // `m.get(1)` is always Some when the overall match succeeded and
            // m.len() > 1, UNLESS the group is optional and did not participate.
            // Return the captured string, which may be empty ("").
            Some(
                m.get(1)
                    .map_or_else(String::new, |g| g.as_str().to_string()),
            )
        } else {
            Some(m.get(0)?.as_str().to_string())
        }
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

    async fn html_search_meta(&mut self, name: String, html: String) -> Option<String> {
        // Mirrors yt-dlp's `_html_search_meta` (extractor/common.py:1492+).
        // Tries 5 attribute names: itemprop / name / property / id / http-equiv.
        // Tries content-after AND content-before patterns.
        //
        // L3 fix: the old `[^"']*` content pattern truncated on mixed-quote
        // content (e.g. `content="a'b"` → returned `"a"` instead of `"a'b"`).
        // We now use two separate regexes per layout — one for double-quoted
        // content and one for single-quoted — so each pattern uses the *same*
        // quote character that opened the value.
        let escaped = regex::escape(&name);
        let attrs = "(?:itemprop|name|property|id|http-equiv)";
        // Each (name_attr, content_attr) pattern comes in two quote flavours.
        // Group 1 always captures the content value.
        let patterns: &[String] = &[
            // content-after, double-quoted content
            format!(r#"<meta[^>]+(?:{attrs})=["']{escaped}["'][^>]*content="([^"]*)"#),
            // content-after, single-quoted content
            format!(r#"<meta[^>]+(?:{attrs})=["']{escaped}["'][^>]*content='([^']*)'"#),
            // content-before, double-quoted content
            format!(r#"<meta[^>]+content="([^"]*)"[^>]*(?:{attrs})=["']{escaped}["']"#),
            // content-before, single-quoted content
            format!(r#"<meta[^>]+content='([^']*)'[^>]*(?:{attrs})=["']{escaped}["']"#),
        ];
        for pat in patterns {
            let re = regex::RegexBuilder::new(pat)
                .case_insensitive(true)
                .build()
                .ok()?;
            if let Some(m) = re.captures(&html)
                && let Some(g) = m.get(1)
            {
                return Some(g.as_str().to_string());
            }
        }
        None
    }

    async fn og_search_property(&mut self, prop: String, html: String) -> Option<String> {
        // Mirrors yt-dlp's `_og_regexes` + `_og_search_property` (common.py:1463-1490).
        let prop_escaped = regex::escape(&prop);
        // content= with double-quote (group 1), single-quote (group 2),
        // or unquoted HTML5 attribute value (group 3).
        // The unquoted branch uses `[^\s"'=<>`]+` — excludes whitespace and the
        // HTML5-forbidden characters (`"`, `'`, `=`, `<`, `>`, backtick) but allows
        // `/` so URLs like `https://x.com/y.jpg` round-trip correctly.  In practice
        // the value is terminated by the space that precedes `/>` (or the next
        // attribute), so `/` inside the value is unambiguous.  The `regex` crate
        // does not support lookaheads, so this is the correct no-lookahead form.
        let content_re = r#"content=(?:"([^"]+?)"|'([^']+?)'|([^\s"'=<>`]+))"#;
        let sep = r"(?:&#x3A;|[:-])";
        let property_re =
            format!(r#"(?:name|property)=(?:'og{sep}{prop_escaped}'|"og{sep}{prop_escaped}")"#);
        let templates = [
            format!(r#"<meta[^>]+?{property_re}[^>]+?{content_re}"#),
            format!(r#"<meta[^>]+?{content_re}[^>]+?{property_re}"#),
        ];
        for pat in &templates {
            let Ok(re) = regex::RegexBuilder::new(pat)
                .dot_matches_new_line(true)
                .case_insensitive(true)
                .build()
            else {
                continue;
            };
            if let Some(m) = re.captures(&html) {
                for i in 1..m.len() {
                    if let Some(g) = m.get(i) {
                        return Some(html_escape::decode_html_entities(g.as_str()).into_owned());
                    }
                }
            }
        }
        None
    }

    async fn rta_search(&mut self, html: String) -> Option<u8> {
        // Mirrors yt-dlp's `_rta_search` (common.py:1525-1543).
        let official = regex::RegexBuilder::new(
            r#"<meta\s+name="rating"\s+content="RTA-5042-1996-1400-1577-RTA""#,
        )
        .case_insensitive(true)
        .ignore_whitespace(true)
        .build()
        .ok()?;
        if official.is_match(&html) {
            return Some(18);
        }
        let markers = [
            r#"Proudly Labeled <a href="http://www\.rtalabel\.org/" title="Restricted to Adults">RTA</a>"#,
            r">[^<]*you acknowledge you are at least (\d+) years old",
            r">\s*(?:18\s+U(?:\.S\.C\.|SC)\s+)?(?:§+\s*)?2257\b",
        ];
        let mut age_limit: Option<u8> = None;
        for m in markers {
            let Some(re) = regex::Regex::new(m).ok() else {
                continue;
            };
            if let Some(cap) = re.captures(&html) {
                let val: u8 = cap
                    .get(1)
                    .and_then(|g| g.as_str().parse().ok())
                    .unwrap_or(18);
                age_limit = Some(age_limit.map_or(val, |x| x.max(val)));
            }
        }
        age_limit
    }

    async fn search_json(
        &mut self,
        start_pattern: String,
        end_pattern: String,
        haystack: String,
    ) -> Option<String> {
        // Mirrors yt-dlp's `_search_json` brace-balanced extraction.
        // Default contains-pattern is `{(?s:.+)}` — greedy, allows nesting.
        let full = format!(r"(?:{start_pattern})\s*(?P<json>\{{(?s:.+)\}})\s*(?:{end_pattern})");
        let re = regex::Regex::new(&full).ok()?;
        let cap = re.captures(&haystack)?;
        let json = cap.name("json")?.as_str();
        Some(json.to_string())
    }

    async fn extract_m3u8(
        &mut self,
        url: String,
        _video_id: String,
        opts: crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options,
    ) -> Result<
        crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Extraction,
        crate::bindings::rdlp::plugin::host_fetch::FetchError,
    > {
        use crate::bindings::rdlp::plugin::host_extract_helpers::{
            ExtractHelpersSubtitle, M3u8Extraction, M3u8Format,
        };
        use crate::bindings::rdlp::plugin::host_fetch::{FetchError, Host as FetchHost, Request};

        let result: Result<M3u8Extraction, FetchError> = async {
            let req = Request {
                url: url.clone(),
                method: "GET".to_string(),
                headers: vec![],
                body: None,
                timeout_ms: Some(30_000),
            };
            let resp = self.fetch(req).await?;
            let body =
                String::from_utf8(resp.body).map_err(|e| FetchError::Network(e.to_string()))?;
            let variants = rdlp_extractor::hls::parse_master_playlist(&url, &body)
                .map_err(FetchError::Network)?;
            let formats: Vec<M3u8Format> = variants
                .into_iter()
                .map(|v| M3u8Format {
                    format_id: opts
                        .m3u8_id
                        .as_deref()
                        .map(|p| format!("{p}-{}", v.format_id))
                        .unwrap_or(v.format_id),
                    url: v.url,
                    ext: opts.ext.clone().unwrap_or(v.ext),
                    protocol: opts.protocol.clone().unwrap_or(v.protocol),
                    tbr: v.tbr,
                    width: v.width,
                    height: v.height,
                    fps: v.fps,
                    vcodec: v.vcodec,
                    acodec: v.acodec,
                    vbr: v.vbr,
                    abr: v.abr,
                    language: v.language,
                    format_note: v.format_note,
                    format_index: v.format_index,
                    manifest_url: v.manifest_url,
                    has_drm: v.has_drm,
                    preference: v.preference,
                    quality: v.quality,
                })
                .collect();
            Ok(M3u8Extraction {
                formats,
                subtitles: Vec::<ExtractHelpersSubtitle>::new(),
            })
        }
        .await;

        match (result, opts.fatal) {
            (Ok(x), _) => Ok(x),
            (Err(e), true) => Err(e),
            (Err(_), false) => Ok(M3u8Extraction {
                formats: vec![],
                subtitles: Vec::<ExtractHelpersSubtitle>::new(),
            }),
        }
    }

    async fn extract_json_ld(
        &mut self,
        html: String,
    ) -> Option<crate::bindings::rdlp::plugin::host_extract_helpers::JsonLdVideo> {
        use crate::bindings::rdlp::plugin::host_extract_helpers::JsonLdVideo;
        let parsed = scraper::Html::parse_document(&html);
        let v = rdlp_extractor::base::common::json_ld::extract_json_ld(&parsed)?;
        // Duration: parse ISO 8601 string via BaseExtractor, convert f64 → u32 with range check
        let duration = v
            .duration
            .as_deref()
            .and_then(rdlp_extractor::base::common::BaseExtractor::parse_iso8601_duration)
            .and_then(|d| {
                if d >= 0.0 && d <= u32::MAX as f64 {
                    Some(d as u32)
                } else {
                    None
                }
            });
        // Thumbnails: extract_thumbnails returns Option<Vec<rdlp_types::Thumbnail>>;
        // Thumbnail.url is String (not Option<String>), so map, not filter_map.
        let thumbnails = rdlp_extractor::base::common::json_ld::extract_thumbnails(&v)
            .map(|ts| ts.into_iter().map(|t| t.url).collect())
            .unwrap_or_default();
        Some(JsonLdVideo {
            title: v.name.clone(),
            description: v.description.clone(),
            thumbnail: rdlp_extractor::base::common::json_ld::get_thumbnail_url(&v),
            thumbnails,
            upload_date: v.upload_date.clone(),
            duration,
            view_count: rdlp_extractor::base::common::json_ld::extract_view_count(&v),
            like_count: rdlp_extractor::base::common::json_ld::extract_like_count(&v),
            tags: rdlp_extractor::base::common::json_ld::extract_tags(&v).unwrap_or_default(),
            categories: rdlp_extractor::base::common::json_ld::extract_categories(&v)
                .unwrap_or_default(),
        })
    }
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

    /// Regression guard for L2: `_search_regex` must return group 1 even when
    /// it captures an empty string (i.e. when `group(1)` exists but is empty).
    ///
    /// Before the fix the loop skipped `None`/empty groups and fell through to
    /// group 0, diverging from yt-dlp's `m.group(1)` unconditional return.
    ///
    /// Pattern `(a)(b)?` against `"a"`:
    ///   - group 1 = "a", group 2 = None (optional, didn't match)
    ///   - expected: "a" (group 1)
    #[tokio::test]
    async fn search_regex_group1_wins_over_group0_regression() {
        let mut c = ctx();
        let r = c
            .search_regex(
                r"(a)(b)?".to_string(),
                "a".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::empty(),
            )
            .await;
        assert_eq!(r, Some("a".to_string()), "group 1 must be returned");
    }

    /// Regression guard for L2 (empty capture): pattern `(x)?(y)` against `"y"`:
    ///   - group 1 did not participate (optional `x` not present) → captured empty/None
    ///   - group 2 = "y"
    ///   - yt-dlp returns `m.group(1)` which is `""` (empty string)
    ///   - before the fix: old loop skipped the non-participating group and returned "y" (group 2)
    ///
    /// Note: in the `regex` crate, a non-participating optional group returns
    /// `None` from `m.get(1)`. Our implementation maps that to `""` to match
    /// yt-dlp's `m.group(1)` returning `""` for a non-participating group in Python's re.
    #[tokio::test]
    async fn search_regex_returns_empty_string_for_non_participating_group1() {
        let mut c = ctx();
        let r = c
            .search_regex(
                r"(x)?(y)".to_string(),
                "y".to_string(),
                crate::bindings::rdlp::plugin::host_extract_helpers::RegexFlags::empty(),
            )
            .await;
        // group 1 did not participate → empty string (not "y" from group 2)
        assert_eq!(
            r,
            Some(String::new()),
            "non-participating group 1 must yield empty string, not group 2's value"
        );
    }

    /// Sanity check: no capture groups → return the whole match (group 0).
    #[tokio::test]
    async fn search_regex_no_groups_returns_whole_match() {
        let mut c = ctx();
        let r = c
            .search_regex(
                r"\d+".to_string(),
                "abc 42 def".to_string(),
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

    #[tokio::test]
    async fn html_search_meta_property_form() {
        let mut c = ctx();
        let r = c
            .html_search_meta(
                "og:title".to_string(),
                r#"<meta property="og:title" content="Hello"/>"#.to_string(),
            )
            .await;
        assert_eq!(r, Some("Hello".to_string()));
    }

    #[tokio::test]
    async fn html_search_meta_name_form() {
        let mut c = ctx();
        let r = c
            .html_search_meta(
                "description".to_string(),
                r#"<meta name="description" content="Page text"/>"#.to_string(),
            )
            .await;
        assert_eq!(r, Some("Page text".to_string()));
    }

    #[tokio::test]
    async fn html_search_meta_missing_returns_none() {
        let mut c = ctx();
        let r = c
            .html_search_meta("nope".to_string(), "<html></html>".to_string())
            .await;
        assert_eq!(r, None);
    }

    /// Regression guard for L3: `_html_search_meta` must not truncate content
    /// at a single-quote when the content is enclosed in double quotes.
    ///
    /// Before the fix `[^"']*` stopped at the first `'` inside `"a'b"`,
    /// returning `"a"` instead of `"a'b"`.
    #[tokio::test]
    async fn html_search_meta_mixed_quote_content_not_truncated_regression() {
        let mut c = ctx();
        // content is double-quoted but contains an apostrophe
        let r = c
            .html_search_meta(
                "x".to_string(),
                r#"<meta name="x" content="a'b">"#.to_string(),
            )
            .await;
        assert_eq!(
            r,
            Some("a'b".to_string()),
            "content containing single-quote inside double-quoted attribute must not be truncated"
        );
    }

    /// Companion: content in single quotes may contain a double-quote.
    #[tokio::test]
    async fn html_search_meta_double_quote_inside_single_quoted_content() {
        let mut c = ctx();
        let r = c
            .html_search_meta(
                "x".to_string(),
                r#"<meta name='x' content='say "hello"'>"#.to_string(),
            )
            .await;
        assert_eq!(r, Some(r#"say "hello""#.to_string()));
    }

    #[tokio::test]
    async fn og_search_property_extracts_title() {
        let mut c = ctx();
        let r = c
            .og_search_property(
                "title".to_string(),
                r#"<meta property="og:title" content="My Video"/>"#.to_string(),
            )
            .await;
        assert_eq!(r, Some("My Video".to_string()));
    }

    #[tokio::test]
    async fn og_search_property_extracts_image() {
        let mut c = ctx();
        let r = c
            .og_search_property(
                "image".to_string(),
                r#"<meta property="og:image" content="https://x.com/y.jpg"/>"#.to_string(),
            )
            .await;
        assert_eq!(r, Some("https://x.com/y.jpg".to_string()));
    }

    #[tokio::test]
    async fn og_search_property_unescapes_entities() {
        let mut c = ctx();
        let r = c
            .og_search_property(
                "title".to_string(),
                r#"<meta property="og:title" content="A &amp; B"/>"#.to_string(),
            )
            .await;
        assert_eq!(r, Some("A & B".to_string()));
    }

    #[tokio::test]
    async fn og_search_property_extracts_unquoted_value() {
        let mut c = ctx();
        let r = c
            .og_search_property(
                "image".to_string(),
                r#"<meta property="og:image" content=https://x.com/y.jpg />"#.to_string(),
            )
            .await;
        assert_eq!(r, Some("https://x.com/y.jpg".to_string()));
    }

    #[tokio::test]
    async fn rta_search_official_meta_returns_18() {
        let mut c = ctx();
        let r = c
            .rta_search(r#"<meta name="rating" content="RTA-5042-1996-1400-1577-RTA">"#.to_string())
            .await;
        assert_eq!(r, Some(18));
    }

    #[tokio::test]
    async fn rta_search_2257_marker_returns_18() {
        let mut c = ctx();
        let r = c
            .rta_search("footer text > 18 U.S.C. § 2257 statement".to_string())
            .await;
        assert_eq!(r, Some(18));
    }

    #[tokio::test]
    async fn rta_search_no_marker_returns_none() {
        let mut c = ctx();
        let r = c
            .rta_search("<html><body>nothing</body></html>".to_string())
            .await;
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn search_json_extracts_simple_object() {
        let mut c = ctx();
        let r = c
            .search_json(
                r"var data\s*=".to_string(),
                ";".to_string(),
                r#"var data = {"key": "value"};"#.to_string(),
            )
            .await;
        assert_eq!(r, Some(r#"{"key": "value"}"#.to_string()));
    }

    #[tokio::test]
    async fn search_json_handles_nested() {
        let mut c = ctx();
        let r = c
            .search_json(
                r"var x\s*=".to_string(),
                ";".to_string(),
                r#"var x = {"a": {"b": [1,2,3]}};"#.to_string(),
            )
            .await;
        assert_eq!(r, Some(r#"{"a": {"b": [1,2,3]}}"#.to_string()));
    }

    #[tokio::test]
    async fn search_json_no_match_returns_none() {
        let mut c = ctx();
        let r = c
            .search_json(
                r"NOPE".to_string(),
                "".to_string(),
                "irrelevant".to_string(),
            )
            .await;
        assert_eq!(r, None);
    }

    #[tokio::test]
    async fn extract_m3u8_returns_formats_via_fixture() {
        use crate::bindings::rdlp::plugin::host_extract_helpers::Host as _;
        use crate::host::fetch_fixtures::{FetchFixtures, FixtureResponse};
        use std::sync::Arc;

        let mut c = ctx();
        let master = "\
#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n\
hi.m3u8\n";
        let fixtures = Arc::new(FetchFixtures::new().with(
            "https://x.com/master.m3u8",
            FixtureResponse::ok(master.as_bytes().to_vec()),
        ));
        c.fetch = Some(crate::host::fetch::FetchCtx {
            client: rdlp_http::wreq::Client::builder()
                .build()
                .expect("test client"),
            fixtures: Some(fixtures),
        });
        let opts = crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options {
            ext: None,
            protocol: None,
            m3u8_id: None,
            fatal: true,
        };
        let r = c
            .extract_m3u8(
                "https://x.com/master.m3u8".to_string(),
                "vid".to_string(),
                opts,
            )
            .await
            .unwrap();
        assert_eq!(r.formats.len(), 1);
        assert_eq!(r.formats[0].width, Some(1920));
    }

    #[tokio::test]
    async fn extract_json_ld_returns_typed_video() {
        let mut c = ctx();
        let html = r#"<script type="application/ld+json">
        {"@type":"VideoObject","name":"Test","description":"d","thumbnailUrl":"https://x/t.jpg","duration":"PT5M","uploadDate":"2024-01-01"}
        </script>"#;
        let r = c.extract_json_ld(html.to_string()).await.unwrap();
        assert_eq!(r.title, Some("Test".to_string()));
        assert_eq!(r.duration, Some(300));
    }

    #[tokio::test]
    async fn extract_m3u8_non_fatal_swallows_fetch_failure() {
        // No FetchCtx installed → fetch returns Err("fetch capability not granted").
        // fatal=false should swallow the error and return an empty extraction.
        let mut c = ctx();
        let opts = crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options {
            ext: None,
            protocol: None,
            m3u8_id: None,
            fatal: false,
        };
        let r = c
            .extract_m3u8(
                "https://x.com/never-resolved.m3u8".to_string(),
                "vid".to_string(),
                opts,
            )
            .await
            .unwrap();
        assert_eq!(r.formats.len(), 0);
        assert_eq!(r.subtitles.len(), 0);
    }
}
