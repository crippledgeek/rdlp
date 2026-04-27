//! ABXXX extractor.
//!
//! ABXXX is a KVS (Kernel Video Sharing) tube site that delivers its player
//! configuration via JSON XHR endpoints instead of inline `flashvars`.
//!
//! ## Extraction flow
//! 1. `GET /api/videofile.php?video_id={id}&lifetime=86400` — JSON array of
//!    `{format, video_url, ...}`. Each `video_url` is obfuscated (Cyrillic
//!    homoglyphs + comma-split base64); `decode::decode_video_url` recovers
//!    `(path, query)`. The canonical playable URL is
//!    `https://abxxx.com{path}?{query}`; the CDN edge 302s to a signed
//!    `https://ahcdn.abxxx.com/key=…` URL the downloader follows.
//! 2. `GET /api/json/video/{lifetime}/{e6}/{e3}/{id}.json` — rich per-video
//!    metadata (real title, description, structured models + categories,
//!    statistics, post date, full-resolution thumbnails). See
//!    `metadata::lookup_endpoint`.
//! 3. *Best-effort* `GET /api/videos2.php?…&s={slug-words}` — slug-based
//!    search to pull the row with `file_dimensions` + `file_formats`,
//!    which gives the playable variant's `width`/`height`/`filesize`.
//!    A search miss is non-fatal: the extractor still ships a usable
//!    Format without dimensions.
//!
//! Step 2 also shapes `humanize_slug` as a deterministic fallback when the
//! lookup endpoint is unavailable (e.g. the page exists but the JSON cache
//! 404s, which the live site itself handles by retrying).

mod patterns;
mod search;

use async_trait::async_trait;
use log::debug;
use rdlp_core::{ExtractionContext, InfoExtractor, RdlpError, Result};
use rdlp_types::{DownloadProtocol, Format, InfoDict};
use regex::Regex;
use serde_json::Value;

use crate::base::common::BaseExtractor;
use crate::base::kvs::{api as kvs_api, file_formats as kvs_file_formats, url_obfuscation};

const ABXXX_BASE_URL: &str = "https://abxxx.com";
const ABXXX_NAME: &str = "ABXXX";
const ABXXX_PRIORITY: i32 = 50;
/// Lifetime parameter (seconds) the site itself uses when calling videofile.php.
const VIDEOFILE_LIFETIME: u32 = 86_400;
/// Per-page result count used by the slug-driven enrichment search.
const SLUG_SEARCH_RESULTS_PER_PAGE: u32 = 60;

/// ABXXX video extractor.
#[derive(Default)]
pub struct AbxxxExtractor;

impl AbxxxExtractor {
    /// Create a new ABXXX extractor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// Build the videofile.php endpoint URL for `video_id`.
fn videofile_endpoint(video_id: &str) -> String {
    format!("{ABXXX_BASE_URL}/api/videofile.php?video_id={video_id}&lifetime={VIDEOFILE_LIFETIME}")
}

/// Build the slug-driven search URL used to look up file format details
/// for a single video page.
///
/// The search query is the slug with hyphens replaced by spaces. The
/// canonical row is then identified by matching `video_id` in the result
/// set, not by relying on positional order — see
/// [`crate::base::kvs::file_formats::parse_search_for_file_formats`].
fn slug_search_endpoint(slug: &str) -> String {
    let query = slug.replace('-', " ");
    let q = urlencoding::encode(&query);
    format!(
        "{ABXXX_BASE_URL}/api/videos2.php?params=0/str/relevance/{count}/search..1.all..&s={q}",
        count = SLUG_SEARCH_RESULTS_PER_PAGE,
        q = q,
    )
}

/// Build the conventional KVS preview thumbnail URL for `video_id`.
///
/// ABXXX groups screenshots into 1000-id buckets:
/// `/contents/videos_screenshots/{floor(id/1000)*1000}/{id}/preview.jpg`.
fn thumbnail_url(video_id: &str) -> Option<String> {
    let id: u64 = video_id.parse().ok()?;
    let bucket = (id / 1000) * 1000;
    Some(format!(
        "{ABXXX_BASE_URL}/contents/videos_screenshots/{bucket}/{id}/preview.jpg"
    ))
}

/// Humanize a URL slug into a display title.
///
/// `excogi-katie-carmine-in-hd` → `Excogi Katie Carmine In Hd`. The actual
/// page never exposes a server-rendered title, so the slug is the only signal.
fn humanize_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse `d=<seconds>&br=<bitrate>&...` query string into best-effort metadata.
fn parse_query_metadata(query: &str) -> (Option<f64>, Option<u32>) {
    let mut duration = None;
    let mut bitrate = None;
    for pair in query.split('&') {
        let mut split = pair.splitn(2, '=');
        let (Some(k), Some(v)) = (split.next(), split.next()) else {
            continue;
        };
        match k {
            "d" => duration = v.parse::<f64>().ok(),
            "br" => bitrate = v.parse::<u32>().ok(),
            _ => {}
        }
    }
    (duration, bitrate)
}

/// Convert one `videofile.php` JSON entry into a `Format`.
fn entry_to_format(video_id: &str, idx: usize, entry: &Value) -> Option<Format> {
    let encoded = entry.get("video_url").and_then(|v| v.as_str())?;
    let (path, query) = url_obfuscation::decode_video_url(encoded)?;
    let ext_dot = entry
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or(".mp4");
    let ext = ext_dot.trim_start_matches('.');

    let url = match query.as_deref() {
        Some(q) if !q.is_empty() => format!("{ABXXX_BASE_URL}{path}?{q}"),
        _ => format!("{ABXXX_BASE_URL}{path}"),
    };

    let format_id = format!("{ext}-{idx}-{video_id}");
    let mut f = Format::new(format_id, url, ext, DownloadProtocol::Https);
    f.container = Some(ext.to_string());
    f.vcodec = Some("h264".to_string());
    f.acodec = Some("aac".to_string());
    // The KVS API returns a single muxed stream representing the best variant
    // available. Marking it explicitly avoids the UI rendering the QUALITY
    // column as "Unknown".
    f.format_note = Some("best".to_string());

    if let Some(q) = query.as_deref() {
        let (_d, br) = parse_query_metadata(q);
        f.tbr = br.map(f64::from);
    }
    Some(f)
}

/// Apply file-format details from the slug-search enrichment to the single
/// playable Format the videofile.php endpoint returned.
fn apply_file_format_info(format: &mut Format, info: kvs_file_formats::KvsFileFormatInfo) {
    if format.width.is_none() {
        format.width = info.width;
    }
    if format.height.is_none() {
        format.height = info.height;
    }
    if format.filesize.is_none() {
        format.filesize = info.filesize;
    }
    if let Some(h) = info.height {
        format.format_note = Some(format!("{h}p"));
    }
}

#[async_trait]
impl InfoExtractor for AbxxxExtractor {
    fn name(&self) -> &str {
        ABXXX_NAME
    }

