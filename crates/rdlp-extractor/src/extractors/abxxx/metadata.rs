//! Metadata enrichment for ABXXX.
//!
//! ABXXX's `/api/videofile.php` only returns the playable URL — no title,
//! no dimensions, no models. This module owns the two endpoints that fill
//! in the gaps:
//!
//! 1. **`/api/json/video/{lifetime}/{e6}/{e3}/{id}.json`** — rich per-video
//!    metadata: real title, description, structured models + categories,
//!    statistics, post date, and full-resolution thumbnails. The path
//!    components `e6` and `e3` are bucket prefixes
//!    `floor(id/1e6)*1e6` / `floor(id/1e3)*1e3` (mirroring the
//!    `app.js` `video()` helper).
//!
//! 2. **`/api/videos2.php?params=…&s={slug-words}`** — the search endpoint.
//!    A search keyed by the slug words almost always returns the canonical
//!    video row, which carries `file_dimensions` (`WxH`) and a packed
//!    `file_formats` string with per-variant `{ext}|{WxH}|{duration}|
//!    {filesize}|{tnc}|{tnt}|{flag}` records. Only the variant whose
//!    extension is plain `.mp4` (NOT `_tr.mp4` — that's the trailer
//!    preview) describes the playable file. The lookup is best-effort:
//!    failure is non-fatal and the extractor still ships a usable Format
//!    without dimensions.

use serde_json::Value;

const ABXXX_BASE_URL: &str = "https://abxxx.com";
const LOOKUP_LIFETIME: u32 = 86_400;
const SEARCH_RESULTS_PER_PAGE: u32 = 60;

/// Build the per-video lookup endpoint URL for `video_id`.
///
/// Mirrors the `app.js` `video()` helper:
/// `/api/json/video/{lifetime}/{floor(id/1e6)*1e6}/{floor(id/1e3)*1e3}/{id}.json`
#[must_use]
pub(crate) fn lookup_endpoint(video_id: &str) -> Option<String> {
    let id: u64 = video_id.parse().ok()?;
    let e6 = (id / 1_000_000) * 1_000_000;
    let e3 = (id / 1_000) * 1_000;
    Some(format!(
        "{ABXXX_BASE_URL}/api/json/video/{LOOKUP_LIFETIME}/{e6}/{e3}/{id}.json"
    ))
}

/// Build the slug-driven search URL used to look up file format details.
///
/// The search query is the slug with hyphens replaced by spaces. The
/// canonical row is then identified by matching `video_id` in the result
/// set, not by relying on positional order.
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

/// Lookup metadata extracted from `/api/json/video/.../{id}.json`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct LookupMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub upload_date: Option<String>,
    pub view_count: Option<u64>,
    pub average_rating: Option<f64>,
    pub uploader: Option<String>,
    pub uploader_id: Option<String>,
    pub categories: Vec<String>,
    pub actors: Vec<String>,
}

/// File-level format metadata extracted from a search row's `file_formats` string.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileFormatInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub filesize: Option<u64>,
}

/// Parse `MM:SS` or `HH:MM:SS` into seconds.
fn parse_duration(s: &str) -> Option<f64> {
    let parts: Vec<u64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [s] => Some(*s as f64),
        [m, s] => Some((m * 60 + s) as f64),
        [h, m, s] => Some((h * 3600 + m * 60 + s) as f64),
        _ => None,
    }
}

