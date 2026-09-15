//! `host:extract-helpers` capability — Slice-2.5 host-side helpers
//! delegating to rdlp's existing extractor primitives. Plugins call
//! these via WIT bindings; the Python compat shim's I/O methods become
//! 2-line passthroughs over them.
//!
//! All functions sync (pure CPU) except `extract_m3u8` and `extract_mpd`,
//! which fetch via the `host:fetch` wreq client, and `expand_hls` /
//! `probe_format_sizes` (@since 0.5.1), which wrap the in-tree
//! `rdlp_extractor::hls` helpers over the plugin's granted `wreq::Client`
//! directly (those helpers take an `Arc<wreq::Client>`, not a `host:fetch`
//! request/response pair).

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

use std::sync::{Arc, LazyLock};

use crate::bindings::rdlp::plugin::host_extract_helpers::{
    FetchOptions, HlsFormat, HlsStreamFlags, RegexFlags, SizeProbe,
};
use crate::bindings::rdlp::plugin::host_fetch::{FetchError, Host as FetchHost, Request};
use crate::bindings::rdlp::plugin::types::Format as WitFormat;
use crate::host::fetch::FETCH_NOT_GRANTED;
use crate::instance::PluginStoreData;
use rdlp_http::wreq;
use wasmtime::component::Linker;

static RE_WHITESPACE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\s+").expect("valid whitespace pattern"));
static RE_BR: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\s?<\s?br\s?/?\s?>\s?").expect("valid <br> pattern"));
static RE_P: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<\s?/\s?p\s?>\s?<\s?p[^>]*>").expect("valid <p> pattern"));
static RE_TAGS: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"<.*?>").expect("valid tag-strip pattern"));

/// Plugin-facing manifest fetches share one timeout: yt-dlp's default socket
/// timeout is 20 s and a master playlist / MPD is a few KB, so 30 s leaves
/// headroom for a slow origin without letting a stalled one hold the
/// extractor for minutes. `host:fetch` clamps it to its own `MAX_TIMEOUT_MS`.
const MANIFEST_FETCH_TIMEOUT_MS: u32 = 30_000;

/// Upper bound on a plugin-supplied regex pattern, in bytes, checked before
/// compiling. Plugin patterns are third-party input, and `rust-regex-craft`
/// requires untrusted patterns to be bounded on both pattern length and
/// compiled size. 4 KiB holds because of WHICH patterns reach this path:
/// yt-dlp's giant literals — `_VALID_URL` up to ~65 KB (`peertube.py`
/// `_INSTANCES_RE`) — are compiled guest-side by the compat shim with
/// stdlib `re` (`info_extractor.py` `_match_valid_url`), and `_EMBED_REGEX`
/// has no host-side implementation at all. Only the `_search_regex` /
/// `_html_search_meta` / `_og_search_property` / `_search_json` patterns
/// are routed here, and those are short by construction (a value locator,
/// not a site-wide URL matcher). This bound alone is not sufficient — see
/// [`PLUGIN_REGEX_SIZE_LIMIT`].
const PLUGIN_REGEX_MAX_PATTERN_LEN: usize = 4096;

/// Cap on the compiled size of a plugin-supplied regex, passed to
/// `RegexBuilder::size_limit`; exceeding it fails the build with "Compiled
/// regex exceeds size limit". Compiled size is independent of pattern
/// length: Unicode `\w` spans ~140k codepoints, so the 6-byte `\w{50}`
/// compiles to ~2.4 MiB and `\w{200}` to ~10 MiB (measured, regex 1.12.3).
/// 4 MiB admits the largest we found reaching this path — `googledrive.py`'s
/// `"(\w{39})"` at ~1.9 MiB — with 2x headroom, and sits 2.5x below the
/// crate's 10 MiB default, which a plugin must not be able to spend freely.
const PLUGIN_REGEX_SIZE_LIMIT: usize = 4 << 20;

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

