//! Tests for the format selector parser and evaluator.

use super::*;
use crate::format::Format;
use crate::protocol::DownloadProtocol;

// ---- Test helpers ----

fn make_combined(id: &str, ext: &str, height: u32, quality: i32) -> Format {
    let mut f = Format::new(id, format!("url_{id}"), ext, DownloadProtocol::Https);
    f.vcodec = Some("h264".to_string());
    f.acodec = Some("aac".to_string());
    f.height = Some(height);
    f.width = Some(height * 16 / 9);
    f.quality = Some(quality);
    f.tbr = Some(height as f64 * 2.0);
    f
}

fn make_video_only(id: &str, ext: &str, height: u32) -> Format {
    let mut f = Format::new(id, format!("url_{id}"), ext, DownloadProtocol::Https);
    f.vcodec = Some("h264".to_string());
    f.acodec = Some("none".to_string());
    f.height = Some(height);
    f.width = Some(height * 16 / 9);
    f.vbr = Some(height as f64 * 1.5);
    f
}

fn make_audio_only(id: &str, ext: &str, abr: f64) -> Format {
    let mut f = Format::new(id, format!("url_{id}"), ext, DownloadProtocol::Https);
    f.vcodec = Some("none".to_string());
    f.acodec = Some("aac".to_string());
    f.abr = Some(abr);
    f
}

fn test_formats() -> Vec<Format> {
    vec![
        make_combined("c360", "mp4", 360, 1),
        make_combined("c720", "mp4", 720, 2),
        make_combined("c1080", "mp4", 1080, 3),
        make_video_only("v720", "mp4", 720),
        make_video_only("v1080", "webm", 1080),
        make_video_only("v1440", "mp4", 1440),
        make_audio_only("a128", "m4a", 128.0),
        make_audio_only("a256", "m4a", 256.0),
        make_audio_only("a64", "webm", 64.0),
    ]
}

// ---- Parser tests ----

#[test]
fn test_parse_basic_selectors() {
    assert!(FormatSelector::parse("best").is_ok());
    assert!(FormatSelector::parse("worst").is_ok());
    assert!(FormatSelector::parse("b").is_ok());
    assert!(FormatSelector::parse("w").is_ok());
    assert!(FormatSelector::parse("bestvideo").is_ok());
    assert!(FormatSelector::parse("bv").is_ok());
    assert!(FormatSelector::parse("bv*").is_ok());
    assert!(FormatSelector::parse("bestaudio").is_ok());
    assert!(FormatSelector::parse("ba").is_ok());
    assert!(FormatSelector::parse("ba*").is_ok());
    assert!(FormatSelector::parse("worstvideo").is_ok());
    assert!(FormatSelector::parse("worstaudio").is_ok());
}

#[test]
fn test_parse_format_id() {
    let sel = FormatSelector::parse("720p").unwrap();
    assert_eq!(sel.expression(), "720p");
}

#[test]
fn test_parse_merge() {
    assert!(FormatSelector::parse("bv+ba").is_ok());
    assert!(FormatSelector::parse("bestvideo+bestaudio").is_ok());
    assert!(FormatSelector::parse("bv*+ba").is_ok());
}

#[test]
fn test_parse_filters() {
    assert!(FormatSelector::parse("bv[height<=720]").is_ok());
    assert!(FormatSelector::parse("bv[height<=720]+ba[abr>=128]").is_ok());
    assert!(FormatSelector::parse("best[ext=mp4]").is_ok());
    assert!(FormatSelector::parse("bv[height<=1080][ext=mp4]").is_ok());
}

#[test]
fn test_parse_fallback() {
    assert!(FormatSelector::parse("bv+ba/b").is_ok());
    assert!(FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/best").is_ok());
}

#[test]
fn test_parse_errors() {
    assert!(FormatSelector::parse("").is_err());
    assert!(FormatSelector::parse("bv[height<=").is_err());
    assert!(FormatSelector::parse("bv[unknownfield=1]").is_err());
    assert!(FormatSelector::parse("[height<=720]").is_err()); // missing base
    assert!(FormatSelector::parse("bv[height]").is_err()); // no operator
}

#[test]
fn test_parse_empty_fallback_rejected() {
    assert!(FormatSelector::parse("/best").is_err());
    assert!(FormatSelector::parse("best/").is_ok()); // trailing empty is trimmed out by split
}

#[test]
fn test_parse_all_operators() {
    assert!(FormatSelector::parse("bv[height=720]").is_ok());
    assert!(FormatSelector::parse("bv[height!=720]").is_ok());
    assert!(FormatSelector::parse("bv[height<720]").is_ok());
    assert!(FormatSelector::parse("bv[height<=720]").is_ok());
    assert!(FormatSelector::parse("bv[height>720]").is_ok());
    assert!(FormatSelector::parse("bv[height>=720]").is_ok());
}

#[test]
fn test_parse_all_fields() {
    for field in &[
        "height",
        "width",
        "ext",
        "vcodec",
        "acodec",
        "fps",
        "tbr",
        "vbr",
        "abr",
        "asr",
        "filesize",
        "protocol",
        "format_id",
    ] {
        let expr = format!("best[{field}=1]");
        assert!(
            FormatSelector::parse(&expr).is_ok(),
            "Failed to parse field: {field}"
        );
    }
}

// ---- Selection tests ----

#[test]
fn test_select_best() {
    let formats = test_formats();
    let sel = FormatSelector::parse("best").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080"); // highest quality combined
}

#[test]
fn test_select_worst() {
    let formats = test_formats();
    let sel = FormatSelector::parse("worst").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c360"); // lowest quality combined
}

#[test]
fn test_select_bestvideo() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bestvideo").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440"); // highest video-only
}

