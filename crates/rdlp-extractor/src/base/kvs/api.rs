//! KVS JSON API helpers — endpoint URL builders and response parsers.
//!
//! The upstream KVS PHP code ships with a small set of JSON endpoints that
//! every deployment exposes:
//!
//! - `/api/json/video/{lifetime}/{e6}/{e3}/{id}.json` — per-video metadata
//!   with id-bucketed path components (`floor(id/1e6)*1e6` and
//!   `floor(id/1e3)*1e3`).
//! - `/api/videos2.php?params={lifetime}/{gender}/{sort}/{count}/{section}.{object_id}.{page}.{type}.{duration}.{date}&s={query}`
//!   — listing/search endpoint. The `params` blob is a slash-separated
//!   path string built by the site's `videos()` helper in `app.js`.
//!
//! This module owns the URL shapes and the response parser for the
//! per-video lookup. Callers pass the deployment's base URL so the
//! same code works for ABXXX, AbJav, AbGay, AbTranny, AbMilf, etc.

use serde_json::Value;

/// Canonical KVS lifetime parameter (seconds). The same value backs every
/// stock KVS endpoint that accepts a `lifetime` slot — `videofile.php`,
/// `/api/json/video/...`, and `/api/videos2.php` — and matches the value
/// the live JS uses for cache warming.
pub(crate) const KVS_DEFAULT_LIFETIME: u32 = 86_400;

/// Default per-page result count for `/api/videos2.php` listings — matches
/// the live JS default and is honoured by every KVS deployment.
pub(crate) const KVS_VIDEOS2_DEFAULT_PAGE_SIZE: u32 = 60;

/// Build the playable-URL endpoint (`/api/videofile.php`).
///
/// `base_url` should NOT have a trailing slash (e.g. `https://abxxx.com`).
#[must_use]
pub(crate) fn videofile_endpoint(base_url: &str, video_id: &str, lifetime: u32) -> String {
    format!("{base_url}/api/videofile.php?video_id={video_id}&lifetime={lifetime}")
}

/// Build the per-video lookup endpoint URL.
///
/// Mirrors the upstream KVS `video()` helper in `app.js`: the path embeds
/// `floor(id/1e6)*1e6` and `floor(id/1e3)*1e3` bucket prefixes for cache
/// sharding. `base_url` should NOT have a trailing slash. Returns `None`
/// when `video_id` isn't a positive integer.
#[must_use]
pub(crate) fn video_lookup_endpoint(
    base_url: &str,
    video_id: &str,
    lifetime: u32,
) -> Option<String> {
    let id: u64 = video_id.parse().ok()?;
    let e6 = (id / 1_000_000) * 1_000_000;
    let e3 = (id / 1_000) * 1_000;
    Some(format!(
        "{base_url}/api/json/video/{lifetime}/{e6}/{e3}/{id}.json"
    ))
}

/// Build the canonical KVS search URL (`/api/videos2.php` with
/// `section=search`).
///
/// Mirrors the upstream `videos()` helper in `app.js`. The fixed slots
/// (`lifetime=0` for jsond, `gender=str`, empty `object_id`/`type`/
/// `duration`/`date`) match KVS defaults. Sibling deployments that need
/// a different gender (e.g. `gay`, `tranny`) should adopt a wrapper that
/// fills in the gender slot — most current consumers use the default.
#[must_use]
pub(crate) fn videos2_search_endpoint(
    base_url: &str,
    query: &str,
    sort: &str,
    page: u32,
    count: u32,
) -> String {
    let q = urlencoding::encode(query);
    format!("{base_url}/api/videos2.php?params=0/str/{sort}/{count}/search..{page}.all..&s={q}")
}

/// Per-video metadata extracted from the KVS lookup endpoint.
///
/// Fields mirror the canonical KVS schema; missing data produces
/// `None`/empty rather than errors because callers treat enrichment as
/// best-effort.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct KvsVideoMetadata {
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

/// Parse a KVS duration string into seconds.
///
/// Accepts `MM:SS`, `HH:MM:SS`, and bare seconds (`SS`). Returns `None`
/// for any other shape (e.g. the empty string, ISO 8601, or anything
/// containing non-numeric segments).
#[must_use]
pub(crate) fn parse_kvs_duration(s: &str) -> Option<f64> {
    let parts: Vec<u64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [s] => Some(*s as f64),
        [m, s] => Some((m * 60 + s) as f64),
        [h, m, s] => Some((h * 3600 + m * 60 + s) as f64),
        _ => None,
    }
}

/// Sort the entries of a JSON object by numeric key (with lexicographic
/// fallback for non-numeric ids), then collect their `title` fields.
///
/// KVS returns categories/models as `{id: {title, dir, …}}` maps. Sorting
/// by numeric id gives a stable, UI-matching order.
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

