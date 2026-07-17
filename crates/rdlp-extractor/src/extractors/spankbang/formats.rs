//! Format extraction for SpankBang.
//!
//! Two paths in priority order:
//!
//! 1. **Inline `stream_data`**: the video page embeds a Python-dict literal
//!    with every resolution and HLS variant. Single GET, no extra round-trip.
//! 2. **Formats API fallback**: when `stream_data` is absent (legacy pages),
//!    regex-extract `data-streamkey`, POST to `/api/videos/stream`, parse
//!    the same JSON shape.

use rdlp_types::Codec;
use rdlp_types::{DownloadProtocol, Format};
use serde_json::Value;

use super::patterns;

/// Map a SpankBang JSON-key like "1080p" to a height in pixels.
fn height_for_label(label: &str) -> Option<u32> {
    match label {
        "240p" => Some(240),
        "320p" => Some(320),
        "480p" => Some(480),
        "720p" => Some(720),
        "1080p" => Some(1080),
        "4k" => Some(2160),
        _ => None,
    }
}

/// Extract the inline `stream_data` JSON value from a SpankBang video page.
/// Returns `None` if the page does not embed it (caller should fall back to
/// the formats-API path).
pub(super) fn parse_inline_stream_data(html: &str) -> Option<Value> {
    let caps = patterns::STREAM_DATA_INLINE.captures(html)?;
    let body = caps.get(1)?.as_str();
    let json = patterns::pydict_to_json(body);
    serde_json::from_str(&json).ok()
}