/// The one place a plugin-supplied pattern is compiled: every helper that
/// derives a regex from plugin input goes through here so both bounds
/// ([`PLUGIN_REGEX_MAX_PATTERN_LEN`], [`PLUGIN_REGEX_SIZE_LIMIT`]) apply
/// uniformly. An over-long pattern is reported as a syntax error rather
/// than compiled and then rejected — the length check exists to skip the
/// compile.
///
/// Every helper turns a failed build into `None`, which the compat shim
/// surfaces as "Unable to extract …" — indistinguishable from a genuine
/// no-match. A bound refusal therefore warns on the plugin's own log
/// target (`log_target`, the same one `host:log` writes to) with the byte
/// length and the limit; the pattern text is plugin-controlled and may be
/// huge, so it is never logged.
fn build_regex(
    pattern: &str,
    flags: RegexFlags,
    log_target: &str,
) -> Result<regex::Regex, regex::Error> {
    if pattern.len() > PLUGIN_REGEX_MAX_PATTERN_LEN {
        log::warn!(
            target: log_target,
            "refused plugin regex: pattern length {} B exceeds the {PLUGIN_REGEX_MAX_PATTERN_LEN} B limit",
            pattern.len()
        );
        return Err(regex::Error::Syntax(format!(
            "plugin pattern is {} bytes; limit is {PLUGIN_REGEX_MAX_PATTERN_LEN}",
            pattern.len()
        )));
    }
    let mut builder = regex::RegexBuilder::new(pattern);
    builder.size_limit(PLUGIN_REGEX_SIZE_LIMIT);
    builder.case_insensitive(flags.contains(RegexFlags::IGNORE_CASE));
    builder.multi_line(flags.contains(RegexFlags::MULTILINE));
    builder.dot_matches_new_line(flags.contains(RegexFlags::DOTALL));
    builder.ignore_whitespace(flags.contains(RegexFlags::VERBOSE));
    let built = builder.build();
    if let Err(regex::Error::CompiledTooBig(_)) = &built {
        log::warn!(
            target: log_target,
            "refused plugin regex: compiled size exceeds the {PLUGIN_REGEX_SIZE_LIMIT} B limit (pattern {} B)",
            pattern.len()
        );
    }
    built
}

/// A non-fatal extraction failure yields the empty result instead of an
/// error — yt-dlp's `fatal=False` semantics for `_extract_m3u8_formats` /
/// `_extract_mpd_formats`.
fn empty_unless_fatal<T>(
    result: Result<T, FetchError>,
    fatal: bool,
    empty: impl FnOnce() -> T,
) -> Result<T, FetchError> {
    match (result, fatal) {
        (Ok(x), _) => Ok(x),
        (Err(e), true) => Err(e),
        (Err(_), false) => Ok(empty()),
    }
}

impl PluginStoreData {
    /// Fetch a manifest through the plugin's own `host:fetch` capability
    /// (same SSRF gate, same body cap) and return `(url_fetched, body)`.
    /// The URL returned carries the appended query so the caller resolves
    /// relative manifest entries against what was actually requested.
    async fn fetch_manifest_text(
        &mut self,
        url: String,
        fetch: &FetchOptions,
    ) -> Result<(String, String), FetchError> {
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
            timeout_ms: Some(MANIFEST_FETCH_TIMEOUT_MS),
        };
        let resp = self.fetch(req).await?;
        let body = String::from_utf8(resp.body).map_err(|e| FetchError::Network(e.to_string()))?;
        Ok((url, body))
    }

    /// The plugin's granted fetch client as the `Arc<wreq::Client>` the HLS
    /// helpers take. Mirrors `fetch`'s "capability not granted" refusal so a
    /// plugin without `fetch` gets the same error from every I/O helper.
    fn hls_http_client(&self) -> Result<Arc<wreq::Client>, FetchError> {
        self.fetch
            .as_ref()
            .map(|c| Arc::new(c.client.clone()))
            .ok_or_else(|| FetchError::Network(FETCH_NOT_GRANTED.into()))
    }
}