    fn valid_url(&self) -> &Regex {
        &patterns::ABXXX_URL_PATTERN
    }

    fn priority(&self) -> i32 {
        ABXXX_PRIORITY
    }

    fn suitable(&self, url: &str) -> bool {
        patterns::is_suitable(url)
    }

    async fn extract(&self, url: &str, ctx: &ExtractionContext) -> Result<InfoDict> {
        let video_id = patterns::extract_video_id(url).ok_or_else(|| RdlpError::Extraction {
            message: format!("Could not extract video id from URL: {url}"),
            url: Some(url.to_string()),
        })?;
        let slug = patterns::extract_slug(url);

        // 1) Fetch the playable URL via videofile.php (load-bearing).
        let api_url = videofile_endpoint(&video_id);
        debug!("ABXXX: fetching videofile.php endpoint: {api_url}");
        let body = BaseExtractor::fetch_webpage_with_headers(
            &api_url,
            &[
                ("Referer", url),
                ("Accept", "application/json, text/plain, */*"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            ctx,
        )
        .await?;

        let parsed: Value = serde_json::from_str(&body).map_err(|e| RdlpError::Extraction {
            message: format!("videofile.php returned non-JSON body ({e}): {body}"),
            url: Some(api_url.clone()),
        })?;
        let entries = parsed.as_array().ok_or_else(|| RdlpError::Extraction {
            message: format!("videofile.php payload is not a JSON array: {parsed}"),
            url: Some(api_url.clone()),
        })?;

        let mut formats = Vec::with_capacity(entries.len());
        let mut duration_from_query: Option<f64> = None;
        for (idx, entry) in entries.iter().enumerate() {
            match entry_to_format(&video_id, idx, entry) {
                Some(f) => {
                    if duration_from_query.is_none()
                        && let Some(enc) = entry.get("video_url").and_then(|v| v.as_str())
                        && let Some((_, Some(q))) = url_obfuscation::decode_video_url(enc)
                    {
                        duration_from_query = parse_query_metadata(&q).0;
                    }
                    formats.push(f);
                }
                None => debug!("ABXXX: skipping unparseable videofile entry #{idx}: {entry}"),
            }
        }

        if formats.is_empty() {
            return Err(RdlpError::Extraction {
                message: format!("No usable formats decoded from videofile.php for {video_id}"),
                url: Some(api_url),
            });
        }

        // 2) Enrich with the per-video lookup endpoint (best-effort).
        let lookup = if let Some(lookup_url) =
            kvs_api::video_lookup_endpoint(ABXXX_BASE_URL, &video_id)
        {
            debug!("ABXXX: fetching lookup endpoint: {lookup_url}");
            match BaseExtractor::fetch_webpage_with_headers(
                &lookup_url,
                &[
                    ("Referer", url),
                    ("Accept", "application/json, text/plain, */*"),
                    ("X-Requested-With", "XMLHttpRequest"),
                ],
                ctx,
            )
            .await
            {
                Ok(body) => kvs_api::parse_video_lookup(&body).unwrap_or_default(),
                Err(e) => {
                    debug!(
                        "ABXXX: lookup endpoint failed ({e}); falling back to slug-derived metadata"
                    );
                    kvs_api::KvsVideoMetadata::default()
                }
            }
        } else {
            kvs_api::KvsVideoMetadata::default()
        };

        // 3) Enrich format dimensions/filesize via slug-based search (best-effort).
        if let Some(slug_str) = slug.as_deref() {
            let search_url = slug_search_endpoint(slug_str);
            debug!("ABXXX: enriching formats via slug search: {search_url}");
            match BaseExtractor::fetch_webpage_with_headers(
                &search_url,
                &[
                    ("Referer", url),
                    ("Accept", "application/json, text/plain, */*"),
                    ("X-Requested-With", "XMLHttpRequest"),
                ],
                ctx,
            )
            .await
            {
                Ok(body) => {
                    if let Some(info) =
                        kvs_file_formats::parse_search_for_file_formats(&body, &video_id)
                    {
                        for f in &mut formats {
                            apply_file_format_info(f, info);
                        }
                    }
                }
                Err(e) => debug!(
                    "ABXXX: slug-search enrichment failed ({e}); shipping without dimensions"
                ),
            }
        }

        // 4) Compose the final InfoDict, preferring lookup-derived fields.
        let title = lookup
            .title
            .clone()
            .or_else(|| slug.as_deref().map(humanize_slug).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| format!("ABXXX video {video_id}"));

        let mut info = InfoDict::new(video_id.clone(), title, ABXXX_NAME, url);
        info.formats = formats;
        info.duration = lookup.duration.or(duration_from_query);
        info.description = lookup.description;
        info.thumbnail = lookup.thumbnail.or_else(|| thumbnail_url(&video_id));
        info.upload_date = lookup.upload_date;
        info.view_count = lookup.view_count;
        info.average_rating = lookup.average_rating;
        info.uploader = lookup.uploader;
        info.uploader_id = lookup.uploader_id;
        if !lookup.actors.is_empty() {
            info.actors = lookup.actors;
        }
        if !lookup.categories.is_empty() {
            info.categories = Some(lookup.categories);
        }
        info.age_limit = Some(18);
        info.propagate_duration();

        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extractor_metadata() {
        let e = AbxxxExtractor::new();
        assert_eq!(e.name(), "ABXXX");
        assert_eq!(e.priority(), 50);
    }

    #[test]
    fn url_routing() {
        let e = AbxxxExtractor::new();
        assert!(e.suitable("https://abxxx.com/video/129452/excogi-katie-carmine-in-hd/"));
        assert!(e.suitable("https://www.abxxx.com/video/1/"));
        assert!(!e.suitable("https://example.com/video/1/title/"));
    }

    #[test]
    fn slug_search_endpoint_replaces_hyphens_with_spaces() {
        let url = slug_search_endpoint("excogi-katie-carmine-in-hd");
        assert!(url.contains("&s=excogi%20katie%20carmine%20in%20hd"));
        assert!(url.contains("/search..1.all.."));
    }

    #[test]
    fn humanize_slug_handles_typical_kvs_slug() {
        assert_eq!(
            humanize_slug("excogi-katie-carmine-in-hd"),
            "Excogi Katie Carmine In Hd"
        );
    }

    #[test]
    fn humanize_slug_drops_empty_segments() {
        assert_eq!(humanize_slug("--foo---bar--"), "Foo Bar");
        assert_eq!(humanize_slug(""), "");
    }

    #[test]
    fn thumbnail_url_uses_1000_bucket() {
        assert_eq!(
            thumbnail_url("129452").as_deref(),
            Some("https://abxxx.com/contents/videos_screenshots/129000/129452/preview.jpg")
        );
        assert_eq!(
            thumbnail_url("42").as_deref(),
            Some("https://abxxx.com/contents/videos_screenshots/0/42/preview.jpg")
        );
    }

    #[test]
    fn parse_query_metadata_extracts_duration_and_bitrate() {
        let (d, br) = parse_query_metadata("d=5443&br=235&ti=1777247222");
        assert_eq!(d, Some(5443.0));
        assert_eq!(br, Some(235));
    }

    #[test]
    fn entry_to_format_builds_canonical_url() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let entry: Value = serde_json::json!({
            "format": ".mp4",
            "video_url": format!("{},{}",
                STANDARD.encode("/get_file/1/abc/0/42/42.mp4/"),
                STANDARD.encode("d=10&br=500"),
            ),
            "is_default": 1,
        });
        let f = entry_to_format("42", 0, &entry).expect("format built");
        assert_eq!(
            f.url,
            "https://abxxx.com/get_file/1/abc/0/42/42.mp4/?d=10&br=500"
        );
        assert_eq!(f.ext, "mp4");
        assert_eq!(f.tbr, Some(500.0));
    }
}