/// Parse the body of `/api/json/video/...` into a [`KvsVideoMetadata`].
///
/// Returns `None` when the body isn't valid JSON in the expected
/// envelope shape (`{"video": {…}}` or, as a fallback, a top-level
/// object with the same fields).
#[must_use]
pub(crate) fn parse_video_lookup(body: &str) -> Option<KvsVideoMetadata> {
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
        .and_then(parse_kvs_duration);

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

    Some(KvsVideoMetadata {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_endpoint_uses_million_and_thousand_buckets() {
        assert_eq!(
            video_lookup_endpoint("https://abxxx.com", "129452", KVS_DEFAULT_LIFETIME).as_deref(),
            Some("https://abxxx.com/api/json/video/86400/0/129000/129452.json")
        );
        assert_eq!(
            video_lookup_endpoint("https://example.com", "2500123", KVS_DEFAULT_LIFETIME)
                .as_deref(),
            Some("https://example.com/api/json/video/86400/2000000/2500000/2500123.json")
        );
        assert!(video_lookup_endpoint("https://abxxx.com", "not-numeric", 0).is_none());
    }

    #[test]
    fn lookup_endpoint_threads_explicit_lifetime() {
        assert_eq!(
            video_lookup_endpoint("https://abxxx.com", "1", 14400).as_deref(),
            Some("https://abxxx.com/api/json/video/14400/0/0/1.json")
        );
    }

    #[test]
    fn videofile_endpoint_builds_expected_url() {
        assert_eq!(
            videofile_endpoint("https://abxxx.com", "129452", KVS_DEFAULT_LIFETIME),
            "https://abxxx.com/api/videofile.php?video_id=129452&lifetime=86400"
        );
    }

    #[test]
    fn videos2_search_endpoint_encodes_query_and_threads_sort_and_page() {
        let url = videos2_search_endpoint(
            "https://abxxx.com",
            "katie carmine",
            "relevance",
            1,
            KVS_VIDEOS2_DEFAULT_PAGE_SIZE,
        );
        assert_eq!(
            url,
            "https://abxxx.com/api/videos2.php?params=0/str/relevance/60/search..1.all..&s=katie%20carmine"
        );
    }

    #[test]
    fn videos2_search_endpoint_paginates() {
        let url = videos2_search_endpoint(
            "https://abxxx.com",
            "x",
            "latest-updates",
            5,
            KVS_VIDEOS2_DEFAULT_PAGE_SIZE,
        );
        assert!(url.contains("/latest-updates/"));
        assert!(url.contains("search..5.all.."));
    }

    #[test]
    fn parse_kvs_duration_handles_all_three_shapes() {
        assert_eq!(parse_kvs_duration("06:15"), Some(375.0));
        assert_eq!(parse_kvs_duration("55:20"), Some(3320.0));
        assert_eq!(parse_kvs_duration("1:05:42"), Some(3942.0));
        assert_eq!(parse_kvs_duration("90"), Some(90.0));
        assert_eq!(parse_kvs_duration(""), None);
        assert_eq!(parse_kvs_duration("garbage"), None);
    }

    #[test]
    fn parse_video_lookup_extracts_full_metadata() {
        let body = serde_json::json!({
            "video": {
                "video_id": "129452",
                "title": "ExCoGi - Katie Carmine in HD",
                "description": "the description",
                "duration": "1:30:43",
                "post_date": "2020-09-23 12:45:52",
                "thumb": "https://ii.example.com/.../480x270/4.jpg",
                "thumbsrc": "https://ii.example.com/.../preview.jpg",
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
        let m = parse_video_lookup(&body).expect("parsed");
        assert_eq!(m.title.as_deref(), Some("ExCoGi - Katie Carmine in HD"));
        assert_eq!(m.description.as_deref(), Some("the description"));
        assert_eq!(m.duration, Some(5443.0));
        assert_eq!(
            m.thumbnail.as_deref(),
            Some("https://ii.example.com/.../preview.jpg")
        );
        assert_eq!(m.upload_date.as_deref(), Some("2020-09-23 12:45:52"));
        assert_eq!(m.view_count, Some(87637));
        assert_eq!(m.average_rating, Some(4.20));
        assert_eq!(m.uploader.as_deref(), Some("Xavier Torres"));
        assert_eq!(m.uploader_id.as_deref(), Some("8755"));
        // Numeric-id sort: 4 before 8
        assert_eq!(m.categories, vec!["HD", "Deepthroat"]);
        assert_eq!(m.actors, vec!["Katie Carmine"]);
    }

    #[test]
    fn parse_video_lookup_falls_back_to_thumb_when_thumbsrc_absent() {
        let body =
            serde_json::json!({"video": {"thumb": "https://example.com/thumb.jpg"}}).to_string();
        let m = parse_video_lookup(&body).expect("parsed");
        assert_eq!(
            m.thumbnail.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
    }

    #[test]
    fn parse_video_lookup_handles_missing_fields_gracefully() {
        let body = serde_json::json!({"video": {}}).to_string();
        let m = parse_video_lookup(&body).expect("parsed");
        assert!(m.title.is_none());
        assert!(m.duration.is_none());
        assert!(m.categories.is_empty());
        assert!(m.actors.is_empty());
    }

    #[test]
    fn parse_video_lookup_returns_none_on_garbage() {
        assert!(parse_video_lookup("not json").is_none());
    }
}