/// Apply `fetch-options` to one `expand-hls` / `probe-format-sizes` seed
/// before it reaches `rdlp_extractor::hls`.
///
/// `query` is appended to the seed's `url` via the same
/// [`build_url_with_query`] `extract_m3u8`/`extract_mpd` use. `headers`
/// become the row's `http_headers` so the expander (and later the
/// downloader) send them on every playlist/segment request — the same
/// channel in-tree extractors use for a required `Referer`. Two divergences
/// from `extract_m3u8`/`extract_mpd`'s use of the same `fetch-options`
/// record, both because these two imports never issue a request of their
/// own (they hand the URL to `rdlp_extractor::hls`, which always GETs it):
/// `body` is IGNORED — there is no POST here for a body to attach to — and
/// duplicate header keys COLLAPSE (last one wins), because
/// `Format::http_headers` is a `HashMap`, not the ordered
/// `Vec<(String, String)>` `host:fetch`'s `Request` uses, so duplicates
/// cannot be preserved verbatim across this boundary the way they can there.
fn apply_fetch_headers(mut f: rdlp_types::Format, fetch: &FetchOptions) -> rdlp_types::Format {
    if !fetch.query.is_empty() {
        f.url = build_url_with_query(f.url, &fetch.query);
    }
    if !fetch.headers.is_empty() {
        f.http_headers
            .get_or_insert_with(std::collections::HashMap::new)
            .extend(fetch.headers.iter().cloned());
    }
    f
}

/// Convert an expanded `rdlp_types::Format` (post `expand_hls_in_place`) to
/// the WIT `hls-format` record. Only called on rows that already carry
/// `fragments` — the caller filters those out first.
fn hls_format_to_wit(f: &rdlp_types::Format) -> HlsFormat {
    let fragments = f
        .fragments
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(crate::convert::fragment_to_wit)
        .collect();
    HlsFormat {
        duration: f.duration,
        filesize_approx: f.filesize_approx,
        language: f.language.clone(),
        fragments,
        format: crate::convert::format_to_wit(f),
    }
}

/// Strip HTML tags and collapse whitespace.
///
/// SECOND IMPLEMENTATION, deliberately: the same contract exists in Python at
/// `tools/ytdlp-compat/rdlp_ytdlp_compat/_utils.py::clean_html`. They cannot
/// be merged — a Python plugin calls that one directly, a WASM plugin reaches
/// this one over WIT — so the four regexes below are duplicated by necessity
/// and must stay identical to it. Both were changed together to stop
/// unescaping; a change to one without the other is a drift bug.
///
/// Shaped after yt-dlp's `clean_html` (`utils/_utils.py:527-540`) with ONE
/// deliberate divergence: yt-dlp unescapes entities here, and this does not.
/// Whatever a plugin returns as a display field is decoded downstream by
/// `InfoDict::decode_text_fields`, so decoding here too would be two
/// independent single-pass decodes — which compose into exactly the
/// double-decode the boundary exists to avoid, taking a literal `&amp;lt;`
/// to `<`. Parity with yt-dlp is the means here; decoding once is the end.
fn clean_html(html: &str) -> String {
    let collapsed = RE_WHITESPACE.replace_all(html, " ");
    let no_br = RE_BR.replace_all(&collapsed, "\n");
    let no_p = RE_P.replace_all(&no_br, "\n");
    RE_TAGS.replace_all(&no_p, "").trim().to_string()
}

/// Wire `host:extract-helpers` into a linker.
pub fn add_to_linker(linker: &mut Linker<PluginStoreData>) -> wasmtime::Result<()> {
    crate::bindings::rdlp::plugin::host_extract_helpers::add_to_linker(linker, |s| s)
}