#[test]
fn test_select_bestaudio() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bestaudio").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "a256"); // highest audio-only
}

#[test]
fn test_select_worstvideo() {
    let formats = test_formats();
    let sel = FormatSelector::parse("worstvideo").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v720"); // lowest video-only
}

#[test]
fn test_select_worstaudio() {
    let formats = test_formats();
    let sel = FormatSelector::parse("worstaudio").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "a64"); // lowest audio-only
}

#[test]
fn test_select_bv_star_includes_combined() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv*").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    // bv* considers all formats with video, including combined -- v1440 has highest height
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_select_ba_star_includes_combined() {
    let formats = test_formats();
    let sel = FormatSelector::parse("ba*").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    // ba* considers all formats with audio -- a256 has highest abr
    assert_eq!(result[0].format_id, "a256");
}

#[test]
fn test_select_merge() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv+ba").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].format_id, "v1440"); // best video-only
    assert_eq!(result[1].format_id, "a256"); // best audio-only
}

#[test]
fn test_select_filter_height_le() {
    let formats = test_formats();
    let sel = FormatSelector::parse("best[height<=720]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c720"); // best combined <=720
}

#[test]
fn test_select_filter_height_lt() {
    let formats = test_formats();
    let sel = FormatSelector::parse("best[height<720]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c360"); // only combined <720
}

#[test]
fn test_select_filter_ext() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[ext=webm]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1080"); // only webm video
}

#[test]
fn test_select_filter_ext_ne() {
    let formats = test_formats();
    let sel = FormatSelector::parse("ba[ext!=m4a]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "a64"); // only webm audio
}

#[test]
fn test_select_filter_abr_ge() {
    let formats = test_formats();
    let sel = FormatSelector::parse("ba[abr>=128]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "a256"); // best audio >=128
}

#[test]
fn test_select_multiple_filters() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[height<=1080][ext=mp4]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v720"); // mp4 video-only <=1080
}

#[test]
fn test_select_merge_with_filters() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[height<=720]+ba[abr>=128]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].format_id, "v720"); // best video <=720
    assert_eq!(result[1].format_id, "a256"); // best audio >=128
}

#[test]
fn test_select_fallback() {
    let formats = test_formats();
    // No video-only format at exactly 360p -- falls back to best
    let sel = FormatSelector::parse("bv[height=360]/best").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080"); // fallback to best
}

#[test]
fn test_select_fallback_first_matches() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[height<=720]/best").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v720"); // first fallback matches
}

#[test]
fn test_select_format_id() {
    let formats = test_formats();
    let sel = FormatSelector::parse("c720").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c720");
}

#[test]
fn test_select_drm_excluded() {
    let mut formats = test_formats();
    // Make the best combined format DRM-protected
    formats[2].has_drm = Some(true); // c1080
    let sel = FormatSelector::parse("best").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c720"); // c1080 excluded, next best
}

#[test]
fn test_select_empty_formats() {
    let formats: Vec<Format> = vec![];
    let sel = FormatSelector::parse("best").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_select_no_match() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[height>=4320]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_select_missing_field_conservative() {
    // Format without fps set -- filter on fps should not match
    let mut f = make_combined("test", "mp4", 1080, 5);
    f.fps = None;
    let formats = vec![f];
    let sel = FormatSelector::parse("best[fps>=30]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_select_shorthand_aliases() {
    let formats = test_formats();

    let sel_b = FormatSelector::parse("b").unwrap();
    let sel_best = FormatSelector::parse("best").unwrap();
    assert_eq!(
        sel_b.select(&formats)[0].format_id,
        sel_best.select(&formats)[0].format_id
    );

    let sel_w = FormatSelector::parse("w").unwrap();
    let sel_worst = FormatSelector::parse("worst").unwrap();
    assert_eq!(
        sel_w.select(&formats)[0].format_id,
        sel_worst.select(&formats)[0].format_id
    );
}

#[test]
fn test_select_complex_expression() {
    let formats = test_formats();
    // "best mp4 video + m4a audio, fallback to best combined mp4, fallback to anything"
    let sel =
        FormatSelector::parse("bv[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4]/best").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].format_id, "v720"); // best mp4 video <=1080
    assert_eq!(result[1].format_id, "a256"); // best m4a audio
}
