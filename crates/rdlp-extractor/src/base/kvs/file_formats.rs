//! Parser for KVS's packed `file_formats` blob.
//!
//! The KVS search/listing endpoint (`/api/videos2.php` JSON) returns each
//! row's playable variants encoded as a single packed string:
//!
//! ```text
//! ||{ext}|{WxH}|{duration}|{filesize}|{tnc}|{tnt}|{flag}||{ext2}|...
//! ```
//!
//! Pipe-separated records, each delimited by `||`. The trailer/preview
//! variant uses an extension prefixed with `_tr` (e.g. `_tr.mp4`); we
//! skip it because it never describes the playable file.
//!
//! Because the same row also exposes a separate top-level
//! `file_dimensions` field, [`parse_search_for_file_formats`] prefers
//! that for `width`/`height` and falls back to the blob.

use serde_json::Value;

/// Width/height/filesize extracted from a KVS row's `file_formats` blob.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KvsFileFormatInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub filesize: Option<u64>,
}

/// Parse a `file_formats` packed string.
///
/// Picks the first variant whose extension is NOT prefixed with `_tr`.
/// Returns a fully-empty `KvsFileFormatInfo` when no playable variant is
/// found (e.g. a row that contains only the trailer preview).
pub(crate) fn parse_file_formats_blob(blob: &str) -> KvsFileFormatInfo {
    let mut info = KvsFileFormatInfo::default();
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

/// Find the `videos[]` row with `video_id == target_id` in a KVS search
/// response and project its `file_dimensions` + `file_formats` into a
/// `KvsFileFormatInfo`.
///
/// Returns `None` when the body isn't valid JSON in the expected shape,
/// the target row is absent, or the row carries no usable dimension /
/// filesize information.
#[must_use]
pub(crate) fn parse_search_for_file_formats(
    body: &str,
    target_id: &str,
) -> Option<KvsFileFormatInfo> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let videos = parsed.get("videos")?.as_array()?;
    let row = videos
        .iter()
        .find(|v| v.get("video_id").and_then(|x| x.as_str()) == Some(target_id))?;

    let mut info = KvsFileFormatInfo::default();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_skips_trailer_and_extracts_first_real_variant() {
        let blob = "||.mp4|1280x720|5443|853473694|363|15|0||_tr.mp4|384x216|7|195678|0|0|0";
        let info = parse_file_formats_blob(blob);
        assert_eq!(info.width, Some(1280));
        assert_eq!(info.height, Some(720));
        assert_eq!(info.filesize, Some(853_473_694));
    }

    #[test]
    fn blob_handles_only_trailer() {
        let blob = "||_tr.mp4|384x216|7|195678|0|0|0";
        assert_eq!(parse_file_formats_blob(blob), KvsFileFormatInfo::default());
    }

    #[test]
    fn search_finds_matching_row() {
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
    fn search_returns_none_when_id_missing() {
        let body = serde_json::json!({"videos": [{"video_id": "X", "file_dimensions": "1x1"}]})
            .to_string();
        assert!(parse_search_for_file_formats(&body, "missing").is_none());
    }

    #[test]
    fn search_falls_back_to_dimensions_only() {
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