impl crate::bindings::rdlp::plugin::host_extract_helpers::Host for PluginStoreData {
    fn search_regex(
        &mut self,
        pattern: String,
        haystack: String,
        re_flags: RegexFlags,
    ) -> Option<String> {
        let pat = build_regex(&pattern, re_flags, &self.log_target).ok()?;
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

    fn html_search_regex(
        &mut self,
        pattern: String,
        haystack: String,
        re_flags: RegexFlags,
    ) -> Option<String> {
        let raw = self.search_regex(pattern, haystack, re_flags)?;
        Some(clean_html(&raw))
    }

    fn html_search_meta(&mut self, name: String, html: String) -> Option<String> {
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
            let re = build_regex(pat, RegexFlags::IGNORE_CASE, &self.log_target).ok()?;
            if let Some(m) = re.captures(&html)
                && let Some(g) = m.get(1)
            {
                return Some(g.as_str().to_string());
            }
        }
        None
    }

    fn og_search_property(&mut self, prop: String, html: String) -> Option<String> {
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
            let Ok(re) = build_regex(
                pat,
                RegexFlags::IGNORE_CASE | RegexFlags::DOTALL,
                &self.log_target,
            ) else {
                continue;
            };
            if let Some(m) = re.captures(&html)
                && let Some(s) = (1..m.len()).find_map(|i| {
                    // Not decoded here — see `clean_html` for why the
                    // boundary is the only decoder.
                    m.get(i).map(|g| g.as_str().to_owned())
                })
            {
                return Some(s);
            }
        }
        None
    }

    fn rta_search(&mut self, html: String) -> Option<u8> {
        // Delegates to the shared host+generic-extractor RTA detector (#497)
        // — mirrors yt-dlp's `_rta_search` (common.py:1520-1539).
        rdlp_extractor::base::common::age_rating::rta_search(&html)
    }

    fn search_json(
        &mut self,
        start_pattern: String,
        end_pattern: String,
        haystack: String,
    ) -> Option<String> {
        // Mirrors yt-dlp's `_search_json` brace-balanced extraction.
        // Default contains-pattern is `{(?s:.+)}` — greedy, allows nesting.
        let full = format!(r"(?:{start_pattern})\s*(?P<json>\{{(?s:.+)\}})\s*(?:{end_pattern})");
        let re = build_regex(&full, RegexFlags::empty(), &self.log_target).ok()?;
        let cap = re.captures(&haystack)?;
        let json = cap.name("json")?.as_str();
        Some(json.to_string())
    }

    async fn extract_m3u8(
        &mut self,
        url: String,
        _video_id: String,
        opts: crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Options,
        fetch: FetchOptions,
    ) -> Result<crate::bindings::rdlp::plugin::host_extract_helpers::M3u8Extraction, FetchError>
    {
        use crate::bindings::rdlp::plugin::host_extract_helpers::{
            ExtractHelpersSubtitle, M3u8Extraction, M3u8Format,
        };

        let result: Result<M3u8Extraction, FetchError> = async {
            let (url, body) = self.fetch_manifest_text(url, &fetch).await?;
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
                    // `M3u8Format.vcodec`/`.acodec` are plain `String` at the
                    // WIT boundary (the wire format is unaffected by #642's
                    // `Rfc6381Codec` newtype), so the validated value is
                    // stringified here rather than propagated as a typed field.
                    vcodec: v.vcodec.map(|c| c.to_string()),
                    acodec: v.acodec.map(|c| c.to_string()),
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

        empty_unless_fatal(result, opts.fatal, || M3u8Extraction {
            formats: vec![],
            subtitles: vec![],
        })
    }

    async fn extract_mpd(
        &mut self,
        url: String,
        _video_id: String,
        opts: crate::bindings::rdlp::plugin::host_extract_helpers::MpdOptions,
        fetch: FetchOptions,
    ) -> Result<crate::bindings::rdlp::plugin::host_extract_helpers::MpdExtraction, FetchError>
    {
        use crate::bindings::rdlp::plugin::host_extract_helpers::{
            ExtractHelpersSubtitle, MpdExtraction, MpdFormat,
        };
        use rdlp_extractor::base::common::dash::{DashExpansion, expand_dash_representations};
        use url::Url;

        let result: Result<MpdExtraction, FetchError> = async {
            let (url, body) = self.fetch_manifest_text(url, &fetch).await?;
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
                    tbr: f.tbr.map(crate::convert::narrow_f64),
                    width: f.width,
                    height: f.height,
                    fps: f.fps.map(crate::convert::narrow_f64),
                    asr: f.asr,
                    language: f.language,
                    container: f.container,
                    manifest_url: Some(url.clone()),
                    fragment_base_url: f.fragment_base_url,
                    fragments: f
                        .fragments
                        .unwrap_or_default()
                        .into_iter()
                        .map(crate::convert::fragment_to_wit)
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

        empty_unless_fatal(result, opts.fatal, || MpdExtraction {
            formats: vec![],
            subtitles: vec![],
        })
    }

    fn extract_json_ld(
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

    // See the WIT doc comment on `expand-hls` for the contract this wraps
    // unchanged (`expand_hls_in_place`): drop-not-fail per row, SSRF-gated,
    // capped. The only `FetchError` this can return is
    // `Network(FETCH_NOT_GRANTED)` from `hls_http_client` above — nothing
    // downstream of it is fallible at this boundary.
    async fn expand_hls(
        &mut self,
        formats: Vec<WitFormat>,
        fetch: FetchOptions,
    ) -> Result<Vec<HlsFormat>, FetchError> {
        let http = self.hls_http_client()?;
        let seeds: Vec<rdlp_types::Format> = formats
            .into_iter()
            .map(|w| apply_fetch_headers(crate::convert::format_from_wit(w), &fetch))
            .collect();
        let expanded = rdlp_extractor::hls::expand_hls_in_place(seeds, http).await;
        Ok(expanded
            .iter()
            .filter(|f| f.fragments.is_some())
            .map(hls_format_to_wit)
            .collect())
    }

    // See the WIT doc comment on `probe-format-sizes`. The WIT `format`
    // record carries no `fragments` field, so `format_from_wit` always
    // produces `rdlp_types::Format { fragments: None, .. }` here — the
    // in-tree fragment-reuse short-circuit inside
    // `detect_format_sizes_inner` (fragments already present → skip
    // re-expansion) can NEVER trigger from this import. Every call re-fetches
    // and re-parses each row's own playlist URL.
    async fn probe_format_sizes(
        &mut self,
        formats: Vec<WitFormat>,
        fetch: FetchOptions,
    ) -> Result<SizeProbe, FetchError> {
        let http = self.hls_http_client()?;
        let seeds: Vec<rdlp_types::Format> = formats
            .into_iter()
            .map(|w| apply_fetch_headers(crate::convert::format_from_wit(w), &fetch))
            .collect();
        // No `ExtractionContext` exists at the plugin host, so `SizeProbeEnv`
        // needs a `Config` from somewhere. `Config::default()` is fine
        // because nothing this lazy probe does reads it: `detect_sizes` is
        // hardcoded `false` by `detect_format_sizes_lazy_in`, so the
        // non-HLS HEAD-probe branch (the only reader of
        // `hls_head_probe_timeout`) never runs. The only `Config` field this
        // path is sensitive to is `verbose` (debug-log detail), which
        // defaults to `false` either way.
        let config = rdlp_types::Config::default();
        let probe = rdlp_extractor::hls::SizeProbeEnv {
            http_client: http,
            config: &config,
            extractor_name: &self.plugin_name,
        };
        let (out, flags) = rdlp_extractor::hls::detect_format_sizes_lazy_in(seeds, &probe).await;
        Ok(SizeProbe {
            formats: out.iter().map(crate::convert::format_to_wit).collect(),
            stream_flags: HlsStreamFlags {
                is_live: flags.is_live,
                has_any_drm: flags.has_any_drm,
            },
        })
    }
}

#[cfg(test)]
mod tests;