/// Extract the `data-streamkey` token from a video-page HTML for the
/// formats-API fallback. Returns `None` when the anchor is missing.
pub(super) fn parse_streamkey(html: &str) -> Option<String> {
    patterns::STREAMKEY
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Convert a SpankBang `stream_data` / API JSON object to `Vec<Format>`.
/// Skips empty arrays. Sets `vcodec=h264`, `acodec=aac`, `container=mp4`
/// for direct MP4s; HLS rows get `protocol = M3u8`.
///
/// Rows are built from explicit label allow-lists. Two `stream_data` keys are
/// deliberately excluded, and the allow-lists are what make that correct:
///
/// - **`main`** aliases the highest available progressive rendition — live
///   probing (n=5, 2026-07-17) found it byte-identical to the top MP4 modulo
///   the per-request `secure=` token. Emitting it would duplicate that row.
/// - **`mpd`** is the DASH manifest key. It was empty in every video sampled
///   and in both fixtures; wiring an unexercised DASH path would contradict
///   the project's no-dead-code-for-future stance. Deferred until a video
///   with a populated `mpd` turns up (issue #500).
pub(super) fn build_formats(data: &Value) -> Vec<Format> {
    let mut formats = Vec::new();
    let Value::Object(map) = data else {
        return formats;
    };

    // Direct MP4 progressive.
    for label in ["240p", "320p", "480p", "720p", "1080p", "4k"] {
        if let Some(arr) = map.get(label).and_then(Value::as_array)
            && let Some(url) = arr.first().and_then(Value::as_str)
        {
            let mut f = Format::new(label, url, "mp4", DownloadProtocol::Https);
            f.vcodec = Codec::from("h264".to_string());
            f.acodec = Codec::from("aac".to_string());
            f.container = Some("mp4".to_string());
            f.height = height_for_label(label);
            f.format_note = Some(label.to_uppercase());
            formats.push(f);
        }
    }

    // Multi-rendition master HLS playlist.
    if let Some(arr) = map.get("m3u8").and_then(Value::as_array)
        && let Some(url) = arr.first().and_then(Value::as_str)
    {
        let mut f = Format::new("hls", url, "m3u8", DownloadProtocol::M3u8);
        f.vcodec = Codec::from("h264".to_string());
        f.acodec = Codec::from("aac".to_string());
        f.container = Some("m3u8".to_string());
        f.format_note = Some("HLS".to_string());
        formats.push(f);
    }

    // Single-rendition HLS variants (one per resolution).
    // 320p is intentionally omitted — SpankBang does not serve a stable
    // m3u8_320p stream; matches yt-dlp's reference behaviour.
    for label in ["240p", "480p", "720p", "1080p", "4k"] {
        let key = format!("m3u8_{label}");
        if let Some(arr) = map.get(&key).and_then(Value::as_array)
            && let Some(url) = arr.first().and_then(Value::as_str)
        {
            let mut f = Format::new(&key, url, "m3u8", DownloadProtocol::M3u8);
            f.vcodec = Codec::from("h264".to_string());
            f.acodec = Codec::from("aac".to_string());
            f.container = Some("m3u8".to_string());
            f.height = height_for_label(label);
            f.format_note = Some(format!("HLS {}", label.to_uppercase()));
            formats.push(f);
        }
    }

    formats
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("tests/spankbang_video_page.html");
    const API: &str = include_str!("tests/spankbang_formats_api.json");

    #[test]
    fn inline_stream_data_present_in_fixture() {
        let data = parse_inline_stream_data(PAGE).expect("inline stream_data must parse");
        assert!(data.is_object());
        assert!(data.get("1080p").is_some(), "1080p key present");
        assert!(data.get("m3u8").is_some(), "m3u8 master present");
        assert_eq!(data.get("length").and_then(Value::as_u64), Some(910));
    }

    #[test]
    fn streamkey_extractable_from_fixture() {
        let key = parse_streamkey(PAGE).expect("data-streamkey must be present");
        // Exact value from the recorded fixture — deterministic, locks the
        // shape <base>.<sig> contract.
        assert_eq!(key, "MTcwMDE4NDU.XTPJk92TYuF3gscEzR5UhXY4ttk");
    }

    #[test]
    fn build_formats_from_inline_data() {
        let data = parse_inline_stream_data(PAGE).expect("inline parse");
        let formats = build_formats(&data);
        assert!(!formats.is_empty(), "at least one format");
        // 1080p MP4 must be present + carry the right shape.
        let f1080 = formats
            .iter()
            .find(|f| f.format_id == "1080p")
            .expect("1080p MP4 row");
        assert_eq!(f1080.ext, "mp4");
        assert_eq!(f1080.height, Some(1080));
        assert_eq!(f1080.vcodec.as_str(), Some("h264"));
        // Multi-rendition HLS master row.
        let hls = formats
            .iter()
            .find(|f| f.format_id == "hls")
            .expect("hls master row");
        assert_eq!(hls.ext, "m3u8");

        // Empty arrays in the fixture (`'320p': []`, `'4k': []`) must NOT
        // produce format rows.
        assert!(
            formats.iter().all(|f| f.format_id != "320p"),
            "320p is empty in fixture; must not produce a row"
        );
        assert!(
            formats.iter().all(|f| f.format_id != "4k"),
            "4k is empty in fixture; must not produce a row"
        );
    }

    #[test]
    fn build_formats_from_api_json() {
        let data: Value = serde_json::from_str(API).expect("API JSON parse");
        let formats = build_formats(&data);
        assert!(!formats.is_empty(), "API path also produces formats");
        assert!(formats.iter().any(|f| f.format_id == "1080p"));
        assert!(formats.iter().any(|f| f.format_id == "hls"));
    }

    // `main` is an alias for the highest-available progressive rendition
    // (verified live: byte-identical URL to the top MP4 rendition, modulo
    // the per-request `secure=` token). It is deliberately skipped by
    // `build_formats` — including it would emit a duplicate row. The
    // fixture MUST keep `main` populated, or this test would pass
    // vacuously; assert the precondition alongside the behaviour.
    #[test]
    fn main_key_populated_but_produces_no_row() {
        let data = parse_inline_stream_data(PAGE).expect("inline parse");
        let main = data
            .get("main")
            .and_then(Value::as_array)
            .expect("fixture precondition: 'main' key must be a populated array");
        assert!(
            !main.is_empty(),
            "fixture precondition: 'main' must be non-empty for this test to be meaningful"
        );

        let formats = build_formats(&data);
        assert!(
            formats.iter().all(|f| f.format_id != "main"),
            "'main' is an alias for the top progressive rendition; must not produce its own row"
        );
    }

    // `mpd` (DASH) is deliberately unwired for SpankBang: it was empty in all
    // 5 videos sampled live and in BOTH fixtures, and building an unexercised
    // code path contradicts the no-dead-code-for-future stance (issue #500).
    //
    // The fixtures cannot pin that decision. An empty array is dropped by the
    // `arr.first()` guard no matter what the allow-list says, so a
    // fixture-driven `mpd` assertion passes even with `mpd` in the allow-list
    // — it would test the empty-array skip (already covered by the 320p/4k
    // assertions above) while appearing to test the DASH decision. Feed a
    // synthetic POPULATED `mpd` instead, so the test actually fails if someone
    // starts emitting DASH rows without a real fixture to justify them.
    #[test]
    fn populated_mpd_still_produces_no_row() {
        let data = serde_json::json!({
            "1080p": ["https://example.invalid/v-1080p.mp4"],
            "mpd": ["https://example.invalid/v.mpd"],
        });

        let formats = build_formats(&data);
        assert!(
            !formats.is_empty(),
            "guard: the synthetic 1080p row must still be built, or this test proves nothing"
        );
        assert!(
            formats.iter().all(|f| f.format_id != "mpd"),
            "DASH is deliberately unwired; a populated 'mpd' must not produce a row"
        );
    }

    #[test]
    fn no_duplicate_format_ids_on_either_fixture() {
        for (label, data) in [
            (
                "inline",
                parse_inline_stream_data(PAGE).expect("inline parse"),
            ),
            ("api", serde_json::from_str(API).expect("API JSON parse")),
        ] {
            let formats = build_formats(&data);
            let mut ids: Vec<&str> = formats.iter().map(|f| f.format_id.as_str()).collect();
            let before = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(
                ids.len(),
                before,
                "{label} fixture: duplicate format_id among emitted rows"
            );
        }
    }

    // Height legitimately repeats ACROSS protocols today (a progressive MP4
    // row and its HLS-variant sibling both carry `height = Some(1080)`), so
    // uniqueness is pinned only WITHIN the progressive (Https) rows — i.e.
    // that `height_for_label` stays injective over the allow-list.
    //
    // Scope note: this does NOT catch a bare `main` added to the allow-list
    // (`height_for_label("main")` is `None`, which is unique among the real
    // heights) — `main_key_populated_but_produces_no_row` is what catches
    // that. This one catches an alias mapped ONTO an existing height.
    #[test]
    fn no_duplicate_height_among_progressive_rows() {
        for (label, data) in [
            (
                "inline",
                parse_inline_stream_data(PAGE).expect("inline parse"),
            ),
            ("api", serde_json::from_str(API).expect("API JSON parse")),
        ] {
            let formats = build_formats(&data);
            let mut heights: Vec<Option<u32>> = formats
                .iter()
                .filter(|f| f.protocol == DownloadProtocol::Https)
                .map(|f| f.height)
                .collect();
            let before = heights.len();
            heights.sort_unstable();
            heights.dedup();
            assert_eq!(
                heights.len(),
                before,
                "{label} fixture: duplicate height among progressive (Https) rows"
            );
        }
    }
}
