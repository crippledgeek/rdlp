//! `host:extract-helpers` capability — Slice-2.5 host-side helpers
//! delegating to rdlp's existing extractor primitives. Plugins call
//! these via WIT bindings; the Python compat shim's I/O methods become
//! 2-line passthroughs over them.
//!
//! All functions sync (pure CPU) except `extract_m3u8` which fetches
//! via the existing `host:fetch` wreq client.

// Lints below are from the new per-crate pedantic/nursery config; these
// pre-existing patterns are accepted for now — addressed in a separate pass.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::needless_raw_string_hashes,
    clippy::too_long_first_doc_paragraph,
    clippy::expect_used,
    clippy::missing_errors_doc
)]

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

fn build_url_with_query(base_url: String, query: &[(String, String)]) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    if query.is_empty() {
        return base_url;
    }
    let qs: String = query
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, NON_ALPHANUMERIC),
                utf8_percent_encode(v, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let sep = if base_url.contains('?') { '&' } else { '?' };
    format!("{base_url}{sep}{qs}")
}

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
            if let Some(m) = re.captures(&html)
                && let Some(s) = (1..m.len()).find_map(|i| {
                    m.get(i)
                        .map(|g| html_escape::decode_html_entities(g.as_str()).into_owned())
                })
            {
                return Some(s);
            }
        }
        None
    }

    async fn rta_search(&mut self, html: String) -> Option<u8> {
        // Delegates to the shared host+generic-extractor RTA detector (#497)
        // — mirrors yt-dlp's `_rta_search` (common.py:1520-1539).
        rdlp_extractor::base::common::age_rating::rta_search(&html)
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
        fetch: crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions,
    ) -> Result<
        crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Extraction,
        crate::bindings::rdlp::plugin::host_fetch::FetchError,
    > {
        use crate::bindings::rdlp::plugin::host_extract_helpers::{
            ExtractHelpersSubtitle, M3u8Extraction, M3u8Format,
        };
        use crate::bindings::rdlp::plugin::host_fetch::{FetchError, Host as FetchHost, Request};

        let result: Result<M3u8Extraction, FetchError> = async {
            let url = build_url_with_query(url, &fetch.query);
            let req = Request {
                url: url.clone(),
                method: if fetch.body.is_some() {
                    "POST".into()
                } else {
                    "GET".into()
                },
                headers: fetch.headers.clone(),
                body: fetch.body.clone(),
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

    async fn extract_mpd(
        &mut self,
        url: String,
        _video_id: String,
        opts: crate::bindings::rdlp::plugin::host_extract_helpers::MpdOptions,
        fetch: crate::bindings::rdlp::plugin::host_extract_helpers::FetchOptions,
    ) -> Result<
        crate::bindings::rdlp::plugin::host_extract_helpers::MpdExtraction,
        crate::bindings::rdlp::plugin::host_fetch::FetchError,
    > {
        use crate::bindings::rdlp::plugin::host_extract_helpers::{
            ExtractHelpersSubtitle, MpdExtraction, MpdFormat, MpdFragment,
        };
        use crate::bindings::rdlp::plugin::host_fetch::{FetchError, Host as FetchHost, Request};
        use rdlp_extractor::base::common::dash::{DashExpansion, expand_dash_representations};
        use url::Url;

        let result: Result<MpdExtraction, FetchError> = async {
            let url = build_url_with_query(url, &fetch.query);
            let req = Request {
                url: url.clone(),
                method: if fetch.body.is_some() {
                    "POST".into()
                } else {
                    "GET".into()
                },
                headers: fetch.headers.clone(),
                body: fetch.body.clone(),
                timeout_ms: Some(30_000),
            };
            let resp = self.fetch(req).await?;
            let body =
                String::from_utf8(resp.body).map_err(|e| FetchError::Network(e.to_string()))?;
            let base = Url::parse(&url).map_err(|e| FetchError::Network(e.to_string()))?;
            let DashExpansion { formats, subtitles } = expand_dash_representations(&body, &base)
                .map_err(|e| FetchError::Network(format!("{e:#}")))?;

            let mpd_formats: Vec<MpdFormat> = formats
                .into_iter()
                .map(|f| MpdFormat {
                    format_id: f.format_id,
                    url: f.url,
                    ext: f.ext.clone(),
                    vcodec: f.vcodec.as_str().map(str::to_owned),
                    acodec: f.acodec.as_str().map(str::to_owned),
                    tbr: f.tbr.map(|v| v as f32),
                    width: f.width,
                    height: f.height,
                    fps: f.fps.map(|v| v as f32),
                    asr: f.asr,
                    language: f.language,
                    container: f.container,
                    manifest_url: Some(url.clone()),
                    fragment_base_url: f.fragment_base_url,
                    fragments: f
                        .fragments
                        .unwrap_or_default()
                        .into_iter()
                        .map(|fr| MpdFragment {
                            url: fr.url,
                            duration: fr.duration,
                            // `(start, end_exclusive)` convention mirrors
                            // `rdlp_types::Fragment.byte_range` exactly — both
                            // tuples are passed through verbatim. Plugins see
                            // the same end-exclusive semantics as the host's
                            // downloader (see WIT doc-comments on
                            // `mpd-fragment.byte-range` / `.init-byte-range`).
                            byte_range: fr.byte_range,
                            init_url: fr.init_url,
                            init_byte_range: fr.init_byte_range,
                        })
                        .collect(),
                })
                .collect();

            let mpd_subtitles: Vec<ExtractHelpersSubtitle> = subtitles
                .into_iter()
                .map(|s| ExtractHelpersSubtitle {
                    language: s.language.unwrap_or_default(),
                    url: s.url,
                    ext: Some(s.ext),
                })
                .collect();

            Ok(MpdExtraction {
                formats: mpd_formats,
                subtitles: mpd_subtitles,
            })
        }
        .await;

        match (result, opts.fatal) {
            (Ok(x), _) => Ok(x),
            (Err(e), true) => Err(e),
            (Err(_), false) => Ok(MpdExtraction {
                formats: vec![],
                subtitles: vec![],
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
mod tests;