/// Sort the entries of a JSON object by numeric key, stably falling back to
/// lexicographic order on non-numeric keys, then collect their `title` fields.
///
/// ABXXX returns categories/models as `{id: {title, dir, …}}` maps. Using a
/// numeric-key sort produces a stable ordering matching the site's own UI
/// pagination of these lists.
fn collect_titles_in_key_order(value: Option<&Value>) -> Vec<String> {
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut entries: Vec<(&String, &Value)> = obj.iter().collect();
    entries.sort_by(|a, b| {
        let na: u64 = a.0.parse().unwrap_or(u64::MAX);
        let nb: u64 = b.0.parse().unwrap_or(u64::MAX);
        na.cmp(&nb).then_with(|| a.0.cmp(b.0))
    });
    entries
        .into_iter()
        .filter_map(|(_, v)| {
            v.get("title")
                .and_then(|t| t.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect()
}

/// Parse the `/api/json/video/...` JSON body into a `LookupMetadata`.
///
/// Returns `None` if the response shape is unrecognisable. The function is
/// tolerant: missing fields produce `None`/empty values rather than errors,
/// because the lookup is enrichment, not a load-bearing extraction step.
#[must_use]
pub(crate) fn parse_lookup_response(body: &str) -> Option<LookupMetadata> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let v = parsed.get("video").or(Some(&parsed))?;

    let str_field = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    let title = str_field("title");
    let description = str_field("description");
    let upload_date = str_field("post_date");
    let duration = v
        .get("duration")
        .and_then(|d| d.as_str())
        .and_then(parse_duration);

    // Prefer the high-resolution preview thumbnail when present.
    let thumbnail = str_field("thumbsrc").or_else(|| str_field("thumb"));

    let stats = v.get("statistics");
    let view_count = stats
        .and_then(|s| s.get("viewed"))
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| stats.and_then(|s| s.get("viewed")).and_then(|x| x.as_u64()));
    let average_rating = stats
        .and_then(|s| s.get("rating"))
        .and_then(|x| x.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| stats.and_then(|s| s.get("rating")).and_then(|x| x.as_f64()));

    let user = v.get("user");
    let uploader = user
        .and_then(|u| u.get("username"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let uploader_id = user
        .and_then(|u| u.get("id"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let categories = collect_titles_in_key_order(v.get("categories"));
    let actors = collect_titles_in_key_order(v.get("models"));

    Some(LookupMetadata {
        title,
        description,
        duration,
        thumbnail,
        upload_date,
        view_count,
        average_rating,
        uploader,
        uploader_id,
        categories,
        actors,
    })
}

/// Find the `videos[]` row matching `target_id` and parse its `file_formats`
/// string into a `FileFormatInfo`.
///
/// Returns `None` if the JSON is unparseable, no row matches the target id,
/// or the `file_formats` string contains no playable variant.
#[must_use]
pub(crate) fn parse_search_for_file_formats(body: &str, target_id: &str) -> Option<FileFormatInfo> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let videos = parsed.get("videos")?.as_array()?;
    let row = videos
        .iter()
        .find(|v| v.get("video_id").and_then(|x| x.as_str()) == Some(target_id))?;

    // Prefer the structured `file_dimensions` field (e.g. "1280x720"); fall
    // back to parsing the packed `file_formats` blob.
    let mut info = FileFormatInfo::default();
    if let Some(dims) = row.get("file_dimensions").and_then(|x| x.as_str())
        && let Some((w, h)) = dims.split_once('x')
    {
        info.width = w.parse().ok();
        info.height = h.parse().ok();
    }

    if let Some(blob) = row.get("file_formats").and_then(|x| x.as_str()) {
        let parsed = parse_file_formats_blob(blob);
        if info.width.is_none() {
            info.width = parsed.width;
        }
        if info.height.is_none() {
            info.height = parsed.height;
        }
        info.filesize = parsed.filesize;
    }

    if info.width.is_none() && info.height.is_none() && info.filesize.is_none() {
        None
    } else {
        Some(info)
    }
}

/// Parse the packed `file_formats` blob.
///
/// Format (observed): `||{ext}|{WxH}|{duration}|{filesize}|{tnc}|{tnt}|{flag}`
/// repeated per variant, separated by `||`. The trailer preview uses an
/// extension starting with `_tr` (e.g. `_tr.mp4`); we skip it and pick the
/// first remaining record.
fn parse_file_formats_blob(blob: &str) -> FileFormatInfo {
    let mut info = FileFormatInfo::default();
    for entry in blob.split("||").filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = entry.split('|').collect();
        if parts.len() < 4 {
            continue;
        }
        let ext = parts[0];
        if ext.starts_with("_tr") {
            continue;
        }
        let dims = parts[1];
        if let Some((w, h)) = dims.split_once('x') {
            info.width = w.parse().ok();
            info.height = h.parse().ok();
        }
        info.filesize = parts[3].parse().ok();
        return info;
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_endpoint_uses_million_and_thousand_buckets() {
        assert_eq!(
            lookup_endpoint("129452").as_deref(),
            Some("https://abxxx.com/api/json/video/86400/0/129000/129452.json")
        );
        assert_eq!(
            lookup_endpoint("2_500_123".replace('_', "").as_str()).as_deref(),
            Some("https://abxxx.com/api/json/video/86400/2000000/2500000/2500123.json")
        );
        assert!(lookup_endpoint("not-numeric").is_none());
    }

    #[test]
    fn slug_search_endpoint_replaces_hyphens_with_spaces() {
        let url = slug_search_endpoint("excogi-katie-carmine-in-hd");
        assert!(url.contains("&s=excogi%20katie%20carmine%20in%20hd"));
        assert!(url.contains("/search..1.all.."));
    }

    #[test]
    fn parse_lookup_extracts_full_metadata() {
        let body = serde_json::json!({
            "video": {
                "video_id": "129452",
                "title": "ExCoGi - Katie Carmine in HD",
                "description": "the description",
                "dir": "excogi-katie-carmine-in-hd",
                "duration": "1:30:43",
                "post_date": "2020-09-23 12:45:52",
                "thumb": "https://ii.abxxx.com/.../480x270/4.jpg",
                "thumbsrc": "https://ii.abxxx.com/.../preview.jpg",
                "statistics": {"viewed": "87637", "rating": "4.20"},
                "user": {"id": "8755", "username": "Xavier Torres"},
                "categories": {
                    "4": {"title": "HD", "dir": "hd"},
                    "8": {"title": "Deepthroat", "dir": "deepthroat"},
                },
                "models": {"3110": {"title": "Katie Carmine", "dir": "katie-carmine"}},
            }
        })
        .to_string();
        let m = parse_lookup_response(&body).expect("parsed");
        assert_eq!(m.title.as_deref(), Some("ExCoGi - Katie Carmine in HD"));
        assert_eq!(m.description.as_deref(), Some("the description"));
        assert_eq!(m.duration, Some(5443.0));
        assert_eq!(
            m.thumbnail.as_deref(),
            Some("https://ii.abxxx.com/.../preview.jpg")
        );
        assert_eq!(m.upload_date.as_deref(), Some("2020-09-23 12:45:52"));
        assert_eq!(m.view_count, Some(87637));
        assert_eq!(m.average_rating, Some(4.20));
        assert_eq!(m.uploader.as_deref(), Some("Xavier Torres"));
        assert_eq!(m.uploader_id.as_deref(), Some("8755"));
        // Categories ordered numerically by id (4 before 8)
        assert_eq!(m.categories, vec!["HD", "Deepthroat"]);
        assert_eq!(m.actors, vec!["Katie Carmine"]);
    }

    #[test]
    fn parse_lookup_falls_back_to_thumb_when_thumbsrc_absent() {
        let body = serde_json::json!({
            "video": {"thumb": "https://example.com/thumb.jpg"}
        })
        .to_string();
        let m = parse_lookup_response(&body).expect("parsed");
        assert_eq!(
            m.thumbnail.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
    }

    #[test]
    fn parse_lookup_handles_missing_fields_gracefully() {
        let body = serde_json::json!({"video": {}}).to_string();
        let m = parse_lookup_response(&body).expect("parsed");
        assert!(m.title.is_none());
        assert!(m.duration.is_none());
        assert!(m.categories.is_empty());
        assert!(m.actors.is_empty());
    }

    #[test]
    fn parse_lookup_returns_none_on_garbage() {
        assert!(parse_lookup_response("not json").is_none());
    }

    #[test]
    fn file_formats_blob_skips_trailer_and_extracts_first_real_variant() {
        let blob = "||.mp4|1280x720|5443|853473694|363|15|0||_tr.mp4|384x216|7|195678|0|0|0";
        let info = parse_file_formats_blob(blob);
        assert_eq!(info.width, Some(1280));
        assert_eq!(info.height, Some(720));
        assert_eq!(info.filesize, Some(853_473_694));
    }

    #[test]
    fn file_formats_blob_handles_only_trailer() {
        let blob = "||_tr.mp4|384x216|7|195678|0|0|0";
        assert_eq!(parse_file_formats_blob(blob), FileFormatInfo::default());
    }

    #[test]
    fn parse_search_for_file_formats_finds_matching_row() {
        let body = serde_json::json!({
            "videos": [
                {
                    "video_id": "OTHER",
                    "file_dimensions": "640x480",
                    "file_formats": "||.mp4|640x480|10|123|0|0|0"
                },
                {
                    "video_id": "129452",
                    "file_dimensions": "1280x720",
                    "file_formats": "||.mp4|1280x720|5443|853473694|363|15|0||_tr.mp4|384x216|7|195678|0|0|0"
                }
            ]
        })
        .to_string();
        let info = parse_search_for_file_formats(&body, "129452").expect("found");
        assert_eq!(info.width, Some(1280));
        assert_eq!(info.height, Some(720));
        assert_eq!(info.filesize, Some(853_473_694));
    }

    #[test]
    fn parse_search_for_file_formats_returns_none_when_id_missing() {
        let body = serde_json::json!({"videos": [{"video_id": "X", "file_dimensions": "1x1"}]})
            .to_string();
        assert!(parse_search_for_file_formats(&body, "missing").is_none());
    }

    #[test]
    fn parse_search_for_file_formats_falls_back_to_dimensions_only() {
        let body = serde_json::json!({
            "videos": [{"video_id": "X", "file_dimensions": "1920x1080"}]
        })
        .to_string();
        let info = parse_search_for_file_formats(&body, "X").expect("found");
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.height, Some(1080));
        assert_eq!(info.filesize, None);
    }
}
