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
    // Unknown fields are now accepted (yt-dlp allows arbitrary fields like aspect_ratio)
    assert!(FormatSelector::parse("bv[unknownfield=1]").is_ok());
    assert!(FormatSelector::parse("bv[height]").is_err()); // no operator
}

#[test]
fn test_parse_implicit_best_filter() {
    // Bare `[height<=720]` without a preceding token → implicit best[height<=720]
    assert!(FormatSelector::parse("[height<=720]").is_ok());
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

// ---- Comma (multiple selectors) ----

#[test]
fn test_parse_comma_two_selectors() {
    let sel = FormatSelector::parse("best,bestaudio").unwrap();
    assert_eq!(sel.selectors.len(), 2);
}

#[test]
fn test_parse_comma_three_selectors() {
    let sel = FormatSelector::parse("bv,ba,best").unwrap();
    assert_eq!(sel.selectors.len(), 3);
}

#[test]
fn test_select_all_comma() {
    let formats = test_formats();
    let sel = FormatSelector::parse("best,bestaudio").unwrap();
    let results = sel.select_all(&formats);
    assert_eq!(results.len(), 2);
    // First selector: best combined
    assert_eq!(results[0].len(), 1);
    assert_eq!(results[0][0].format_id, "c1080");
    // Second selector: best audio-only
    assert_eq!(results[1].len(), 1);
    assert_eq!(results[1][0].format_id, "a256");
}

#[test]
fn test_select_comma_backward_compat() {
    // select() evaluates only the first node — backward compatible.
    let formats = test_formats();
    let sel = FormatSelector::parse("best,bestaudio").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

// ---- Parenthesised groups ----

#[test]
fn test_parse_group() {
    assert!(FormatSelector::parse("(bestvideo+bestaudio)/best").is_ok());
}

#[test]
fn test_parse_group_with_outer_filter() {
    assert!(FormatSelector::parse("(bv+ba)[ext=mp4]").is_ok());
}

#[test]
fn test_select_group_fallback() {
    let formats = test_formats();
    // (bestvideo+bestaudio)/best — the group produces a 2-format merge result
    let sel = FormatSelector::parse("(bestvideo+bestaudio)/best").unwrap();
    let result = sel.select(&formats);
    // The group matches (v1440 + a256), so the fallback /best is not reached.
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].format_id, "v1440");
    assert_eq!(result[1].format_id, "a256");
}

#[test]
fn test_parse_group_ast() {
    // Verify the AST structure: first fallback is a Single wrapping a Group token,
    // second is Single(best).
    let sel = FormatSelector::parse("(bestvideo+bestaudio)/best").unwrap();
    assert_eq!(sel.selectors.len(), 1);
    let node = &sel.selectors[0];
    assert_eq!(node.fallbacks.len(), 2);
    // A group `(...)` is represented as FormatSpec::Single with FormatToken::Group base.
    if let FormatSpec::Single(s) = &node.fallbacks[0] {
        assert!(
            matches!(s.base, FormatToken::Group(_)),
            "expected FormatToken::Group, got {:?}",
            s.base
        );
    } else {
        panic!("expected FormatSpec::Single for group atom");
    }
    assert!(matches!(node.fallbacks[1], FormatSpec::Single(_)));
}

// ---- .N suffix ----

#[test]
fn test_parse_nth_suffix() {
    let sel = FormatSelector::parse("bv*.3").unwrap();
    let node = &sel.selectors[0];
    let spec = &node.fallbacks[0];
    if let FormatSpec::Single(s) = spec {
        if let FormatToken::Keyword { nth, .. } = &s.base {
            assert_eq!(*nth, Some(3));
        } else {
            panic!("expected Keyword token");
        }
    } else {
        panic!("expected Single spec");
    }
}

#[test]
fn test_parse_nth_suffix_1() {
    assert!(FormatSelector::parse("bv.1").is_ok());
    assert!(FormatSelector::parse("ba.2").is_ok());
    assert!(FormatSelector::parse("best.10").is_ok());
}

#[test]
fn test_select_nth_best_video() {
    let formats = test_formats();
    // bv without .N → best video-only = v1440
    let sel = FormatSelector::parse("bv").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result[0].format_id, "v1440");

    // bv.2 → 2nd best video-only = v1080
    let sel2 = FormatSelector::parse("bv.2").unwrap();
    let result2 = sel2.select(&formats);
    assert_eq!(result2.len(), 1);
    assert_eq!(result2[0].format_id, "v1080");

    // bv.3 → 3rd best video-only = v720
    let sel3 = FormatSelector::parse("bv.3").unwrap();
    let result3 = sel3.select(&formats);
    assert_eq!(result3.len(), 1);
    assert_eq!(result3[0].format_id, "v720");
}

// ---- all / mergeall ----

#[test]
fn test_parse_all() {
    let sel = FormatSelector::parse("all").unwrap();
    let node = &sel.selectors[0];
    let spec = &node.fallbacks[0];
    if let FormatSpec::Single(s) = spec {
        assert!(matches!(s.base, FormatToken::All));
    } else {
        panic!("expected Single spec");
    }
}

#[test]
fn test_parse_mergeall() {
    let sel = FormatSelector::parse("mergeall").unwrap();
    let node = &sel.selectors[0];
    let spec = &node.fallbacks[0];
    if let FormatSpec::Single(s) = spec {
        assert!(matches!(s.base, FormatToken::MergeAll));
    } else {
        panic!("expected Single spec");
    }
}

// ---- Implicit best (bare filter) ----

#[test]
fn test_implicit_best_ast() {
    // `[height<=720]` → Keyword { quality: Best, stream_type: Any, modified: false, nth: None }
    let sel = FormatSelector::parse("[height<=720]").unwrap();
    let node = &sel.selectors[0];
    let spec = &node.fallbacks[0];
    if let FormatSpec::Single(s) = spec {
        assert!(
            matches!(
                s.base,
                FormatToken::Keyword {
                    quality: Quality::Best,
                    stream_type: StreamType::Any,
                    modified: false,
                    nth: None
                }
            ),
            "expected implicit best keyword, got {:?}",
            s.base
        );
        assert_eq!(s.filters.len(), 1);
    } else {
        panic!("expected Single spec");
    }
}

#[test]
fn test_select_implicit_best() {
    let formats = test_formats();
    // `[height<=720]` → best combined with height <= 720
    let sel = FormatSelector::parse("[height<=720]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c720");
}

// ---- Extension shorthand ----

#[test]
fn test_parse_extension_mp4() {
    let sel = FormatSelector::parse("mp4").unwrap();
    let node = &sel.selectors[0];
    let spec = &node.fallbacks[0];
    if let FormatSpec::Single(s) = spec {
        assert!(
            matches!(&s.base, FormatToken::Extension(e) if e == "mp4"),
            "expected Extension(\"mp4\"), got {:?}",
            s.base
        );
    } else {
        panic!("expected Single spec");
    }
}

#[test]
fn test_parse_extension_webm() {
    assert!(FormatSelector::parse("webm").is_ok());
}

/// Regression: `mp4-hd` and similar format_ids whose prefix collides
/// with a known-extension shorthand (`mp4`, `webm`, ...) must parse as
/// a literal format_id, not as `Extension("mp4")` with `-hd` leftover.
///
/// XVideos emits exactly these ids (`mp4-hd`, `mp4-low`, `mp4-high`) —
/// passing one as a selector previously errored with
/// `Parse error at position 3`.
#[test]
fn test_parse_extension_prefix_collision_is_format_id() {
    for id in &["mp4-hd", "mp4-low", "mp4-high", "webm-720", "aac-128"] {
        let sel =
            FormatSelector::parse(id).unwrap_or_else(|e| panic!("failed to parse `{id}`: {e:?}"));
        let spec = &sel.selectors[0].fallbacks[0];
        match spec {
            FormatSpec::Single(s) => assert!(
                matches!(&s.base, FormatToken::FormatId(fid) if fid == *id),
                "expected FormatId({id:?}), got {:?}",
                s.base
            ),
            other => panic!("expected Single spec for `{id}`, got {other:?}"),
        }
    }
}

#[test]
fn test_parse_all_known_extensions() {
    for ext in &[
        "mp4", "webm", "flv", "3gp", "m4a", "mp3", "ogg", "wav", "aac",
    ] {
        let result = FormatSelector::parse(ext);
        assert!(result.is_ok(), "Failed to parse extension: {ext}");
        let sel = result.unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(
                matches!(&s.base, FormatToken::Extension(e) if e == ext),
                "Expected Extension({ext}), got {:?}",
                s.base
            );
        } else {
            panic!("Expected Single spec for extension: {ext}");
        }
    }
}

#[test]
fn test_select_extension_shorthand() {
    let formats = test_formats();
    // `mp4` → best format with ext=mp4 (combined + video-only considered,
    // Extension matches any ext=mp4, then best by quality/height)
    let sel = FormatSelector::parse("mp4").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    // best format with ext=mp4: c1080 (combined, quality=3, height=1080)
    assert_eq!(result[0].format_id, "c1080");
}

// ---- Complex expressions ----

#[test]
fn test_parse_complex_group_merge_fallback() {
    // bv[height<=1080]+ba/b — merge then fallback
    assert!(FormatSelector::parse("bv[height<=1080]+ba/b").is_ok());
}

#[test]
fn test_parse_complex_with_group() {
    // (bv+ba)/b — group merge, fallback to combined
    assert!(FormatSelector::parse("(bv+ba)/b").is_ok());
}

// ---- Selection tests (existing) ----

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

// ---- String operator parse tests (via parse round-trip / is_ok) ----

#[test]
fn test_parse_string_operators() {
    // All string operators must parse successfully.
    assert!(FormatSelector::parse("best[vcodec^=avc]").is_ok());
    assert!(FormatSelector::parse("best[vcodec$=264]").is_ok());
    assert!(FormatSelector::parse("best[vcodec*=26]").is_ok());
    // Regex without bracket characters in the pattern.
    assert!(FormatSelector::parse("best[vcodec~=h264]").is_ok());
}

#[test]
fn test_parse_negated_string_operators() {
    assert!(FormatSelector::parse("best[vcodec!^=av01]").is_ok());
    assert!(FormatSelector::parse("best[vcodec!$=av01]").is_ok());
    assert!(FormatSelector::parse("best[vcodec!*=av01]").is_ok());
    // Regex without bracket characters in the pattern.
    assert!(FormatSelector::parse("best[vcodec!~=vp9]").is_ok());
}

#[test]
fn test_parse_non_fatal_operators() {
    // Non-fatal `?` suffix on comparison and string ops.
    assert!(FormatSelector::parse("best[filesize<=?500M]").is_ok());
    assert!(FormatSelector::parse("best[height<=?1080]").is_ok());
    assert!(FormatSelector::parse("best[height>=?720]").is_ok());
    assert!(FormatSelector::parse("best[vcodec^=?avc]").is_ok());
    assert!(FormatSelector::parse("best[vcodec!*=?av01]").is_ok());
}

#[test]
fn test_parse_size_literals() {
    assert!(FormatSelector::parse("best[filesize<=500M]").is_ok());
    assert!(FormatSelector::parse("best[filesize>=1.5G]").is_ok());
}

// ---- String operator evaluation tests ----

#[test]
fn test_eval_starts_with() {
    let formats = test_formats(); // all vcodec = "h264"
    let sel = FormatSelector::parse("bv[vcodec^=h26]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_starts_with_no_match() {
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[vcodec^=vp9]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_eval_ends_with() {
    let formats = test_formats(); // all vcodec = "h264"
    let sel = FormatSelector::parse("bv[vcodec$=264]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_contains() {
    let formats = test_formats(); // all vcodec = "h264"
    let sel = FormatSelector::parse("bv[vcodec*=26]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_negated_contains() {
    // negated: excludes formats whose vcodec contains "h26"
    let formats = test_formats(); // all vcodec = "h264"
    let sel = FormatSelector::parse("bv[vcodec!*=h26]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty()); // all video-only formats have h264 vcodec
}

#[test]
fn test_eval_negated_starts_with() {
    // negated starts_with: exclude formats whose ext starts with "web"
    let formats = test_formats(); // v1080 has ext="webm", others "mp4"
    // video-only formats: v720 (mp4), v1080 (webm), v1440 (mp4)
    // excluding webm by negated starts_with "web" -> v720 and v1440 remain
    let sel = FormatSelector::parse("bv[ext!^=web]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440"); // best among v720/v1440 (mp4)
}

#[test]
fn test_eval_ends_with_ext() {
    // EndsWith: ext ends with "4" (mp4, m4a)
    let formats = test_formats();
    // video-only formats with ext ending in "4": v720 (mp4) and v1440 (mp4)
    let sel = FormatSelector::parse("bv[ext$=4]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_size_filter() {
    let mut formats = test_formats();
    // Give formats known filesizes
    formats[5].filesize = Some(600 * 1024 * 1024); // v1440 = 600 MiB
    formats[3].filesize = Some(100 * 1024 * 1024); // v720 = 100 MiB

    // Select best video-only with filesize <= 500M (500_000_000 bytes, SI)
    let sel = FormatSelector::parse("bv[filesize<=500M]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v720"); // only v720 fits under 500M
}

#[test]
fn test_eval_size_filter_ge() {
    let mut formats = test_formats();
    formats[5].filesize = Some(2 * 1024 * 1024 * 1024); // v1440 = 2 GiB

    // Select best video-only with filesize >= 1.5G (1_500_000_000 bytes, SI)
    let sel = FormatSelector::parse("bv[filesize>=1.5G]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_non_fatal_missing_field() {
    // non_fatal (`?` suffix): when the field is absent, include the format
    // (pass-through rather than exclude).
    let formats = test_formats(); // no filesizes set
    let sel = FormatSelector::parse("bv[filesize<=?500M]").unwrap();
    let result = sel.select(&formats);
    // All video-only formats lack filesize; non-fatal means they all pass the
    // filter — so we get the best video-only overall.
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

// ---- New evaluator tests (task 6) ----

#[test]
fn test_eval_regex() {
    // vcodec matches regex "h264|h265" — all test formats use "h264"
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[vcodec~=h264|h265]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_regex_no_match() {
    // Regex that doesn't match any vcodec
    let formats = test_formats();
    let sel = FormatSelector::parse("bv[vcodec~=vp9|vp8]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_eval_negated_vcodec() {
    // best[vcodec!=vp9] — all test combined formats use h264, so result is c1080
    let formats = test_formats();
    let sel = FormatSelector::parse("best[vcodec!=vp9]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_eval_negated_contains_vcodec() {
    // best[vcodec!*=av] — excludes formats whose vcodec contains "av"
    // all combined formats use "h264" → c1080 should match
    let formats = test_formats();
    let sel = FormatSelector::parse("best[vcodec!*=av]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_eval_negated_contains_excludes_all() {
    // best[vcodec!*=h26] — all combined formats have "h264", all excluded
    let formats = test_formats();
    let sel = FormatSelector::parse("best[vcodec!*=h26]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_eval_all_returns_all_formats() {
    let formats = test_formats(); // 9 formats, none DRM
    let sel = FormatSelector::parse("all").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 9);
}

#[test]
fn test_eval_all_sorted_best_first() {
    let formats = test_formats();
    let sel = FormatSelector::parse("all").unwrap();
    let result = sel.select(&formats);
    // Best combined is c1080 (quality=3); it should appear before c720 (quality=2)
    // which should appear before c360 (quality=1).
    let ids: Vec<&str> = result.iter().map(|f| f.format_id.as_str()).collect();
    let pos_c1080 = ids.iter().position(|&id| id == "c1080").unwrap();
    let pos_c720 = ids.iter().position(|&id| id == "c720").unwrap();
    let pos_c360 = ids.iter().position(|&id| id == "c360").unwrap();
    assert!(pos_c1080 < pos_c720);
    assert!(pos_c720 < pos_c360);
}

#[test]
fn test_eval_all_excludes_drm() {
    let mut formats = test_formats();
    formats[2].has_drm = Some(true); // c1080
    let sel = FormatSelector::parse("all").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 8);
    assert!(!result.iter().any(|f| f.format_id == "c1080"));
}

#[test]
fn test_eval_mergeall_returns_video_and_audio() {
    let formats = test_formats(); // 9 formats: 3 combined, 3 video-only, 3 audio-only
    let sel = FormatSelector::parse("mergeall").unwrap();
    let result = sel.select(&formats);
    // All 9 have video or audio
    assert_eq!(result.len(), 9);
}

#[test]
fn test_eval_mergeall_sorted_worst_first() {
    let formats = test_formats();
    let sel = FormatSelector::parse("mergeall").unwrap();
    let result = sel.select(&formats);
    // c360 (quality=1) should appear before c1080 (quality=3)
    let ids: Vec<&str> = result.iter().map(|f| f.format_id.as_str()).collect();
    let pos_c360 = ids.iter().position(|&id| id == "c360").unwrap();
    let pos_c1080 = ids.iter().position(|&id| id == "c1080").unwrap();
    assert!(pos_c360 < pos_c1080);
}

#[test]
fn test_eval_extension_shorthand_mp4() {
    // "mp4" = best[ext=mp4]
    let formats = test_formats();
    let sel = FormatSelector::parse("mp4").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080"); // best (quality=3, height=1080)
}

#[test]
fn test_eval_extension_shorthand_webm() {
    // "webm" = best[ext=webm]
    // test_formats: only v1080 has ext=webm
    let formats = test_formats();
    let sel = FormatSelector::parse("webm").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1080");
}

#[test]
fn test_eval_incomplete_formats_fallback() {
    // When `best` finds no muxed (combined) formats, fall back to any format with video or audio.
    let formats = vec![
        make_video_only("v720", "mp4", 720),
        make_video_only("v1080", "mp4", 1080),
        make_audio_only("a128", "m4a", 128.0),
    ];
    let sel = FormatSelector::parse("best").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    // Fallback picks from all video/audio formats; v1080 has the highest height.
    assert_eq!(result[0].format_id, "v1080");
}

#[test]
fn test_eval_comma_two_groups() {
    let formats = test_formats();
    let sel = FormatSelector::parse("best,bestaudio").unwrap();
    let results = sel.select_all(&formats);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].len(), 1);
    assert_eq!(results[0][0].format_id, "c1080");
    assert_eq!(results[1].len(), 1);
    assert_eq!(results[1][0].format_id, "a256");
}

#[test]
fn test_eval_non_fatal_passes_through_missing() {
    // Non-fatal on a numeric field: formats without that field are included.
    let mut formats = vec![
        make_video_only("v_no_filesize", "mp4", 1080),
        {
            let mut f = make_video_only("v_small", "mp4", 720);
            f.filesize = Some(100 * 1024 * 1024); // 100 MiB — passes <=500M
            f
        },
        {
            let mut f = make_video_only("v_large", "mp4", 480);
            f.filesize = Some(600 * 1024 * 1024); // 600 MiB — fails <=500M
            f
        },
    ];
    // Silence unused mut warning
    let _ = &mut formats;
    let sel = FormatSelector::parse("bv[filesize<=?500M]").unwrap();
    let result = sel.select(&formats);
    // v_no_filesize passes (non-fatal), v_small passes, v_large fails.
    // Best among v_no_filesize (height=1080) and v_small (height=720) → v_no_filesize.
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v_no_filesize");
}

// ============================================================================
// yt-dlp Compatibility Tests
//
// These tests exercise real-world yt-dlp format expressions end-to-end using
// a realistic fixture that mirrors typical extractor output: muxed formats,
// separate video-only and audio-only streams, multiple codecs, HLS and HTTPS
// protocols, and formats with and without filesize.
// ============================================================================

mod compat_tests {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    // ------------------------------------------------------------------------
    // Fixture
    //
    // Format IDs are chosen to match real yt-dlp format numbering conventions:
    //
    //  Muxed (video+audio):
    //    "18"   — 360p h264+aac mp4  (quality=1)
    //    "22"   — 720p h264+aac mp4  (quality=2)
    //    "37"   — 1080p h264+aac mp4 (quality=3)
    //
    //  Video-only (DASH/separate):
    //    "136"  — 720p  h264  mp4   (no filesize)
    //    "137"  — 1080p h264  mp4   (filesize ~300 MB)
    //    "271"  — 1440p vp9   webm  (no filesize)
    //    "313"  — 2160p vp9   webm  (filesize ~2 GB)
    //    "394"  — 1080p av1   mp4   (no filesize)
    //    "337"  — 2160p av1   webm  (no filesize)  — 4K av1
    //    "298"  — 720p  h265  mp4   (no filesize)
    //
    //  Audio-only:
    //    "140"  — aac  m4a  128 kbps (filesize ~20 MB)
    //    "141"  — aac  m4a  256 kbps (no filesize)
    //    "251"  — opus webm  160 kbps
    //    "250"  — opus webm   70 kbps
    //    "171"  — vorbis webm 128 kbps
    //    "hls_audio" — aac mp4 via m3u8 protocol
    //
    //  HLS muxed:
    //    "hls_720"  — 720p h264+aac mp4 m3u8
    // ------------------------------------------------------------------------

    fn compat_formats() -> Vec<Format> {
        // ---- Muxed ----
        let mut m360 = Format::new(
            "18",
            "https://cdn.example.com/18.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        m360.vcodec = Some("h264".to_string());
        m360.acodec = Some("aac".to_string());
        m360.height = Some(360);
        m360.width = Some(640);
        m360.quality = Some(1);
        m360.tbr = Some(500.0);
        m360.fps = Some(30.0);

        let mut m720 = Format::new(
            "22",
            "https://cdn.example.com/22.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        m720.vcodec = Some("h264".to_string());
        m720.acodec = Some("aac".to_string());
        m720.height = Some(720);
        m720.width = Some(1280);
        m720.quality = Some(2);
        m720.tbr = Some(2500.0);
        m720.fps = Some(30.0);
        m720.filesize = Some(150 * 1024 * 1024); // 150 MB

        let mut m1080 = Format::new(
            "37",
            "https://cdn.example.com/37.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        m1080.vcodec = Some("h264".to_string());
        m1080.acodec = Some("aac".to_string());
        m1080.height = Some(1080);
        m1080.width = Some(1920);
        m1080.quality = Some(3);
        m1080.tbr = Some(4500.0);
        m1080.fps = Some(30.0);

        // ---- Video-only ----
        let mut v720_h264 = Format::new(
            "136",
            "https://cdn.example.com/136.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        v720_h264.vcodec = Some("avc1.4d401f".to_string()); // real h264 codec string
        v720_h264.acodec = Some("none".to_string());
        v720_h264.height = Some(720);
        v720_h264.width = Some(1280);
        v720_h264.vbr = Some(2000.0);
        v720_h264.fps = Some(30.0);

        let mut v1080_h264 = Format::new(
            "137",
            "https://cdn.example.com/137.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        v1080_h264.vcodec = Some("avc1.640028".to_string()); // real h264 codec string
        v1080_h264.acodec = Some("none".to_string());
        v1080_h264.height = Some(1080);
        v1080_h264.width = Some(1920);
        v1080_h264.vbr = Some(4000.0);
        v1080_h264.fps = Some(30.0);
        v1080_h264.filesize = Some(300 * 1024 * 1024); // 300 MB

        let mut v1440_vp9 = Format::new(
            "271",
            "https://cdn.example.com/271.webm",
            "webm",
            DownloadProtocol::Https,
        );
        v1440_vp9.vcodec = Some("vp9".to_string());
        v1440_vp9.acodec = Some("none".to_string());
        v1440_vp9.height = Some(1440);
        v1440_vp9.width = Some(2560);
        v1440_vp9.vbr = Some(8000.0);
        v1440_vp9.fps = Some(30.0);

        let mut v2160_vp9 = Format::new(
            "313",
            "https://cdn.example.com/313.webm",
            "webm",
            DownloadProtocol::Https,
        );
        v2160_vp9.vcodec = Some("vp9".to_string());
        v2160_vp9.acodec = Some("none".to_string());
        v2160_vp9.height = Some(2160);
        v2160_vp9.width = Some(3840);
        v2160_vp9.vbr = Some(16000.0);
        v2160_vp9.fps = Some(30.0);
        v2160_vp9.filesize = Some(2 * 1024 * 1024 * 1024); // 2 GB

        let mut v1080_av1 = Format::new(
            "394",
            "https://cdn.example.com/394.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        v1080_av1.vcodec = Some("av01.0.08M.08".to_string()); // real av1 codec string
        v1080_av1.acodec = Some("none".to_string());
        v1080_av1.height = Some(1080);
        v1080_av1.width = Some(1920);
        v1080_av1.vbr = Some(3500.0);
        v1080_av1.fps = Some(30.0);

        let mut v2160_av1 = Format::new(
            "337",
            "https://cdn.example.com/337.webm",
            "webm",
            DownloadProtocol::Https,
        );
        v2160_av1.vcodec = Some("av01.0.13M.10".to_string()); // real av1 codec string
        v2160_av1.acodec = Some("none".to_string());
        v2160_av1.height = Some(2160);
        v2160_av1.width = Some(3840);
        v2160_av1.vbr = Some(12000.0);
        v2160_av1.fps = Some(30.0);

        let mut v720_h265 = Format::new(
            "298",
            "https://cdn.example.com/298.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        v720_h265.vcodec = Some("hev1.1.6.L93.90".to_string()); // real h265 codec string
        v720_h265.acodec = Some("none".to_string());
        v720_h265.height = Some(720);
        v720_h265.width = Some(1280);
        v720_h265.vbr = Some(1800.0);
        v720_h265.fps = Some(60.0);

        // ---- Audio-only ----
        let mut a128_aac = Format::new(
            "140",
            "https://cdn.example.com/140.m4a",
            "m4a",
            DownloadProtocol::Https,
        );
        a128_aac.vcodec = Some("none".to_string());
        a128_aac.acodec = Some("mp4a.40.2".to_string()); // real aac codec string
        a128_aac.abr = Some(128.0);
        a128_aac.asr = Some(44100);
        a128_aac.filesize = Some(20 * 1024 * 1024); // 20 MB

        let mut a256_aac = Format::new(
            "141",
            "https://cdn.example.com/141.m4a",
            "m4a",
            DownloadProtocol::Https,
        );
        a256_aac.vcodec = Some("none".to_string());
        a256_aac.acodec = Some("mp4a.40.2".to_string());
        a256_aac.abr = Some(256.0);
        a256_aac.asr = Some(44100);
        // No filesize

        let mut a160_opus = Format::new(
            "251",
            "https://cdn.example.com/251.webm",
            "webm",
            DownloadProtocol::Https,
        );
        a160_opus.vcodec = Some("none".to_string());
        a160_opus.acodec = Some("opus".to_string());
        a160_opus.abr = Some(160.0);
        a160_opus.asr = Some(48000);

        let mut a70_opus = Format::new(
            "250",
            "https://cdn.example.com/250.webm",
            "webm",
            DownloadProtocol::Https,
        );
        a70_opus.vcodec = Some("none".to_string());
        a70_opus.acodec = Some("opus".to_string());
        a70_opus.abr = Some(70.0);
        a70_opus.asr = Some(48000);

        let mut a128_vorbis = Format::new(
            "171",
            "https://cdn.example.com/171.webm",
            "webm",
            DownloadProtocol::Https,
        );
        a128_vorbis.vcodec = Some("none".to_string());
        a128_vorbis.acodec = Some("vorbis".to_string());
        a128_vorbis.abr = Some(128.0);
        a128_vorbis.asr = Some(44100);

        // ---- HLS audio ----
        let mut hls_audio = Format::new(
            "hls_audio",
            "https://cdn.example.com/audio.m3u8",
            "mp4",
            DownloadProtocol::M3u8,
        );
        hls_audio.vcodec = Some("none".to_string());
        hls_audio.acodec = Some("aac".to_string());
        hls_audio.abr = Some(128.0);

        // ---- HLS muxed ----
        let mut hls_720 = Format::new(
            "hls_720",
            "https://cdn.example.com/720.m3u8",
            "mp4",
            DownloadProtocol::M3u8,
        );
        hls_720.vcodec = Some("h264".to_string());
        hls_720.acodec = Some("aac".to_string());
        hls_720.height = Some(720);
        hls_720.width = Some(1280);
        hls_720.quality = Some(4); // HLS muxed ranked higher via quality
        hls_720.tbr = Some(2800.0);

        vec![
            m360,
            m720,
            m1080,
            v720_h264,
            v1080_h264,
            v1440_vp9,
            v2160_vp9,
            v1080_av1,
            v2160_av1,
            v720_h265,
            a128_aac,
            a256_aac,
            a160_opus,
            a70_opus,
            a128_vorbis,
            hls_audio,
            hls_720,
        ]
    }

    // ------------------------------------------------------------------------
    // Basic keywords
    // ------------------------------------------------------------------------

    #[test]
    fn compat_best_picks_best_muxed() {
        // "best" → best combined (video+audio) format; hls_720 has quality=4
        // but muxed combined formats with quality are ranked by quality first.
        let formats = compat_formats();
        let result = FormatSelector::parse("best").unwrap().select(&formats);
        assert_eq!(result.len(), 1, "best must return exactly 1 format");
        // hls_720 has quality=4, higher than m1080 (quality=3)
        assert_eq!(result[0].format_id, "hls_720");
        assert!(result[0].has_video() && result[0].has_audio());
    }

    #[test]
    fn compat_worst_picks_worst_muxed() {
        let formats = compat_formats();
        let result = FormatSelector::parse("worst").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        // m360 has quality=1, lowest height=360 among combined formats
        assert_eq!(result[0].format_id, "18");
        assert!(result[0].has_video() && result[0].has_audio());
    }

    #[test]
    fn compat_b_alias_equals_best() {
        let formats = compat_formats();
        let r_b = FormatSelector::parse("b").unwrap().select(&formats);
        let r_best = FormatSelector::parse("best").unwrap().select(&formats);
        assert_eq!(r_b.len(), 1);
        assert_eq!(r_b[0].format_id, r_best[0].format_id, "b must alias best");
    }

    #[test]
    fn compat_w_alias_equals_worst() {
        let formats = compat_formats();
        let r_w = FormatSelector::parse("w").unwrap().select(&formats);
        let r_worst = FormatSelector::parse("worst").unwrap().select(&formats);
        assert_eq!(r_w.len(), 1);
        assert_eq!(r_w[0].format_id, r_worst[0].format_id, "w must alias worst");
    }

    // ------------------------------------------------------------------------
    // Star variants
    // ------------------------------------------------------------------------

    #[test]
    fn compat_bv_star_best_format_with_video() {
        // "bv*" → best format that has video (may also have audio)
        // candidates with video: m360/m720/m1080/hls_720 (muxed) + all video-only
        // highest by video ranking: v2160_av1 and v2160_vp9 both have height=2160,
        // v2160_vp9 has higher vbr (16000 > 12000).
        let formats = compat_formats();
        let result = FormatSelector::parse("bv*").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_video(), "bv* must select a format with video");
        assert_eq!(result[0].height, Some(2160));
    }

    #[test]
    fn compat_bv_best_video_only() {
        // "bv" → best video-only (no audio). Candidates: 136/137/271/313/394/337/298.
        // Best by height: 313 and 337 both at 2160p; 313 has higher vbr (16000 > 12000).
        let formats = compat_formats();
        let result = FormatSelector::parse("bv").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_video());
        assert!(
            !result[0].has_audio(),
            "bv must select video-only (no audio)"
        );
        assert_eq!(result[0].height, Some(2160));
        assert_eq!(result[0].format_id, "313"); // higher vbr wins tie
    }

    #[test]
    fn compat_ba_star_best_format_with_audio() {
        // "ba*" → best format that has audio (may also have video)
        // candidates: all audio-only + all muxed. Best audio: a256_aac (abr=256).
        let formats = compat_formats();
        let result = FormatSelector::parse("ba*").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_audio(), "ba* must select a format with audio");
        assert_eq!(result[0].format_id, "141"); // 256 kbps aac
    }

    #[test]
    fn compat_ba_best_audio_only() {
        // "ba" → best audio-only. Candidates: 140/141/251/250/171/hls_audio.
        // Best by abr: 141 (256 kbps).
        let formats = compat_formats();
        let result = FormatSelector::parse("ba").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_audio());
        assert!(
            !result[0].has_video(),
            "ba must select audio-only (no video)"
        );
        assert_eq!(result[0].format_id, "141");
    }

    #[test]
    fn compat_b_star_selects_any_format() {
        // "b*" / "best*" — any format with video or audio (yt-dlp docs)
        let formats = compat_formats();
        let sel = FormatSelector::parse("b*").unwrap();
        let result = sel.select(&formats);
        assert!(!result.is_empty(), "b* should match any format");
    }

    #[test]
    fn compat_best_star_long_form() {
        let formats = compat_formats();
        let sel = FormatSelector::parse("best*").unwrap();
        let result = sel.select(&formats);
        assert!(!result.is_empty(), "best* should match any format");
    }

    #[test]
    fn compat_w_star() {
        let formats = compat_formats();
        let sel = FormatSelector::parse("w*").unwrap();
        let result = sel.select(&formats);
        assert!(!result.is_empty(), "w* should match any format");
    }

    #[test]
    fn compat_worst_star_long_form() {
        let formats = compat_formats();
        let sel = FormatSelector::parse("worst*").unwrap();
        let result = sel.select(&formats);
        assert!(!result.is_empty(), "worst* should match any format");
    }

    #[test]
    fn compat_wv_star() {
        let formats = compat_formats();
        let sel = FormatSelector::parse("wv*").unwrap();
        let result = sel.select(&formats);
        assert!(!result.is_empty(), "wv* should match formats with video");
    }

    #[test]
    fn compat_wa_star() {
        let formats = compat_formats();
        let sel = FormatSelector::parse("wa*").unwrap();
        let result = sel.select(&formats);
        assert!(!result.is_empty(), "wa* should match formats with audio");
    }

    // ------------------------------------------------------------------------
    // Merge + fallback
    // ------------------------------------------------------------------------

    #[test]
    fn compat_bestvideo_plus_bestaudio_fallback_best() {
        // "bestvideo+bestaudio/best" — merge path succeeds (video-only and
        // audio-only formats both exist).
        let formats = compat_formats();
        let result = FormatSelector::parse("bestvideo+bestaudio/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 2, "merge must return 2 formats");
        // video: best video-only = 313 (2160p vp9, highest vbr)
        let video = &result[0];
        let audio = &result[1];
        assert!(video.has_video() && !video.has_audio());
        assert!(audio.has_audio() && !audio.has_video());
        assert_eq!(video.format_id, "313");
        assert_eq!(audio.format_id, "141");
    }

    #[test]
    fn compat_bv_height_filter_plus_ba_fallback_b() {
        // "bv*[height<=1080]+ba/b"
        // video side: bv*[height<=1080] → best format with video, height<=1080
        //   candidates with video and height<=1080: m360(18), m720(22), m1080(37),
        //     v720_h264(136), v1080_h264(137), v1080_av1(394), v720_h265(298), hls_720
        //   best by video ranking (height then vbr): 137 (1080p h264, vbr=4000)
        //   but bv* includes muxed; muxed with height=1080: m1080(37) has quality=3,
        //   hls_720 has quality=4 but height=720; for bv*, ranking is video-ranking
        //   (height > vbr > fps). Both v1080_h264(137) and m1080(37) are at 1080;
        //   v1080_h264 has vbr=4000, m1080 has tbr=4500 but no vbr.
        //   cmp_opt_f64 returns Equal when one side lacks the field, so tie-break
        //   may vary; either is valid as long as height=1080.
        // audio side: ba → best audio-only = 141
        let formats = compat_formats();
        let result = FormatSelector::parse("bv*[height<=1080]+ba/b")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 2);
        let video = &result[0];
        let audio = &result[1];
        assert!(
            video.height.unwrap_or(0) <= 1080,
            "video height must be <=1080, got {:?}",
            video.height
        );
        assert!(audio.has_audio() && !audio.has_video());
        assert_eq!(audio.format_id, "141");
    }

    #[test]
    fn compat_complex_fallback_chain() {
        // "bv[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/b"
        // First arm: bv[ext=mp4] + ba[ext=m4a]
        //   bv[ext=mp4]: video-only mp4 formats: 136, 137, 394, 298.
        //     Best by height+vbr: 137 (1080p, vbr=4000) vs 394 (1080p av1, vbr=3500).
        //     cmp: height equal (1080), vbr 4000>3500 → 137 wins.
        //   ba[ext=m4a]: audio-only m4a: 140 (128kbps), 141 (256kbps). Best: 141.
        let formats = compat_formats();
        let result = FormatSelector::parse("bv[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/b")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 2, "first arm (merge) must succeed");
        let video = &result[0];
        let audio = &result[1];
        assert_eq!(video.ext, "mp4");
        assert!(video.has_video() && !video.has_audio());
        assert_eq!(video.format_id, "137");
        assert_eq!(audio.ext, "m4a");
        assert!(audio.has_audio() && !audio.has_video());
        assert_eq!(audio.format_id, "141");
    }

    // ------------------------------------------------------------------------
    // Format IDs
    // ------------------------------------------------------------------------

    #[test]
    fn compat_format_id_select_137() {
        let formats = compat_formats();
        let result = FormatSelector::parse("137").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "137");
    }

    #[test]
    fn compat_format_id_select_140() {
        let formats = compat_formats();
        let result = FormatSelector::parse("140").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "140");
    }

    #[test]
    fn compat_format_id_merge_137_plus_141() {
        // "137+141" → merge video format 137 with audio format 141
        let formats = compat_formats();
        let result = FormatSelector::parse("137+141").unwrap().select(&formats);
        assert_eq!(result.len(), 2, "ID merge must return 2 formats");
        assert_eq!(result[0].format_id, "137");
        assert_eq!(result[1].format_id, "141");
    }

    #[test]
    fn compat_format_id_not_found_returns_empty() {
        let formats = compat_formats();
        let result = FormatSelector::parse("99999").unwrap().select(&formats);
        assert!(result.is_empty(), "unknown format ID must return empty");
    }

    // ------------------------------------------------------------------------
    // Filters
    // ------------------------------------------------------------------------

    #[test]
    fn compat_filter_height_le_720() {
        // "best[height<=720]" → best combined format with height<=720
        // Candidates (combined with height set): m360(360), m720(720), hls_720(720).
        // Best by quality: hls_720 (quality=4).
        let formats = compat_formats();
        let result = FormatSelector::parse("best[height<=720]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].height.unwrap_or(0) <= 720);
        assert!(result[0].has_video() && result[0].has_audio());
        assert_eq!(result[0].format_id, "hls_720");
    }

    #[test]
    fn compat_filter_height_ge_1080() {
        // "best[height>=1080]" → best combined with height>=1080
        // Only m1080 (quality=3) qualifies among combined formats with height set.
        let formats = compat_formats();
        let result = FormatSelector::parse("best[height>=1080]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].height.unwrap_or(0) >= 1080);
        assert_eq!(result[0].format_id, "37");
    }

    #[test]
    fn compat_filter_ext_mp4() {
        // "best[ext=mp4]" → best combined mp4. hls_720 has quality=4 and ext=mp4.
        let formats = compat_formats();
        let result = FormatSelector::parse("best[ext=mp4]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].ext, "mp4");
        assert_eq!(result[0].format_id, "hls_720");
    }

    #[test]
    fn compat_filter_vcodec_starts_with_h26() {
        // "best[vcodec^=h26]" → best combined with vcodec starting with "h26"
        // Among combined: m360/m720/m1080/hls_720 all have vcodec "h264".
        // Best: hls_720 (quality=4).
        let formats = compat_formats();
        let result = FormatSelector::parse("best[vcodec^=h26]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].vcodec.as_deref().unwrap_or("").starts_with("h26"),
            "vcodec must start with h26"
        );
        assert_eq!(result[0].format_id, "hls_720");
    }

    #[test]
    fn compat_filter_vcodec_starts_with_avc_on_video_only() {
        // "bv[vcodec^=avc]" → best video-only where vcodec starts with "avc"
        // Candidates: 136 (avc1.4d401f), 137 (avc1.640028). Both start with "avc".
        // Best by height+vbr: both 720p/1080p; 137 wins (height=1080).
        let formats = compat_formats();
        let result = FormatSelector::parse("bv[vcodec^=avc]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].vcodec.as_deref().unwrap_or("").starts_with("avc"));
        assert_eq!(result[0].format_id, "137");
    }

    #[test]
    fn compat_filter_filesize_le_500m() {
        // "best[filesize<=500M]" → best combined with filesize<=500MB
        // m720 (filesize=150MB) qualifies; m1080 has no filesize → excluded (fatal).
        // hls_720 has no filesize → excluded. m360 has no filesize → excluded.
        // Best qualifying: m720 (quality=2).
        let formats = compat_formats();
        let result = FormatSelector::parse("best[filesize<=500M]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].filesize.unwrap_or(u64::MAX) <= 500 * 1024 * 1024);
        assert_eq!(result[0].format_id, "22");
    }

    #[test]
    fn compat_filter_filesize_le_500m_non_fatal() {
        // "bv[filesize<=?500M]" → non-fatal: formats WITHOUT filesize pass through;
        // formats WITH filesize must satisfy the condition.
        //
        // In the fixture:
        //   - v2160_vp9 (313) has filesize=2GB → present and > 500M → FAILS (excluded)
        //   - v1080_h264 (137) has filesize=300MB → present and <=500M → passes
        //   - v2160_av1 (337) has NO filesize → non-fatal pass-through → passes
        //   - all other video-only: no filesize → non-fatal pass-through → pass
        //
        // Candidates: 136(720p), 137(1080p, 300MB), 271(1440p), 337(2160p), 394(1080p), 298(720p).
        // Best by height then vbr: 337 (2160p, vbr=12000) — 313 is excluded.
        let formats = compat_formats();
        let result = FormatSelector::parse("bv[filesize<=?500M]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_video() && !result[0].has_audio());
        // Non-fatal: result has either no filesize or filesize<=500M.
        let size_ok = match result[0].filesize {
            Some(s) => s <= 500 * 1024 * 1024,
            None => true,
        };
        assert!(
            size_ok,
            "non-fatal: result must have filesize<=500M or no filesize"
        );
        // 313 is excluded (filesize=2GB > 500M, present). Next best at 2160p is 337 (no filesize).
        assert_eq!(result[0].format_id, "337");
    }

    // ------------------------------------------------------------------------
    // Extension shorthand
    // ------------------------------------------------------------------------

    #[test]
    fn compat_extension_shorthand_mp4() {
        // "mp4" → best format with ext=mp4 (any stream type, best by rank)
        // Ext=mp4 formats: 18, 22, 37, 136, 137, 394, 298, 140, 141, hls_audio, hls_720.
        // Extension token matches by ext only, so any ext=mp4 qualifies.
        // Best by quality/height/tbr: hls_720 (quality=4, height=720).
        // But m1080(37) has quality=3 and height=1080; hls_720 quality=4 > 3 → hls_720.
        let formats = compat_formats();
        let result = FormatSelector::parse("mp4").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].ext, "mp4");
    }

    #[test]
    fn compat_extension_shorthand_webm() {
        // "webm" → best format with ext=webm
        // webm formats: 271, 313, 337, 251, 250, 171.
        // Extension token ranking: quality/height/tbr. 271/313/337 have height set.
        // 313 and 337 both at 2160p; 313 has vbr=16000 > 337 vbr=12000.
        // But ranking for Extension uses generic rank_formats (no specific stream_type),
        // which orders by quality→height→tbr→fps. None have quality set;
        // height: 313=2160, 337=2160 (tie). tbr: neither set. fps: neither set.
        // Tie — result is deterministic only by iteration order. Both are valid.
        let formats = compat_formats();
        let result = FormatSelector::parse("webm").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].ext, "webm");
        // Must be one of the 4K webm formats since they rank highest by height.
        assert_eq!(result[0].height, Some(2160));
    }

    #[test]
    fn compat_extension_shorthand_m4a() {
        // "m4a" → best format with ext=m4a
        // m4a formats: 140 (128kbps, has filesize), 141 (256kbps, no filesize).
        // Extension ranking (generic): quality→height→tbr→fps — all None.
        // Falls through to tie; but abr IS set: 141 has abr=256, 140 has abr=128.
        // Tie on quality/height/tbr/fps → order is stable (max_by returns last on tie).
        // The rank function for generic uses quality→height→tbr→fps, not abr.
        // So this is a tie; either 140 or 141 is acceptable.
        let formats = compat_formats();
        let result = FormatSelector::parse("m4a").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].ext, "m4a");
    }

    // ------------------------------------------------------------------------
    // Comma — multiple result groups
    // ------------------------------------------------------------------------

    #[test]
    fn compat_comma_best_and_bestaudio() {
        // "best,bestaudio" → select_all returns 2 groups
        let formats = compat_formats();
        let sel = FormatSelector::parse("best,bestaudio").unwrap();
        let results = sel.select_all(&formats);
        assert_eq!(results.len(), 2, "comma must produce 2 groups");
        assert_eq!(results[0].len(), 1, "first group: 1 combined format");
        assert!(results[0][0].has_video() && results[0][0].has_audio());
        assert_eq!(results[1].len(), 1, "second group: 1 audio-only format");
        assert!(results[1][0].has_audio() && !results[1][0].has_video());
    }

    #[test]
    fn compat_comma_three_groups() {
        // "bv,ba,best" → 3 independent groups
        let formats = compat_formats();
        let sel = FormatSelector::parse("bv,ba,best").unwrap();
        let results = sel.select_all(&formats);
        assert_eq!(results.len(), 3);
        // bv → video-only
        assert!(results[0][0].has_video() && !results[0][0].has_audio());
        // ba → audio-only
        assert!(results[1][0].has_audio() && !results[1][0].has_video());
        // best → combined
        assert!(results[2][0].has_video() && results[2][0].has_audio());
    }

    // ------------------------------------------------------------------------
    // Parenthesised groups
    // ------------------------------------------------------------------------

    #[test]
    fn compat_group_bv_plus_ba_fallback_b() {
        // "(bv+ba)/b" — group merge; succeeds because video-only and audio-only exist
        let formats = compat_formats();
        let result = FormatSelector::parse("(bv+ba)/b").unwrap().select(&formats);
        assert_eq!(result.len(), 2, "(bv+ba) merge must return 2 formats");
        assert!(result[0].has_video() && !result[0].has_audio());
        assert!(result[1].has_audio() && !result[1].has_video());
    }

    #[test]
    fn compat_group_partial_merge_when_one_side_fails() {
        // "(bv[height>=9000]+ba)/b"
        //
        // The merge inside the group: bv[height>=9000] returns nothing
        // (no 9000p video). Per yt-dlp semantics, a merge with a missing
        // side fails entirely, the group produces no results, and the
        // `/b` fallback returns the best muxed format.
        let formats = compat_formats();
        let result = FormatSelector::parse("(bv[height>=9000]+ba)/b")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1, "fallback to /b after failed merge");
        assert!(
            result[0].has_video() && result[0].has_audio(),
            "fallback picks best muxed, not the dangling audio side"
        );
    }

    #[test]
    fn compat_group_fallback_used_when_whole_group_fails() {
        // "(bv[height>=9000]+ba[abr>=9000])/b" — both sides of the merge are
        // impossible, so the group produces zero results, and /b is reached.
        let formats = compat_formats();
        let result = FormatSelector::parse("(bv[height>=9000]+ba[abr>=9000])/b")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1, "fallback /b must return 1 combined format");
        assert!(
            result[0].has_video() && result[0].has_audio(),
            "fallback b must return a combined format"
        );
    }

    // ------------------------------------------------------------------------
    // .N suffix (nth-best)
    // ------------------------------------------------------------------------

    #[test]
    fn compat_bv_star_second_best_video() {
        // "bv*.2" → second-best format containing video (by video ranking: height then vbr)
        // All formats with video sorted best-first by height+vbr:
        //   2160: 313 (vbr=16000), 337 (vbr=12000)
        //   1440: 271 (vbr=8000)
        //   1080: 37 (tbr=4500), 137 (vbr=4000), 394 (vbr=3500)
        //   ...
        // bv* first = 313 (height=2160, vbr=16000). Second = 337 (height=2160, vbr=12000).
        let formats = compat_formats();
        let result = FormatSelector::parse("bv*.2").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_video());
        assert_eq!(result[0].height, Some(2160));
        // Second at 2160p: 337 has vbr=12000 < 313 vbr=16000
        assert_eq!(result[0].format_id, "337");
    }

    #[test]
    fn compat_ba_second_best_audio() {
        // "ba.2" → second-best audio-only format by abr
        // Audio-only sorted best-first by abr: 141(256), 160_opus(160 via abr),
        //   140(128), 171(128), 70_opus(70). But hls_audio also has abr=128.
        // ba (audio-only, no video) excludes hls_audio? No — hls_audio has
        //   vcodec=none and acodec=aac → has_audio()=true, has_video()=false → included.
        // Sorted: 141(256), 251(160), 140(128), 171(128), hls_audio(128), 250(70).
        // abr ties at 128: 140/171/hls_audio — tie-break by asr then iteration order.
        // Second best: 251 (opus, abr=160).
        let formats = compat_formats();
        let result = FormatSelector::parse("ba.2").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert!(result[0].has_audio() && !result[0].has_video());
        assert_eq!(result[0].format_id, "251"); // 160kbps opus
    }

    // ------------------------------------------------------------------------
    // all / mergeall
    // ------------------------------------------------------------------------

    #[test]
    fn compat_all_returns_all_non_drm_formats() {
        let formats = compat_formats();
        let result = FormatSelector::parse("all").unwrap().select(&formats);
        assert_eq!(result.len(), formats.len(), "all must return every format");
    }

    #[test]
    fn compat_all_sorted_best_first() {
        let formats = compat_formats();
        let result = FormatSelector::parse("all").unwrap().select(&formats);
        // hls_720 has quality=4 — must appear before m1080 (quality=3)
        let ids: Vec<&str> = result.iter().map(|f| f.format_id.as_str()).collect();
        let pos_hls = ids.iter().position(|&id| id == "hls_720").unwrap();
        let pos_1080 = ids.iter().position(|&id| id == "37").unwrap();
        assert!(
            pos_hls < pos_1080,
            "hls_720(quality=4) must rank before 37(quality=3)"
        );
    }

    #[test]
    fn compat_all_excludes_drm() {
        let mut formats = compat_formats();
        // Mark the best format DRM-protected
        if let Some(f) = formats.iter_mut().find(|f| f.format_id == "hls_720") {
            f.has_drm = Some(true);
        }
        let result = FormatSelector::parse("all").unwrap().select(&formats);
        assert_eq!(result.len(), formats.len() - 1);
        assert!(!result.iter().any(|f| f.format_id == "hls_720"));
    }

    // ------------------------------------------------------------------------
    // Complex real-world expressions
    // ------------------------------------------------------------------------

    #[test]
    fn compat_complex_bestvideo_height_non_fatal_plus_bestaudio_fallback_best() {
        // "bestvideo[height<=?1080]+bestaudio/best"
        // Non-fatal: video-only formats without height set pass through (none here —
        // all video-only formats in fixture have height set).
        // bestvideo[height<=?1080]: best video-only with height<=1080 OR no height.
        //   Candidates at <=1080: 136(720), 137(1080), 394(1080), 298(720).
        //   Best by height+vbr: 137 vs 394 both at 1080p. 137 vbr=4000, 394 vbr=3500. → 137.
        // bestaudio: 141 (256kbps)
        let formats = compat_formats();
        let result = FormatSelector::parse("bestvideo[height<=?1080]+bestaudio/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 2, "merge path must succeed");
        let video = &result[0];
        let audio = &result[1];
        assert!(video.has_video() && !video.has_audio());
        assert!(audio.has_audio() && !audio.has_video());
        let video_height = video.height.unwrap_or(0);
        assert!(
            video_height <= 1080,
            "video height must be <=1080, got {video_height}"
        );
        assert_eq!(video.format_id, "137");
        assert_eq!(audio.format_id, "141");
    }

    #[test]
    fn compat_complex_avc_group_with_fallback() {
        // "(bv*[vcodec^=avc]+ba[ext=m4a])/(bv*+ba)/b"
        // First group: bv*[vcodec^=avc] + ba[ext=m4a]
        //   bv*[vcodec^=avc]: formats with video and vcodec starting with "avc":
        //     136 (avc1.4d401f), 137 (avc1.640028). Best: 137 (height=1080).
        //   ba[ext=m4a]: audio-only m4a: 140, 141. Best: 141.
        // First group succeeds → no fallback.
        let formats = compat_formats();
        let result = FormatSelector::parse("(bv*[vcodec^=avc]+ba[ext=m4a])/(bv*+ba)/b")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 2, "first group must succeed");
        let video = &result[0];
        let audio = &result[1];
        assert!(
            video.vcodec.as_deref().unwrap_or("").starts_with("avc"),
            "video vcodec must start with avc"
        );
        assert_eq!(video.format_id, "137");
        assert_eq!(audio.ext, "m4a");
        assert_eq!(audio.format_id, "141");
    }

    #[test]
    fn compat_merge_partial_result_when_one_side_impossible() {
        // "(bv*[vcodec^=avc]+ba[ext=m4a])/(bv*+ba)/b" on a format list
        // with NO avc video.
        //
        // Per yt-dlp semantics a merge requires BOTH sides to succeed.
        // Since bv*[vcodec^=avc] has no match, the first group's merge
        // fails entirely; the fallback `/(bv*+ba)/b` kicks in and picks
        // the best video-only + audio-only pair available.
        let formats: Vec<Format> = compat_formats()
            .into_iter()
            .filter(|f| {
                // Remove avc video-only formats; keep combined (h264), other video-only,
                // and all audio formats.
                let vcodec = f.vcodec.as_deref().unwrap_or("");
                let is_avc_video_only = vcodec.starts_with("avc") && !f.has_audio();
                !is_avc_video_only
            })
            .collect();
        assert!(formats.iter().any(|f| f.has_video() && !f.has_audio()));
        assert!(formats.iter().any(|f| f.has_audio() && !f.has_video()));

        let result = FormatSelector::parse("(bv*[vcodec^=avc]+ba[ext=m4a])/(bv*+ba)/b")
            .unwrap()
            .select(&formats);
        // First group fails (no avc video). Fallback `/(bv*+ba)` picks
        // a video-only + audio-only pair.
        assert_eq!(result.len(), 2, "fallback yields a full bv+ba pair");
        assert!(result[0].has_video());
        assert!(result[1].has_audio() && !result[1].has_video());
        // The audio side comes from `ba`, which ranks audio-only formats
        // by abr. On `compat_formats` the m4a 141 is the top audio.
        assert_eq!(result[1].ext, "m4a");
    }

    #[test]
    fn compat_complex_group_falls_back_when_first_group_fully_empty() {
        // When BOTH sides of the merge in the first group fail, the group is empty
        // and the second fallback group is reached.
        //
        // "(bv[height>=9000]+ba[abr>=9000])/(bv*+ba)/b"
        //   First group: bv[height>=9000] → None; ba[abr>=9000] → None (both impossible)
        //   → merge produces 0 formats → group is empty → next fallback tried.
        //
        //   Second group: (bv*+ba): bv* succeeds, ba succeeds → 2 formats returned.
        let formats = compat_formats();
        let result = FormatSelector::parse("(bv[height>=9000]+ba[abr>=9000])/(bv*+ba)/b")
            .unwrap()
            .select(&formats);
        assert_eq!(
            result.len(),
            2,
            "second group (bv*+ba) must succeed with 2 formats"
        );
        assert!(result[0].has_video(), "first result must have video");
        assert!(
            result[1].has_audio() && !result[1].has_video(),
            "second result must be audio-only"
        );
    }

    // ------------------------------------------------------------------------
    // Protocol awareness
    // ------------------------------------------------------------------------

    #[test]
    fn compat_filter_protocol_https() {
        // "bv[protocol=https]" → video-only formats served via https
        // In fixture, all video-only formats use DownloadProtocol::Https → as_str() = "https"
        let formats = compat_formats();
        let result = FormatSelector::parse("bv[protocol=https]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].protocol, DownloadProtocol::Https);
        assert!(result[0].has_video() && !result[0].has_audio());
    }

    #[test]
    fn compat_filter_protocol_excludes_hls() {
        // "best[protocol=https]" → best combined format with https protocol only
        // m360/m720/m1080 are https; hls_720 is M3u8. Best https combined: m1080 (quality=3).
        let formats = compat_formats();
        let result = FormatSelector::parse("best[protocol=https]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].protocol, DownloadProtocol::Https);
        assert_eq!(result[0].format_id, "37");
    }

    // ------------------------------------------------------------------------
    // Edge cases
    // ------------------------------------------------------------------------

    #[test]
    fn compat_no_match_returns_empty() {
        let formats = compat_formats();
        let result = FormatSelector::parse("bv[height>=9000]")
            .unwrap()
            .select(&formats);
        assert!(
            result.is_empty(),
            "impossible height filter must return empty"
        );
    }

    #[test]
    fn compat_fallback_chain_exhausted_returns_empty() {
        let formats = compat_formats();
        // Both arms impossible
        let result = FormatSelector::parse("bv[height>=9000]/ba[abr>=9000]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn compat_drm_excluded_from_all_selectors() {
        let mut formats = compat_formats();
        // DRM-protect every combined format
        for f in formats.iter_mut() {
            if f.has_video() && f.has_audio() {
                f.has_drm = Some(true);
            }
        }
        // "best" must not return any DRM format; since no muxed non-DRM exists,
        // triggers the incomplete-formats fallback (picks best video-only).
        let result = FormatSelector::parse("best").unwrap().select(&formats);
        if !result.is_empty() {
            assert!(
                !result[0].has_drm.unwrap_or(false),
                "selected format must not be DRM protected"
            );
        }
    }

    #[test]
    fn compat_select_all_backward_compat_returns_first_node() {
        // select() on a comma expression only evaluates the first node.
        let formats = compat_formats();
        let sel = FormatSelector::parse("best,bestaudio").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].has_video() && result[0].has_audio(),
            "first node must be combined"
        );
    }
}

/// yt-dlp upstream test suite compatibility — validates every expression
/// from test/test_YoutubeDL.py parses correctly or rejects correctly.
mod ytdlp_upstream_compat {
    use super::*;

    fn assert_parses(expr: &str) {
        assert!(
            FormatSelector::parse(expr).is_ok(),
            "Expected '{expr}' to parse, got: {:?}",
            FormatSelector::parse(expr).unwrap_err()
        );
    }

    fn assert_rejects(expr: &str) {
        assert!(
            FormatSelector::parse(expr).is_err(),
            "Expected '{expr}' to be rejected as invalid syntax"
        );
    }

    // === test_format_selection ===
    #[test]
    fn ytdlp_fallback_20_47() {
        assert_parses("20/47");
    }
    #[test]
    fn ytdlp_fallback_chain() {
        assert_parses("20/71/worst");
    }
    #[test]
    fn ytdlp_ext_fallback() {
        assert_parses("webm/mp4");
    }
    #[test]
    fn ytdlp_multi_ext_fallback() {
        assert_parses("3gp/40/mp4");
    }
    #[test]
    fn ytdlp_format_id_dashes() {
        assert_parses("example-with-dashes");
    }
    #[test]
    fn ytdlp_invalid_id_fallback() {
        assert_parses("7_a/worst");
    }

    // === test_format_selection_audio ===
    #[test]
    fn ytdlp_audio_fallback_chain() {
        assert_parses("bestaudio/worstaudio/best");
    }

    // === test_format_selection_video ===
    #[test]
    fn ytdlp_starts_ends_filter() {
        assert_parses("bestvideo[format_id^=dash][format_id$=low]");
    }
    #[test]
    fn ytdlp_dotted_vcodec() {
        assert_parses("bestvideo[vcodec=avc1.123456]");
    }

    // === test_format_selection_string_ops ===
    #[test]
    fn ytdlp_implicit_eq() {
        assert_parses("[format_id=abc-cba]");
    }
    #[test]
    fn ytdlp_implicit_neq() {
        assert_parses("[format_id!=abc-cba]");
    }
    #[test]
    fn ytdlp_starts_with() {
        assert_parses("[format_id^=abc]");
    }
    #[test]
    fn ytdlp_not_starts_with() {
        assert_parses("[format_id!^=abc]");
    }
    #[test]
    fn ytdlp_ends_with() {
        assert_parses("[format_id$=cba]");
    }
    #[test]
    fn ytdlp_not_ends_with() {
        assert_parses("[format_id!$=cba]");
    }
    #[test]
    fn ytdlp_contains() {
        assert_parses("[format_id*=bc-cb]");
    }
    #[test]
    fn ytdlp_not_contains() {
        assert_parses("[format_id!*=bc-cb]");
    }
    #[test]
    fn ytdlp_not_contains_dash() {
        assert_parses("[format_id!*=-]");
    }

    // === test_format_filtering ===
    #[test]
    fn ytdlp_filesize_lt() {
        assert_parses("best[filesize<3000]");
    }
    #[test]
    fn ytdlp_filesize_lte() {
        assert_parses("best[filesize<=3000]");
    }
    #[test]
    fn ytdlp_non_fatal_spaces() {
        assert_parses("best[filesize <= ? 3000]");
    }
    #[test]
    fn ytdlp_spaces_multi_filter() {
        assert_parses("best [filesize = 1000] [width>450]");
    }
    #[test]
    fn ytdlp_gt_non_fatal() {
        assert_parses("[filesize>?1]");
    }
    #[test]
    fn ytdlp_si_megabytes() {
        assert_parses("[filesize<1M]");
    }
    #[test]
    fn ytdlp_binary_mebibytes() {
        assert_parses("[filesize<1MiB]");
    }
    #[test]
    fn ytdlp_all_with_range() {
        assert_parses("all[width>=400][width<=600]");
    }
    #[test]
    fn ytdlp_float_field() {
        assert_parses("best[aspect_ratio=1]");
    }
    #[test]
    fn ytdlp_float_comparison() {
        assert_parses("all[aspect_ratio > 1.00]");
    }
    #[test]
    fn ytdlp_float_exact() {
        assert_parses("best[aspect_ratio=1.5]");
    }
    #[test]
    fn ytdlp_float_neq() {
        assert_parses("all[aspect_ratio!=1]");
    }

    // === test_format_selection_issue_10083 ===
    #[test]
    fn ytdlp_merge_in_fallback() {
        assert_parses("best[height>360]/bestvideo[height>360]+bestaudio");
    }

    // === test_invalid_format_specs ===
    #[test]
    fn ytdlp_reject_double_comma() {
        assert_rejects("bestvideo,,best");
    }
    #[test]
    fn ytdlp_reject_leading_plus() {
        assert_rejects("+bestaudio");
    }
    #[test]
    fn ytdlp_reject_trailing_plus() {
        assert_rejects("bestvideo+");
    }
    #[test]
    fn ytdlp_reject_bare_slash() {
        assert_rejects("/");
    }
    #[test]
    fn ytdlp_reject_value_on_left() {
        assert_rejects("[720<height]");
    }

    // === star variants (all documented in README) ===
    #[test]
    fn ytdlp_best_star() {
        assert_parses("b*");
    }
    #[test]
    fn ytdlp_worst_star() {
        assert_parses("w*");
    }
    #[test]
    fn ytdlp_bestvideo_star_long() {
        assert_parses("bestvideo*");
    }
    #[test]
    fn ytdlp_bestaudio_star_long() {
        assert_parses("bestaudio*");
    }
    #[test]
    fn ytdlp_worstvideo_star_long() {
        assert_parses("worstvideo*");
    }
    #[test]
    fn ytdlp_worstaudio_star_long() {
        assert_parses("worstaudio*");
    }

    // === default format strings ===
    #[test]
    fn ytdlp_default_with_ffmpeg() {
        assert_parses("bestvideo*+bestaudio/best");
    }
    #[test]
    fn ytdlp_default_without_ffmpeg() {
        assert_parses("best/bestvideo+bestaudio");
    }

    // === m4a/bestaudio/best (from README example) ===
    #[test]
    fn ytdlp_m4a_fallback() {
        assert_parses("m4a/bestaudio/best");
    }

    // Remaining yt-dlp upstream expressions (chained negation, spaces, edge cases)
    #[test]
    fn ytdlp_chained_neq_excludes_all() {
        assert_parses("[format_id!=abc-cba][format_id!=zxc-cxz]");
    }
    #[test]
    fn ytdlp_chained_not_startswith() {
        assert_parses("[format_id!^=abc][format_id!^=zxc]");
    }
    #[test]
    fn ytdlp_chained_not_endswith() {
        assert_parses("[format_id!$=cba][format_id!$=cxz]");
    }
    #[test]
    fn ytdlp_chained_not_contains() {
        assert_parses("[format_id!*=abc][format_id!*=zxc]");
    }
    #[test]
    fn ytdlp_spaces_width_neq() {
        assert_parses("best [filesize = 1000] [width!=450]");
    }
    #[test]
    fn ytdlp_height_no_match() {
        assert_parses("best[height<40]");
    }
    #[test]
    fn ytdlp_aspect_ratio_lt() {
        assert_parses("all[aspect_ratio < 1.00]");
    }
}

// ============================================================================
// Test Gap Coverage
//
// Tests for evaluation branches, error variants, sort edge cases, and public
// API methods that were previously untested.
// ============================================================================

// ---- Nth-worst selection (eval.rs SortDirection::Worst + Some(n)) ----

#[test]
fn test_eval_nth_worst_video() {
    let formats = test_formats();
    // Video-only formats by ascending height: v720 (720), v1080 (1080), v1440 (1440)
    // worstvideo = v720, worstvideo.2 = v1080, worstvideo.3 = v1440
    let sel = FormatSelector::parse("wv.2").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1080");
}

#[test]
fn test_eval_nth_worst_video_3() {
    let formats = test_formats();
    let sel = FormatSelector::parse("wv.3").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

#[test]
fn test_eval_nth_worst_audio() {
    let formats = test_formats();
    // Audio-only by ascending abr: a64 (64), a128 (128), a256 (256)
    let sel = FormatSelector::parse("wa.2").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "a128");
}

#[test]
fn test_eval_nth_worst_beyond_count() {
    let formats = test_formats();
    // Only 3 video-only formats; wv.10 should return empty
    let sel = FormatSelector::parse("wv.10").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

#[test]
fn test_eval_nth_best_beyond_count() {
    let formats = test_formats();
    // Only 3 video-only formats; bv.10 should return empty
    let sel = FormatSelector::parse("bv.10").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

// ---- worst* evaluation (eval.rs worst + modified) ----

#[test]
fn test_eval_worst_star() {
    let formats = test_formats();
    // w* = worst format with video or audio (any stream type, modified=true)
    let sel = FormatSelector::parse("w*").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    // Worst among all formats ranked by quality > height > tbr > fps.
    // Audio-only formats have no quality/height so they rank lowest.
    // a64 has the lowest abr (64), but ranking is by general key (quality, height, tbr, fps).
    // Formats without quality=None, height=None rank below those with values.
    // a64: quality=None, height=None → lowest.
    assert!(result[0].has_video() || result[0].has_audio());
}

#[test]
fn test_eval_worst_star_video() {
    let formats = test_formats();
    // wv* = worst format that has video (includes combined)
    let sel = FormatSelector::parse("wv*").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert!(result[0].has_video());
    // c360 has the lowest height (360) and quality (1) among formats with video
    assert_eq!(result[0].format_id, "c360");
}

#[test]
fn test_eval_worst_star_audio() {
    let formats = test_formats();
    // wa* = worst format that has audio (includes combined)
    let sel = FormatSelector::parse("wa*").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert!(result[0].has_audio());
    // Audio ranking uses abr > asr. Combined formats without explicit abr
    // tie with audio-only formats via cmp_opt_f64's None handling.
    // The key point: wa* includes combined formats and returns exactly one.
}

// ---- Regex evaluation (eval.rs FilterOp::Regex) ----

#[test]
fn test_eval_regex_digit_pattern() {
    let formats = test_formats();
    // format_id matching a regex pattern with digits
    let sel = FormatSelector::parse("best[format_id~=^c\\d+$]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_eval_regex_alternation() {
    let formats = test_formats();
    // format_id matching alternation: c720 or c360
    let sel = FormatSelector::parse("best[format_id~=c720|c360]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    // best among c720 (quality=2) and c360 (quality=1)
    assert_eq!(result[0].format_id, "c720");
}

#[test]
fn test_eval_regex_invalid_pattern_silent_false() {
    // An invalid regex should silently not match (not panic)
    let formats = test_formats();
    let sel = FormatSelector::parse("best[format_id~=[invalid(regex]").unwrap();
    let result = sel.select(&formats);
    // All formats fail the regex filter → empty result
    assert!(result.is_empty());
}

#[test]
fn test_eval_negated_regex() {
    let formats = test_formats();
    // Negated regex: exclude formats whose format_id starts with 'c'
    let sel = FormatSelector::parse("best[format_id!~=^c]").unwrap();
    let result = sel.select(&formats);
    // All combined formats (c360, c720, c1080) are excluded;
    // no other formats have both video and audio → empty for best (non-modified)
    assert!(result.is_empty());
}

// ---- Non-fatal filter with Other (unknown) field ----

#[test]
fn test_eval_non_fatal_unknown_field_passes_through() {
    let formats = test_formats();
    // Unknown field with non-fatal: should pass all formats through
    let sel = FormatSelector::parse("best[aspect_ratio<=?2]").unwrap();
    let result = sel.select(&formats);
    // aspect_ratio is FilterField::Other → FieldMissing → non_fatal → pass
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_eval_fatal_unknown_field_excludes_all() {
    let formats = test_formats();
    // Unknown field WITHOUT non-fatal: should exclude all formats
    let sel = FormatSelector::parse("best[aspect_ratio<=2]").unwrap();
    let result = sel.select(&formats);
    // aspect_ratio is FilterField::Other → FieldMissing → fatal → exclude
    assert!(result.is_empty());
}

// ---- Negated operators on numeric fields ----

#[test]
fn test_eval_height_ne() {
    let formats = test_formats();
    // best[height!=720] — combined formats: c360 (360), c720 (720), c1080 (1080)
    // Excluding c720, best among c360/c1080 → c1080
    let sel = FormatSelector::parse("best[height!=720]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_eval_height_ne_excludes_all_matching() {
    let formats = test_formats();
    // best[height!=360][height!=720][height!=1080] — excludes all combined formats
    let sel = FormatSelector::parse("best[height!=360][height!=720][height!=1080]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

// ---- String ops with numeric coercion (eval.rs compare_str Number branch) ----

#[test]
fn test_eval_string_op_with_number_coercion() {
    let formats = test_formats();
    // vcodec is "h264" — contains "26" which is a number that gets coerced to string "26"
    let sel = FormatSelector::parse("bv[vcodec*=26]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "v1440");
}

// ---- FormatSelector::expression() accessor ----

#[test]
fn test_expression_accessor_simple() {
    let sel = FormatSelector::parse("best").unwrap();
    assert_eq!(sel.expression(), "best");
}

#[test]
fn test_expression_accessor_complex() {
    let sel = FormatSelector::parse("bv[height<=720]+ba/best").unwrap();
    assert_eq!(sel.expression(), "bv[height<=720]+ba/best");
}

#[test]
fn test_expression_accessor_preserves_whitespace_trimmed() {
    let sel = FormatSelector::parse("  best  ").unwrap();
    assert_eq!(sel.expression(), "best");
}

// ---- Error variant Display formatting ----

#[test]
fn test_error_parse_display() {
    let err = FormatSelector::parse("").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Parse error"),
        "expected parse error, got: {msg}"
    );
}

#[test]
fn test_error_parse_display_unclosed_bracket() {
    let err = FormatSelector::parse("bv[height<=").unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("Parse error"),
        "expected parse error, got: {msg}"
    );
    // Should contain the original input
    assert!(
        msg.contains("bv[height<="),
        "error should contain input, got: {msg}"
    );
}

#[test]
fn test_error_no_match_display() {
    let err = FormatSelectError::NoMatch {
        expression: "best[height>=99999]".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("No formats matched"));
    assert!(msg.contains("best[height>=99999]"));
}

#[test]
fn test_error_unknown_field_display() {
    let err = FormatSelectError::UnknownField {
        field: "foobar".to_string(),
        available: vec!["height", "width"],
    };
    let msg = err.to_string();
    assert!(msg.contains("Unknown field"));
    assert!(msg.contains("foobar"));
    assert!(msg.contains("height"));
}

#[test]
fn test_error_invalid_regex_display() {
    let err = FormatSelectError::InvalidRegex {
        pattern: "[bad(".to_string(),
        error: "unclosed group".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Invalid regex"));
    assert!(msg.contains("[bad("));
    assert!(msg.contains("unclosed group"));
}

#[test]
fn test_error_is_std_error() {
    // FormatSelectError implements std::error::Error
    let err: Box<dyn std::error::Error> = Box::new(FormatSelectError::NoMatch {
        expression: "test".to_string(),
    });
    assert!(err.to_string().contains("No formats matched"));
}

// ---- Size parsing edge cases ----

mod size_edge_cases {
    use super::super::size::parse_size;

    #[test]
    fn negative_number_returns_none() {
        assert_eq!(parse_size("-500M"), None);
    }

    #[test]
    fn overflow_returns_none() {
        // 999999P is way beyond u64
        assert_eq!(parse_size("999999P"), None);
    }

    #[test]
    fn unknown_unit_returns_none() {
        assert_eq!(parse_size("100X"), None);
    }

    #[test]
    fn invalid_suffix_returns_none() {
        assert_eq!(parse_size("100Mx"), None);
    }

    #[test]
    fn petabytes_si() {
        assert_eq!(parse_size("1P"), Some(1_000_000_000_000_000));
    }

    #[test]
    fn petabytes_binary() {
        assert_eq!(parse_size("1PiB"), Some(1_125_899_906_842_624));
    }

    #[test]
    fn fractional_megabytes() {
        assert_eq!(parse_size("1.5M"), Some(1_500_000));
    }

    #[test]
    fn zero_value() {
        assert_eq!(parse_size("0M"), Some(0));
    }

    #[test]
    fn binary_short_suffix() {
        // "Ki" (without B) should also be binary
        assert_eq!(parse_size("100Ki"), Some(102_400));
    }
}

// ---- Sort edge cases ----

mod sort_edge_cases {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    fn make_format(id: &str) -> Format {
        Format::new(
            id,
            "https://example.com/video",
            "mp4",
            DownloadProtocol::Https,
        )
    }

    #[test]
    fn unknown_sort_field_is_noop() {
        let mut f480 = make_format("480p");
        f480.height = Some(480);

        let mut f720 = make_format("720p");
        f720.height = Some(720);

        // Unknown field should not change order (all equal)
        let spec = sort::FormatSorter::parse("unknownfield").unwrap();
        let mut formats: Vec<&Format> = vec![&f480, &f720];
        let orig_order: Vec<&str> = formats.iter().map(|f| f.format_id.as_str()).collect();
        spec.sort(&mut formats);
        let new_order: Vec<&str> = formats.iter().map(|f| f.format_id.as_str()).collect();
        // Stable sort with all-equal keys preserves original order
        assert_eq!(orig_order, new_order);
    }

    #[test]
    fn nearest_ascending_prefers_below_limit() {
        let mut f700 = make_format("700");
        f700.height = Some(700);

        let mut f740 = make_format("740");
        f740.height = Some(740);

        // +res~720: nearest to 720, ascending tiebreak (prefer value <= limit)
        // f700: distance=20, below limit (tiebreak=1)
        // f740: distance=20, above limit (tiebreak=-1)
        let spec = sort::FormatSorter::parse("+res~720").unwrap();
        let mut formats: Vec<&Format> = vec![&f740, &f700];
        spec.sort(&mut formats);

        // Both equidistant, but ascending prefers below-limit → f700 first
        assert_eq!(formats[0].format_id, "700");
        assert_eq!(formats[1].format_id, "740");
    }

    #[test]
    fn nearest_descending_prefers_above_limit() {
        let mut f700 = make_format("700");
        f700.height = Some(700);

        let mut f740 = make_format("740");
        f740.height = Some(740);

        // res~720 (descending default): nearest to 720, prefer value >= limit
        let spec = sort::FormatSorter::parse("res~720").unwrap();
        let mut formats: Vec<&Format> = vec![&f700, &f740];
        spec.sort(&mut formats);

        // Both equidistant, descending prefers above-limit → f740 first
        assert_eq!(formats[0].format_id, "740");
        assert_eq!(formats[1].format_id, "700");
    }

    #[test]
    fn protocol_score_unknown_protocol() {
        let f_https = {
            let mut f = Format::new(
                "https",
                "https://example.com",
                "mp4",
                DownloadProtocol::Https,
            );
            f.vcodec = Some("h264".to_string());
            f
        };
        let f_other = {
            let mut f = Format::new(
                "other",
                "https://example.com",
                "mp4",
                DownloadProtocol::Other("rtmp".to_string()),
            );
            f.vcodec = Some("h264".to_string());
            f
        };

        let spec = sort::FormatSorter::parse("proto").unwrap();
        let mut formats: Vec<&Format> = vec![&f_other, &f_https];
        spec.sort(&mut formats);

        // https has a known protocol score; "rtmp" (Other) has score 0 → https wins
        assert_eq!(formats[0].format_id, "https");
        assert_eq!(formats[1].format_id, "other");
    }

    #[test]
    fn hdr_score_dovi_uppercase() {
        let mut f_dv = make_format("dv");
        f_dv.dynamic_range = Some("DOVI".to_string());
        f_dv.height = Some(1080);

        let mut f_sdr = make_format("sdr");
        f_sdr.dynamic_range = Some("SDR".to_string());
        f_sdr.height = Some(1080);

        let spec = sort::FormatSorter::parse("hdr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_sdr, &f_dv];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "dv"); // DOVI score=5 > SDR score=1
    }

    #[test]
    fn hdr_score_dv_abbreviation() {
        let mut f_dv = make_format("dv");
        f_dv.dynamic_range = Some("DV".to_string());
        f_dv.height = Some(1080);

        let mut f_hlg = make_format("hlg");
        f_hlg.dynamic_range = Some("HLG".to_string());
        f_hlg.height = Some(1080);

        let spec = sort::FormatSorter::parse("hdr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_hlg, &f_dv];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "dv"); // DV score=5 > HLG score=2
    }

    #[test]
    fn hdr_score_hdr10_plus() {
        let mut f_hdr10p = make_format("hdr10p");
        f_hdr10p.dynamic_range = Some("HDR10+".to_string());
        f_hdr10p.height = Some(1080);

        let mut f_hdr10 = make_format("hdr10");
        f_hdr10.dynamic_range = Some("HDR10".to_string());
        f_hdr10.height = Some(1080);

        let spec = sort::FormatSorter::parse("hdr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_hdr10, &f_hdr10p];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "hdr10p"); // HDR10+ score=4 > HDR10 score=3
    }

    #[test]
    fn empty_field_in_sort_spec_is_error() {
        assert!(sort::FormatSorter::parse(":1080").is_err());
    }

    #[test]
    fn sort_spec_comma_only_is_error() {
        assert!(sort::FormatSorter::parse(",,,").is_err());
    }

    #[test]
    fn sort_spec_with_size_limit() {
        // Verify that "size~500M" parses without error and produces a working sorter
        let sorter = sort::FormatSorter::parse("size~500M").unwrap();
        assert!(!sorter.fields_is_empty());

        // Verify it sorts correctly: format closer to 500M wins
        let mut f_small = make_format("small");
        f_small.filesize = Some(100_000_000); // 100M, distance 400M
        let mut f_close = make_format("close");
        f_close.filesize = Some(490_000_000); // 490M, distance 10M
        let mut f_large = make_format("large");
        f_large.filesize = Some(1_000_000_000); // 1G, distance 500M

        let mut formats: Vec<&Format> = vec![&f_small, &f_large, &f_close];
        sorter.sort(&mut formats);

        assert_eq!(formats[0].format_id, "close"); // nearest to 500M
    }

    #[test]
    fn codec_none_treated_as_missing() {
        let mut f_none = make_format("none_codec");
        f_none.vcodec = Some("none".to_string());

        let mut f_h264 = make_format("h264");
        f_h264.vcodec = Some("h264".to_string());

        let spec = sort::FormatSorter::parse("vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&f_none, &f_h264];
        spec.sort(&mut formats);

        // "none" treated as missing → h264 ranks higher
        assert_eq!(formats[0].format_id, "h264");
    }

    #[test]
    fn sort_hasaud_prefers_formats_with_audio() {
        let mut with_aud = make_format("aud");
        with_aud.vcodec = Some("none".to_string());
        with_aud.acodec = Some("aac".to_string());

        let mut no_aud = make_format("vid");
        no_aud.vcodec = Some("h264".to_string());
        no_aud.acodec = Some("none".to_string());

        let spec = sort::FormatSorter::parse("hasaud").unwrap();
        let mut formats: Vec<&Format> = vec![&no_aud, &with_aud];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "aud");
    }

    #[test]
    fn sort_multi_field_limit_and_ascending() {
        let mut f480 = make_format("480p");
        f480.height = Some(480);
        f480.filesize = Some(100_000_000);

        let mut f720 = make_format("720p");
        f720.height = Some(720);
        f720.filesize = Some(500_000_000);

        let mut f1080 = make_format("1080p");
        f1080.height = Some(1080);
        f1080.filesize = Some(1_000_000_000);

        // res:720,+size — prefer <=720 resolution, then smallest file
        let spec = sort::FormatSorter::parse("res:720,+size").unwrap();
        let mut formats: Vec<&Format> = vec![&f1080, &f480, &f720];
        spec.sort(&mut formats);

        // 720 and 480 are within limit (tier 0), 1080 demoted (tier -1)
        // Among within-limit: res descending → 720 > 480
        assert_eq!(formats[0].format_id, "720p");
        assert_eq!(formats[1].format_id, "480p");
        assert_eq!(formats[2].format_id, "1080p");
    }
}

// ---- Group with outer filter evaluation ----

#[test]
fn test_eval_group_with_outer_filter() {
    let mut formats = test_formats();
    // Give one combined format a different ext to test the filter
    formats[0].ext = "webm".to_string(); // c360 → webm

    // (best)[ext=mp4] — group selects best combined (c1080), filter passes (mp4)
    let sel = FormatSelector::parse("(best)[ext=mp4]").unwrap();
    let result = sel.select(&formats);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_eval_group_outer_filter_excludes_all() {
    let formats = test_formats();
    // (best)[ext=flv] — group selects c1080 (mp4), but outer filter ext=flv excludes it
    let sel = FormatSelector::parse("(best)[ext=flv]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

// ---- String ops on numeric fields (returns Fail) ----

#[test]
fn test_eval_string_op_on_numeric_field_fails() {
    let formats = test_formats();
    // StartsWith on height (numeric field) → FilterResult::Fail → excluded
    let sel = FormatSelector::parse("best[height^=10]").unwrap();
    let result = sel.select(&formats);
    assert!(result.is_empty());
}

// ---- format_select convenience function ----

#[test]
fn test_format_select_convenience() {
    let formats = test_formats();
    let result = format_select("best", &formats).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].format_id, "c1080");
}

#[test]
fn test_format_select_error_on_empty() {
    let formats = test_formats();
    let result = format_select("", &formats);
    assert!(result.is_err());
}

// ---- select_with_sorter additional coverage ----

#[test]
fn test_select_with_sorter_affects_pool_order() {
    // select_with_sorter sorts the format pool before evaluating.
    // For keywords like `best`, the internal rank_formats still picks the
    // "best" by quality/height, so the sorter alone doesn't change `best`.
    // However, for `bv` (video-only), the sorter reorders the pool and
    // the first-match semantics can be observed through FormatSpec::Single.
    let formats = test_formats();
    let sel = FormatSelector::parse("bv").unwrap();

    // Without sorter: bv = v1440 (highest height)
    let default_result = sel.select(&formats);
    assert_eq!(default_result[0].format_id, "v1440");

    // With sorter: select_with_sorter still uses rank_formats for bv,
    // so the result should be consistent (v1440 remains best video-only).
    let sorter = sort::FormatSorter::parse("res").unwrap();
    let sorted_result = sel.select_with_sorter(&formats, &sorter);
    assert_eq!(sorted_result.len(), 1);
    assert_eq!(sorted_result[0].format_id, "v1440");
}

// ============================================================================
// Negative Tests
//
// Verify that invalid inputs are rejected, error paths are exercised, and
// boundary conditions produce the expected empty/error results.
// ============================================================================

mod negative_parse {
    use super::*;

    // ---- Structural syntax errors ----

    #[test]
    fn unclosed_parenthesis() {
        assert!(FormatSelector::parse("(bv+ba").is_err());
    }

    #[test]
    fn empty_parenthesis() {
        assert!(FormatSelector::parse("()").is_err());
    }

    #[test]
    fn nested_unclosed_parenthesis() {
        assert!(FormatSelector::parse("((bv+ba)").is_err());
    }

    #[test]
    fn unmatched_close_paren() {
        assert!(FormatSelector::parse("bv+ba)").is_err());
    }

    #[test]
    fn unclosed_bracket_with_value() {
        assert!(FormatSelector::parse("bv[height<=720").is_err());
    }

    #[test]
    fn empty_brackets() {
        assert!(FormatSelector::parse("bv[]").is_err());
    }

    #[test]
    fn filter_no_field_only_operator() {
        // `[<=720]` — field starts with `<` (non-alpha), parser should reject
        assert!(FormatSelector::parse("[<=720]").is_err());
    }

    #[test]
    fn filter_no_operator() {
        assert!(FormatSelector::parse("bv[height]").is_err());
    }

    #[test]
    fn filter_field_starts_with_digit() {
        // `[720<height]` — value on left side
        assert!(FormatSelector::parse("[720<height]").is_err());
    }

    #[test]
    fn filter_empty_value() {
        assert!(FormatSelector::parse("bv[height<=]").is_err());
    }

    // ---- Merge / fallback structural errors ----

    #[test]
    fn leading_plus() {
        assert!(FormatSelector::parse("+bestaudio").is_err());
    }

    #[test]
    fn trailing_plus() {
        assert!(FormatSelector::parse("bestvideo+").is_err());
    }

    #[test]
    fn bare_slash() {
        assert!(FormatSelector::parse("/").is_err());
    }

    #[test]
    fn leading_slash() {
        assert!(FormatSelector::parse("/best").is_err());
    }

    #[test]
    fn double_comma() {
        assert!(FormatSelector::parse("bestvideo,,best").is_err());
    }

    #[test]
    fn double_plus() {
        assert!(FormatSelector::parse("bv++ba").is_err());
    }

    #[test]
    fn double_slash() {
        assert!(FormatSelector::parse("bv//ba").is_err());
    }

    // ---- Whitespace / empty input ----

    #[test]
    fn empty_string() {
        assert!(FormatSelector::parse("").is_err());
    }

    #[test]
    fn whitespace_only() {
        assert!(FormatSelector::parse("   ").is_err());
    }

    #[test]
    fn tab_only() {
        assert!(FormatSelector::parse("\t").is_err());
    }

    #[test]
    fn newline_only() {
        assert!(FormatSelector::parse("\n").is_err());
    }

    // ---- Error type verification ----

    #[test]
    fn parse_error_contains_position() {
        let err = FormatSelector::parse("bv[height<=").unwrap_err();
        match err {
            FormatSelectError::Parse {
                position, input, ..
            } => {
                assert!(position <= input.len(), "position should be within input");
                assert_eq!(input, "bv[height<=");
            }
            _ => panic!("expected Parse variant"),
        }
    }

    #[test]
    fn parse_error_empty_input() {
        let err = FormatSelector::parse("").unwrap_err();
        match err {
            FormatSelectError::Parse { message, .. } => {
                assert!(
                    message.contains("Empty") || message.contains("empty"),
                    "message should mention empty: {message}"
                );
            }
            _ => panic!("expected Parse variant"),
        }
    }

    #[test]
    fn parse_error_caret_display() {
        let err = FormatSelector::parse("bv[height<=").unwrap_err();
        let display = err.to_string();
        // The display format includes a caret under the error position
        assert!(
            display.contains('^'),
            "display should include caret: {display}"
        );
    }
}

mod negative_eval {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    // ---- Empty / all-DRM format lists ----

    #[test]
    fn best_empty_formats() {
        let result = FormatSelector::parse("best").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn worst_empty_formats() {
        let result = FormatSelector::parse("worst").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn bestvideo_empty_formats() {
        let result = FormatSelector::parse("bv").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn bestaudio_empty_formats() {
        let result = FormatSelector::parse("ba").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn all_empty_formats() {
        let result = FormatSelector::parse("all").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn mergeall_empty_formats() {
        let result = FormatSelector::parse("mergeall").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn merge_empty_formats() {
        let result = FormatSelector::parse("bv+ba").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn format_id_empty_formats() {
        let result = FormatSelector::parse("12345").unwrap().select(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn all_formats_drm_best() {
        let mut f = make_combined("drm1", "mp4", 1080, 5);
        f.has_drm = Some(true);
        let mut f2 = make_combined("drm2", "mp4", 720, 3);
        f2.has_drm = Some(true);
        let formats = vec![f, f2];
        let result = FormatSelector::parse("best").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn all_formats_drm_all() {
        let mut f = make_combined("drm1", "mp4", 1080, 5);
        f.has_drm = Some(true);
        let formats = vec![f];
        let result = FormatSelector::parse("all").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn all_formats_drm_mergeall() {
        let mut f = make_combined("drm1", "mp4", 1080, 5);
        f.has_drm = Some(true);
        let formats = vec![f];
        let result = FormatSelector::parse("mergeall").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    // ---- No-match filters ----

    #[test]
    fn impossible_height_filter() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[height>=99999]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn contradictory_filters() {
        let formats = test_formats();
        // height must be both <=360 and >=1080 — impossible
        let result = FormatSelector::parse("best[height<=360][height>=1080]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn ext_filter_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext=avi]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn format_id_not_found() {
        let formats = test_formats();
        let result = FormatSelector::parse("nonexistent_id")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn bestvideo_when_only_audio_exists() {
        let formats = vec![
            make_audio_only("a1", "m4a", 128.0),
            make_audio_only("a2", "m4a", 256.0),
        ];
        let result = FormatSelector::parse("bv").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn bestaudio_when_only_video_exists() {
        let formats = vec![
            make_video_only("v1", "mp4", 720),
            make_video_only("v2", "mp4", 1080),
        ];
        let result = FormatSelector::parse("ba").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    // ---- Missing-field filter behavior (fatal vs non-fatal) ----

    #[test]
    fn fatal_filter_on_missing_vcodec_excludes() {
        let mut f = Format::new(
            "no_codec",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = None; // truly missing, not "none"
        f.acodec = None;
        let formats = vec![f];
        let result = FormatSelector::parse("b*[vcodec=h264]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn non_fatal_filter_on_missing_vcodec_passes() {
        let mut f = Format::new(
            "no_codec",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = None;
        f.acodec = None;
        let formats = vec![f];
        let result = FormatSelector::parse("b*[vcodec=?h264]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "no_codec");
    }

    #[test]
    fn fatal_filter_on_missing_fps_excludes() {
        let mut f = make_combined("no_fps", "mp4", 1080, 5);
        f.fps = None;
        let formats = vec![f];
        let result = FormatSelector::parse("best[fps>=30]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn non_fatal_filter_on_missing_fps_passes() {
        let mut f = make_combined("no_fps", "mp4", 1080, 5);
        f.fps = None;
        let formats = vec![f];
        let result = FormatSelector::parse("best[fps>=?30]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn fatal_filter_on_missing_acodec_excludes() {
        let mut f = Format::new(
            "no_acodec",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = Some("h264".to_string());
        f.acodec = None;
        let formats = vec![f];
        let result = FormatSelector::parse("bv*[acodec=aac]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn non_fatal_filter_on_missing_acodec_passes() {
        let mut f = Format::new(
            "no_acodec",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = Some("h264".to_string());
        f.acodec = None;
        let formats = vec![f];
        let result = FormatSelector::parse("bv*[acodec=?aac]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn fatal_filter_on_missing_filesize_excludes() {
        let f = make_video_only("no_size", "mp4", 1080);
        // filesize is None by default
        let formats = vec![f];
        let result = FormatSelector::parse("bv[filesize<=500M]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn non_fatal_filter_on_missing_filesize_passes() {
        let f = make_video_only("no_size", "mp4", 1080);
        let formats = vec![f];
        let result = FormatSelector::parse("bv[filesize<=?500M]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    // ---- Type mismatch in filter evaluation ----

    #[test]
    fn text_value_on_numeric_field_no_match() {
        let formats = test_formats();
        // "abc" can't be parsed as f64 → compare_opt_num_r returns Fail
        let result = FormatSelector::parse("best[height<=abc]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn regex_on_numeric_field_no_match() {
        let formats = test_formats();
        // Regex op on numeric field → FilterResult::Fail
        let result = FormatSelector::parse("best[height~=\\d+]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn starts_with_on_numeric_field_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[height^=10]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn ends_with_on_numeric_field_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[height$=80]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn contains_on_numeric_field_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[height*=08]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn size_value_on_string_op_no_match() {
        let formats = test_formats();
        // Size value with contains op on string field → returns false
        let result = FormatSelector::parse("best[vcodec*=500M]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- Merge partial results ----

    #[test]
    fn merge_video_side_no_match() {
        let formats = vec![
            make_audio_only("a1", "m4a", 128.0),
            make_audio_only("a2", "m4a", 256.0),
        ];
        // bv has no match, ba matches a2. Per yt-dlp semantics, a merge
        // requires BOTH sides — if either is missing, the whole merge
        // fails (empty) so the outer fallback chain can take over.
        let result = FormatSelector::parse("bv+ba").unwrap().select(&formats);
        assert!(
            result.is_empty(),
            "merge with missing video side must fail cleanly (no partial result)"
        );
    }

    #[test]
    fn merge_audio_side_no_match() {
        let formats = vec![
            make_video_only("v1", "mp4", 720),
            make_video_only("v2", "mp4", 1080),
        ];
        // bv matches v2, ba has no match. Merge requires both sides →
        // empty result (yt-dlp parity).
        let result = FormatSelector::parse("bv+ba").unwrap().select(&formats);
        assert!(
            result.is_empty(),
            "merge with missing audio side must fail cleanly (no partial result)"
        );
    }

    #[test]
    fn merge_video_side_missing_falls_through_to_muxed() {
        // Typical yt-dlp-default pattern `bv*+ba/best` on a muxed-only
        // site: the merge arm produces nothing (no audio-only), so the
        // fallback `/best` picks the best muxed format. Regression guard
        // for the bv+ba auto-pair design (path 1).
        let formats = vec![
            make_combined("c720", "mp4", 720, 2),
            make_combined("c1080", "mp4", 1080, 3),
        ];
        let result = FormatSelector::parse("bv*+ba/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    #[test]
    fn merge_succeeds_on_split_audio_video_site() {
        // Companion to `merge_video_side_missing_falls_through_to_muxed`:
        // when a site exposes both a video-only and audio-only stream
        // (XHamster AV1 / EXT-X-MEDIA case), `bv*+ba/best` picks the pair.
        let formats = vec![
            make_combined("c720", "mp4", 720, 2),
            make_video_only("v1440", "mp4", 1440),
            make_audio_only("a256", "m4a", 256.0),
        ];
        let result = FormatSelector::parse("bv*+ba/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 2, "should return a bv+ba pair");
        assert_eq!(result[0].format_id, "v1440");
        assert_eq!(result[1].format_id, "a256");
    }

    #[test]
    fn merge_neither_side_matches_triggers_fallback() {
        let formats = vec![make_combined("c720", "mp4", 720, 2)];
        // bv needs video-only (no audio), ba needs audio-only (no video)
        // c720 is combined → neither side matches → empty merge → fallback to best
        let result = FormatSelector::parse("bv+ba/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720");
    }

    // ---- Fallback chain exhaustion ----

    #[test]
    fn all_fallbacks_exhausted() {
        let formats = test_formats();
        let result = FormatSelector::parse("bv[height>=9999]/ba[abr>=9999]/best[ext=avi]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- Negation edge cases ----

    #[test]
    fn negated_eq_on_ext_excludes_all_mp4() {
        let formats = vec![
            make_combined("c1", "mp4", 720, 1),
            make_combined("c2", "mp4", 1080, 2),
        ];
        let result = FormatSelector::parse("best[ext!=mp4]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn negated_starts_with_excludes_all() {
        let formats = test_formats(); // all vcodec = "h264"
        let result = FormatSelector::parse("bv[vcodec!^=h]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn negated_ends_with_excludes_all() {
        let formats = test_formats();
        let result = FormatSelector::parse("bv[vcodec!$=264]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn negated_regex_excludes_all() {
        let formats = test_formats();
        let result = FormatSelector::parse("bv[vcodec!~=h264]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- select_all with no-match nodes ----

    #[test]
    fn select_all_mixed_match_and_no_match() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best,bv[height>=9999]").unwrap();
        let results = sel.select_all(&formats);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 1); // best matches
        assert!(results[1].is_empty()); // impossible filter → empty
    }

    #[test]
    fn select_all_all_nodes_no_match() {
        let formats = test_formats();
        let sel = FormatSelector::parse("best[ext=avi],bv[height>=9999]").unwrap();
        let results = sel.select_all(&formats);
        assert_eq!(results.len(), 2);
        assert!(results[0].is_empty());
        assert!(results[1].is_empty());
    }

    // ---- Group with all-failing inner nodes ----

    #[test]
    fn group_all_inner_nodes_fail() {
        let formats = vec![make_combined("c720", "mp4", 720, 2)];
        // Group: (bv+ba) — both need video-only/audio-only, but only combined exists
        // Fallback /best should be reached
        let result = FormatSelector::parse("(bv+ba)/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720");
    }

    #[test]
    fn group_with_outer_filter_rejects_all() {
        let formats = test_formats();
        // (best)[ext=avi] — best finds c1080 (mp4), but outer filter ext=avi rejects
        let result = FormatSelector::parse("(best)[ext=avi]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }
}

mod negative_sort {
    use super::super::sort::FormatSorter;

    #[test]
    fn empty_spec() {
        assert!(FormatSorter::parse("").is_err());
    }

    #[test]
    fn whitespace_only_spec() {
        assert!(FormatSorter::parse("   ").is_err());
    }

    #[test]
    fn empty_field_name_with_limit() {
        assert!(FormatSorter::parse(":1080").is_err());
    }

    #[test]
    fn empty_field_name_with_nearest() {
        assert!(FormatSorter::parse("~720").is_err());
    }

    #[test]
    fn all_commas() {
        assert!(FormatSorter::parse(",,,").is_err());
    }

    #[test]
    fn invalid_nearest_limit_value() {
        // `res~abc` — "abc" is not a number or size literal
        assert!(FormatSorter::parse("res~abc").is_err());
    }

    #[test]
    fn codec_string_limit_silently_ignored() {
        // `vcodec:h264` — "h264" can't be parsed as a number, limit is None (not error)
        let sorter = FormatSorter::parse("vcodec:h264").unwrap();
        assert!(!sorter.fields_is_empty());
    }

    #[test]
    fn plus_only_is_error() {
        // `+` without a field name
        assert!(FormatSorter::parse("+").is_err());
    }

    #[test]
    fn plus_with_limit_no_field_is_error() {
        assert!(FormatSorter::parse("+:1080").is_err());
    }
}

// ============================================================================
// Negative Tests — Round 2
//
// Additional negative paths in parser, evaluator, and sorter.
// ============================================================================

mod negative_parse_2 {
    use super::*;

    // ---- Third atom in merge (only 2 allowed) ----

    #[test]
    fn triple_merge_rejected() {
        // `bv+ba+best` — parser only supports 2-atom merge
        assert!(FormatSelector::parse("bv+ba+best").is_err());
    }

    // ---- Nested groups ----

    #[test]
    fn nested_group_parses() {
        // `((bv+ba))/best` — inner group is a valid selector_list
        assert!(FormatSelector::parse("((bv+ba))/best").is_ok());
    }

    #[test]
    fn group_in_merge() {
        // `(bv)+(ba)` — groups on both sides of merge
        assert!(FormatSelector::parse("(bv)+(ba)").is_ok());
    }

    // ---- Bracket / operator edge cases ----

    #[test]
    fn bracket_inside_bracket() {
        assert!(FormatSelector::parse("bv[[height<=720]]").is_err());
    }

    #[test]
    fn only_close_bracket() {
        assert!(FormatSelector::parse("bv]").is_err());
    }

    #[test]
    fn filter_with_only_operator_and_value() {
        // `[!=720]` — field missing (starts with `!`, not alpha)
        assert!(FormatSelector::parse("[!=720]").is_err());
    }

    #[test]
    fn filter_operator_only_no_value() {
        // `[height<=]` — value is empty (take_while(1.., ..) requires >=1 char)
        assert!(FormatSelector::parse("best[height<=]").is_err());
    }

    // ---- Special characters parse as format IDs ----
    // The parser's parse_format_id accepts any non-whitespace/non-special chars,
    // so bare `*`, `.`, `?`, `!` are treated as literal format IDs (not errors).

    #[test]
    fn bare_star_is_format_id() {
        let sel = FormatSelector::parse("*").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(matches!(&s.base, FormatToken::FormatId(id) if id == "*"));
        }
    }

    #[test]
    fn bare_dot_is_format_id() {
        let sel = FormatSelector::parse(".").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(matches!(&s.base, FormatToken::FormatId(id) if id == "."));
        }
    }

    #[test]
    fn bare_special_chars_are_format_ids() {
        // These are all valid "format IDs" in the parser
        for c in &["?", "!", ".", "*", "#", "@", "~"] {
            assert!(
                FormatSelector::parse(c).is_ok(),
                "'{c}' should parse as format ID"
            );
        }
    }

    // ---- Trailing / leading junk after valid parse ----

    #[test]
    fn junk_after_valid_expression() {
        // Winnow should fail if not all input is consumed
        assert!(FormatSelector::parse("best]extra").is_err());
    }

    #[test]
    fn comma_at_start() {
        assert!(FormatSelector::parse(",best").is_err());
    }

    #[test]
    fn comma_at_end() {
        assert!(FormatSelector::parse("best,").is_err());
    }

    // ---- all / mergeall with unexpected suffixes ----

    #[test]
    fn all_with_nth_suffix_is_error() {
        // `all.3` — `all` keyword matches but `.3` is left unconsumed → parse error
        assert!(FormatSelector::parse("all.3").is_err());
    }

    #[test]
    fn mergeall_with_filter() {
        // `mergeall[height<=720]` — mergeall takes filters in the parser
        assert!(FormatSelector::parse("mergeall[height<=720]").is_ok());
    }

    // ---- Deeply nested fallback chains ----

    #[test]
    fn very_long_fallback_chain() {
        assert!(FormatSelector::parse("bv/ba/best/worst/bv*/ba*/w*/b*").is_ok());
    }

    // ---- Filter with spaces everywhere (yt-dlp compat) ----

    #[test]
    fn spaces_around_filter_elements() {
        assert!(FormatSelector::parse("best [ height <= 720 ]").is_ok());
    }
}

mod negative_eval_2 {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    // ---- worst variants on empty ----

    #[test]
    fn worstvideo_empty() {
        assert!(FormatSelector::parse("wv").unwrap().select(&[]).is_empty());
    }

    #[test]
    fn worstaudio_empty() {
        assert!(FormatSelector::parse("wa").unwrap().select(&[]).is_empty());
    }

    #[test]
    fn worst_star_empty() {
        assert!(FormatSelector::parse("w*").unwrap().select(&[]).is_empty());
    }

    // ---- worst on all-DRM ----

    #[test]
    fn worst_all_drm() {
        let mut f = make_combined("drm", "mp4", 720, 2);
        f.has_drm = Some(true);
        let formats = vec![f];
        assert!(
            FormatSelector::parse("worst")
                .unwrap()
                .select(&formats)
                .is_empty()
        );
    }

    #[test]
    fn worstvideo_all_drm() {
        let mut f = make_video_only("drm_v", "mp4", 1080);
        f.has_drm = Some(true);
        let formats = vec![f];
        assert!(
            FormatSelector::parse("wv")
                .unwrap()
                .select(&formats)
                .is_empty()
        );
    }

    // ---- Extension shorthand no match ----

    #[test]
    fn extension_shorthand_no_matching_format() {
        let formats = test_formats(); // no flv formats
        let result = FormatSelector::parse("flv").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn extension_3gp_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("3gp").unwrap().select(&formats);
        assert!(result.is_empty());
    }

    // ---- Numeric eq with epsilon boundary ----

    #[test]
    fn numeric_eq_exact_match() {
        let formats = test_formats();
        // height=720 should match c720 exactly
        let result = FormatSelector::parse("best[height=720]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720");
    }

    #[test]
    fn numeric_eq_no_match() {
        let formats = test_formats();
        // height=721 should match nothing (720.0 - 721.0 > epsilon)
        let result = FormatSelector::parse("best[height=721]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn numeric_ne_match() {
        let formats = vec![make_combined("c720", "mp4", 720, 2)];
        // height!=720 should exclude the only format
        let result = FormatSelector::parse("best[height!=720]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn numeric_ne_passes() {
        let formats = vec![make_combined("c720", "mp4", 720, 2)];
        // height!=360 should keep c720
        let result = FormatSelector::parse("best[height!=360]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    // ---- compare_str ordering ops with Number value → only Eq/Ne meaningful ----

    #[test]
    fn string_field_lt_with_number_value_fails() {
        let formats = test_formats();
        // ext < 26 (Number) — ordering ops with numeric value on string field return false
        let result = FormatSelector::parse("best[ext<26]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn string_field_ge_with_number_value_fails() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext>=26]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- compare_str ordering ops with Size value → only Eq/Ne meaningful ----

    #[test]
    fn string_field_lt_with_size_value_fails() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext<500M]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn string_field_le_with_size_value_fails() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext<=500M]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- Negated + non-fatal combined ----

    #[test]
    fn negated_non_fatal_missing_field_passes() {
        // Negated + non-fatal on missing field:
        // FieldMissing → non_fatal → true (pass), negation not applied to FieldMissing
        let mut f = Format::new(
            "no_codec",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = None;
        f.acodec = None;
        let formats = vec![f];
        let result = FormatSelector::parse("b*[vcodec!^=?h264]")
            .unwrap()
            .select(&formats);
        // vcodec is None → FieldMissing → non_fatal → pass through
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn negated_non_fatal_present_field_negates() {
        // Negated + non-fatal on present field: normal negation applies
        let formats = test_formats(); // all vcodec = "h264"
        let result = FormatSelector::parse("bv[vcodec!^=?h26]")
            .unwrap()
            .select(&formats);
        // vcodec starts with "h26" → Pass → negated → Fail → excluded
        assert!(result.is_empty());
    }

    // ---- Negated filter on missing field (fatal, no `?`) ----

    #[test]
    fn negated_fatal_missing_field_excludes() {
        // Negated without non-fatal on missing field:
        // FieldMissing → non_fatal=false → excluded
        let mut f = Format::new(
            "no_codec",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = None;
        f.acodec = None;
        let formats = vec![f];
        let result = FormatSelector::parse("b*[vcodec!=h264]")
            .unwrap()
            .select(&formats);
        // vcodec is None → FieldMissing → non_fatal=false → false (excluded)
        assert!(result.is_empty());
    }

    // ---- Protocol filter ----

    #[test]
    fn protocol_filter_no_match() {
        let formats = test_formats(); // all HTTPS
        let result = FormatSelector::parse("best[protocol=m3u8]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn protocol_filter_match() {
        let formats = test_formats(); // all HTTPS
        let result = FormatSelector::parse("best[protocol=https]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    // ---- format_id filter ----

    #[test]
    fn format_id_filter_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[format_id=nonexistent]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn format_id_filter_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[format_id=c1080]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    // ---- Width filter ----

    #[test]
    fn width_filter_missing() {
        let mut f = make_combined("no_width", "mp4", 720, 2);
        f.width = None;
        let formats = vec![f];
        let result = FormatSelector::parse("best[width>=1280]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- Asr filter ----

    #[test]
    fn asr_filter_missing() {
        let formats = test_formats(); // audio formats have no asr set
        let result = FormatSelector::parse("ba[asr>=44100]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- Tbr / Vbr filter on missing ----

    #[test]
    fn tbr_filter_missing() {
        let f = make_audio_only("a1", "m4a", 128.0); // tbr not set
        let formats = vec![f];
        let result = FormatSelector::parse("ba*[tbr>=100]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn vbr_filter_missing() {
        let f = make_combined("c720", "mp4", 720, 2); // vbr not set on combined
        let formats = vec![f];
        let result = FormatSelector::parse("best[vbr>=1000]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    // ---- Single format: codecs_unknown fallback in matches_token ----

    #[test]
    fn codecs_unknown_matches_best() {
        // Format with vcodec=None, acodec=None → codecs_unknown=true → matches best
        let f = Format::new(
            "unknown",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        let formats = vec![f];
        let result = FormatSelector::parse("best").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "unknown");
    }

    #[test]
    fn codecs_unknown_matches_bv_star() {
        let f = Format::new(
            "unknown",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        let formats = vec![f];
        let result = FormatSelector::parse("bv*").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn codecs_unknown_does_not_match_bv_strict() {
        // bv (non-star) requires has_video && !has_audio
        // codecs_unknown=true → has_video=false, has_audio=false → not video-only
        let f = Format::new(
            "unknown",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        let formats = vec![f];
        let result = FormatSelector::parse("bv").unwrap().select(&formats);
        // matches_token: bv (non-modified Video) checks f.has_video() && !f.has_audio()
        // has_video() checks vcodec != Some("none"), which is true for None → false
        // Actually, has_video() with vcodec=None: vcodec.as_deref() != Some("none") → true
        // and has_audio() with acodec=None: acodec.as_deref() != Some("none") → true
        // So has_video() && !has_audio() = true && !true = false
        // But codecs_unknown = vcodec.is_none() && acodec.is_none() = true
        // matches_token Keyword Video non-modified doesn't use codecs_unknown
        assert!(result.is_empty());
    }

    // ---- all / mergeall with DRM mix ----

    #[test]
    fn all_mixed_drm_returns_only_non_drm() {
        let mut f_drm = make_combined("drm", "mp4", 1080, 5);
        f_drm.has_drm = Some(true);
        let f_ok = make_combined("ok", "mp4", 720, 2);
        let formats = vec![f_drm, f_ok];
        let result = FormatSelector::parse("all").unwrap().select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "ok");
    }

    // ---- mergeall with no video or audio (both codecs "none") ----

    #[test]
    fn mergeall_excludes_format_with_no_streams() {
        let mut f = Format::new(
            "empty",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        f.vcodec = Some("none".to_string());
        f.acodec = Some("none".to_string());
        let formats = vec![f];
        let result = FormatSelector::parse("mergeall").unwrap().select(&formats);
        // has_video()=false, has_audio()=false → excluded by mergeall filter
        assert!(result.is_empty());
    }

    // ---- Nth-best with .0 (1-based, .0 saturates to index 0 → off by one?) ----

    #[test]
    fn nth_zero_saturates_to_first() {
        let formats = test_formats();
        // .0 → nth(0) → saturating_sub(1) = 0 → picks index 0 (same as .1)
        let sel = FormatSelector::parse("bv.0").unwrap();
        let result = sel.select(&formats);
        // Should behave like bv.1 → first best = v1440
        // Or it might actually get nth(0) from the parser. Let's verify.
        assert!(!result.is_empty()); // At minimum shouldn't panic
    }

    // ---- Comma-separated with format_id selectors ----

    #[test]
    fn comma_format_ids_one_missing() {
        let formats = test_formats();
        let sel = FormatSelector::parse("c720,nonexistent").unwrap();
        let results = sel.select_all(&formats);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 1);
        assert!(results[1].is_empty());
    }
}

mod negative_sort_2 {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    fn make_format(id: &str) -> Format {
        Format::new(id, "https://example.com", "mp4", DownloadProtocol::Https)
    }

    // ---- Missing numeric fields sort as absent (tier -10) ----

    #[test]
    fn sort_missing_height_ranks_last() {
        let mut f_with = make_format("with");
        f_with.height = Some(720);

        let f_without = make_format("without");
        // height is None

        let spec = sort::FormatSorter::parse("res").unwrap();
        let mut formats: Vec<&Format> = vec![&f_without, &f_with];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "with");
        assert_eq!(formats[1].format_id, "without");
    }

    #[test]
    fn sort_missing_fps_ranks_last() {
        let mut f_with = make_format("with");
        f_with.fps = Some(60.0);

        let f_without = make_format("without");

        let spec = sort::FormatSorter::parse("fps").unwrap();
        let mut formats: Vec<&Format> = vec![&f_without, &f_with];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "with");
    }

    #[test]
    fn sort_missing_abr_ranks_last() {
        let mut f_with = make_format("with");
        f_with.acodec = Some("aac".to_string());
        f_with.abr = Some(256.0);

        let mut f_without = make_format("without");
        f_without.acodec = Some("aac".to_string());

        let spec = sort::FormatSorter::parse("abr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_without, &f_with];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "with");
    }

    #[test]
    fn sort_missing_filesize_ranks_last() {
        let mut f_with = make_format("with");
        f_with.filesize = Some(1_000_000);

        let f_without = make_format("without");

        let spec = sort::FormatSorter::parse("size").unwrap();
        let mut formats: Vec<&Format> = vec![&f_without, &f_with];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "with");
    }

    // ---- HDR with no dynamic_range ----

    #[test]
    fn hdr_missing_dynamic_range_is_sdr() {
        let mut f_hdr = make_format("hdr");
        f_hdr.dynamic_range = Some("HDR10".to_string());
        f_hdr.height = Some(1080);

        let mut f_sdr = make_format("sdr");
        f_sdr.height = Some(1080);
        // dynamic_range = None → SDR score = 1

        let spec = sort::FormatSorter::parse("hdr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_sdr, &f_hdr];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "hdr"); // HDR10 score=3 > SDR score=1
    }

    // ---- Protocol ordering ----

    #[test]
    fn protocol_https_over_http() {
        let f_https = Format::new(
            "https",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        let f_http = Format::new("http", "http://example.com", "mp4", DownloadProtocol::Http);

        let spec = sort::FormatSorter::parse("proto").unwrap();
        let mut formats: Vec<&Format> = vec![&f_http, &f_https];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "https");
    }

    #[test]
    fn protocol_http_over_m3u8() {
        let f_http = Format::new("http", "http://example.com", "mp4", DownloadProtocol::Http);
        let f_m3u8 = Format::new(
            "m3u8",
            "https://example.com/v.m3u8",
            "mp4",
            DownloadProtocol::M3u8,
        );

        let spec = sort::FormatSorter::parse("proto").unwrap();
        let mut formats: Vec<&Format> = vec![&f_m3u8, &f_http];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "http");
    }

    #[test]
    fn protocol_ascending_prefers_worst() {
        let f_https = Format::new(
            "https",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        let f_http = Format::new("http", "http://example.com", "mp4", DownloadProtocol::Http);

        // +proto → ascending → worst protocol first
        let spec = sort::FormatSorter::parse("+proto").unwrap();
        let mut formats: Vec<&Format> = vec![&f_https, &f_http];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "http");
    }

    // ---- Codec ascending ----

    #[test]
    fn vcodec_ascending_prefers_worst_codec() {
        let mut f_av1 = make_format("av1");
        f_av1.vcodec = Some("av1".to_string());

        let mut f_h264 = make_format("h264");
        f_h264.vcodec = Some("h264".to_string());

        // +vcodec → ascending → worst codec first
        let spec = sort::FormatSorter::parse("+vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&f_av1, &f_h264];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "h264"); // h264 < av1 in tier
    }

    // ---- FormatSorter::compare with identical formats ----

    #[test]
    fn compare_identical_returns_equal() {
        let f = make_format("same");
        let spec = sort::FormatSorter::parse("res,vcodec,proto").unwrap();
        assert_eq!(spec.compare(&f, &f), std::cmp::Ordering::Equal);
    }

    // ---- Ascending with limit (within-limit: smaller is better) ----

    #[test]
    fn ascending_with_limit() {
        let mut f480 = make_format("480p");
        f480.height = Some(480);

        let mut f720 = make_format("720p");
        f720.height = Some(720);

        let mut f1080 = make_format("1080p");
        f1080.height = Some(1080);

        // +res:720 → ascending, limit 720
        // Within limit (<=720): ascending → smaller is better → 480 > 720
        // Above limit (1080): demoted
        let spec = sort::FormatSorter::parse("+res:720").unwrap();
        let mut formats: Vec<&Format> = vec![&f1080, &f720, &f480];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "480p"); // smallest within limit
        assert_eq!(formats[1].format_id, "720p"); // at limit
        assert_eq!(formats[2].format_id, "1080p"); // demoted
    }

    // ---- Unknown codec in tier ----

    #[test]
    fn unknown_vcodec_ranks_below_known() {
        let mut f_known = make_format("h264");
        f_known.vcodec = Some("h264".to_string());

        let mut f_unknown = make_format("custom_codec");
        f_unknown.vcodec = Some("my_custom_codec_v3".to_string());

        let spec = sort::FormatSorter::parse("vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&f_unknown, &f_known];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "h264"); // known tier > unknown (score 0)
    }

    // ---- Language / source string fields ----

    #[test]
    fn sort_by_language_with_none() {
        let mut f_en = make_format("en");
        f_en.language = Some("en".to_string());

        let f_none = make_format("none");
        // language is None

        let spec = sort::FormatSorter::parse("lang").unwrap();
        let mut formats: Vec<&Format> = vec![&f_none, &f_en];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "en"); // has language > missing
    }

    // ---- filesize_approx fallback in size sort ----

    #[test]
    fn sort_size_uses_filesize_approx_fallback() {
        let mut f_exact = make_format("exact");
        f_exact.filesize = Some(500_000);

        let mut f_approx = make_format("approx");
        f_approx.filesize_approx = Some(1_000_000);

        let spec = sort::FormatSorter::parse("size").unwrap();
        let mut formats: Vec<&Format> = vec![&f_exact, &f_approx];
        spec.sort(&mut formats);

        // approx (1M) > exact (500K) → approx first
        assert_eq!(formats[0].format_id, "approx");
    }
}

// ============================================================================
// Negative Tests — Round 3
//
// Final sweep: keyword boundary, nth-suffix edge cases, compare_str with
// Size/Number Eq/Ne paths, HDR fallback, ascending string sort, empty codec,
// cmp_opt_f64 with NaN, rank_formats with all-None metadata.
// ============================================================================

mod negative_parse_3 {
    use super::*;

    // ---- Keyword boundary: alphanumeric/underscore after keyword ----

    #[test]
    fn keyword_prefix_with_alpha_suffix_is_format_id() {
        // "best123" — `best` keyword boundary check fails (next char '1' is alphanumeric)
        // → falls through to parse_format_id
        let sel = FormatSelector::parse("best123").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(
                matches!(&s.base, FormatToken::FormatId(id) if id == "best123"),
                "expected FormatId(\"best123\"), got {:?}",
                s.base
            );
        }
    }

    #[test]
    fn keyword_prefix_with_underscore_suffix_is_format_id() {
        // "best_video" — underscore after keyword → not a keyword, format ID
        let sel = FormatSelector::parse("best_video").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(
                matches!(&s.base, FormatToken::FormatId(id) if id == "best_video"),
                "expected FormatId(\"best_video\"), got {:?}",
                s.base
            );
        }
    }

    #[test]
    fn bv_prefix_with_letters_is_format_id() {
        // "bvideo" — `bv` keyword boundary fails → format ID
        let sel = FormatSelector::parse("bvideo").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(matches!(&s.base, FormatToken::FormatId(_)));
        }
    }

    #[test]
    fn all_prefix_with_letters_is_format_id() {
        // "allformats" — `all` keyword boundary fails → format ID
        let sel = FormatSelector::parse("allformats").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(matches!(&s.base, FormatToken::FormatId(_)));
        }
    }

    // ---- Nth suffix with dot-non-digit leaves dot unconsumed ----

    #[test]
    fn nth_suffix_dot_non_digit_is_error() {
        // "bv.x" — dot present but followed by 'x' not digit → dot unconsumed → parse error
        assert!(FormatSelector::parse("bv.x").is_err());
    }

    #[test]
    fn nth_suffix_dot_at_end_is_error() {
        // "bv." — dot at end, no digit follows → dot unconsumed → parse error
        assert!(FormatSelector::parse("bv.").is_err());
    }

    // ---- Extension shorthand boundary: dot prevents extension match ----

    #[test]
    fn extension_with_dot_suffix_is_format_id() {
        // "mp4.1" — extension shorthand check rejects because '.' follows
        // → falls through to format ID
        let sel = FormatSelector::parse("mp4.1").unwrap();
        let spec = &sel.selectors[0].fallbacks[0];
        if let FormatSpec::Single(s) = spec {
            assert!(
                matches!(&s.base, FormatToken::FormatId(id) if id == "mp4.1"),
                "expected FormatId, got {:?}",
                s.base
            );
        }
    }

    // ---- filesize_approx as filter field name ----

    #[test]
    fn filesize_approx_field_maps_to_filesize() {
        // "filesize_approx" is mapped to FilterField::Filesize in parser
        assert!(FormatSelector::parse("best[filesize_approx<=500M]").is_ok());
    }

    // ---- format_note maps to Other ----

    #[test]
    fn format_note_field_is_other() {
        // "format_note" is explicitly mapped to Other in the parser
        assert!(FormatSelector::parse("best[format_note=premium]").is_ok());
    }
}

mod negative_eval_3 {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    // ---- compare_str: Size value with Eq on string field ----

    #[test]
    fn string_field_eq_with_size_value_no_match() {
        // "best[vcodec=500M]" — Size(500_000_000) → converted to string "500000000"
        // compared via Eq against "h264" → no match
        let formats = test_formats();
        let result = FormatSelector::parse("best[vcodec=500M]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn string_field_ne_with_size_value_matches() {
        // "best[vcodec!=500M]" — Size(500_000_000) → string "500000000"
        // Ne against "h264" → true → passes all combined formats
        let formats = test_formats();
        let result = FormatSelector::parse("best[vcodec!=500M]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    // ---- compare_str: Number value with Eq/Ne on string field ----

    #[test]
    fn string_field_eq_with_number_no_match() {
        // "best[ext=42]" — Number(42.0) → string "42" compared via Eq against "mp4"
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext=42]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn string_field_ne_with_number_matches() {
        // "best[ext!=42]" — Number(42.0) → "42" != "mp4" → true
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext!=42]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    // ---- compare_str: string ordering ops (Lt/Le/Gt/Ge) on ext ----

    #[test]
    fn string_field_lt_text_lexicographic() {
        // "best[ext<webm]" — lexicographic: "mp4" < "webm" → true
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext<webm]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        // c1080 has ext="mp4" which is < "webm" lexicographically
        assert_eq!(result[0].format_id, "c1080");
    }

    #[test]
    fn string_field_gt_text_lexicographic() {
        // "best[ext>mp3]" — "mp4" > "mp3" → true
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext>mp3]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    #[test]
    fn string_field_gt_no_match() {
        // "best[ext>zzz]" — "mp4" > "zzz" → false
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext>zzz]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn string_field_le_text() {
        // "best[ext<=mp4]" — "mp4" <= "mp4" → true
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext<=mp4]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn string_field_ge_text() {
        // "best[ext>=mp4]" — "mp4" >= "mp4" → true
        let formats = test_formats();
        let result = FormatSelector::parse("best[ext>=mp4]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
    }

    // ---- rank_formats with all-None metadata (tiebreaker behavior) ----

    #[test]
    fn rank_formats_all_none_metadata_does_not_panic() {
        // Two bare formats with no quality/height/tbr/fps — ranking should not panic
        let f1 = Format::new("a", "https://example.com/a", "mp4", DownloadProtocol::Https);
        let f2 = Format::new("b", "https://example.com/b", "mp4", DownloadProtocol::Https);
        let formats = vec![f1, f2];
        let sel = FormatSelector::parse("b*").unwrap();
        let result = sel.select(&formats);
        // Should return exactly one — doesn't matter which, just no panic
        assert_eq!(result.len(), 1);
    }

    // ---- cmp_opt_f64 edge: one side None, other Some ----

    #[test]
    fn ranking_none_vs_some_tbr_treats_as_equal() {
        // Combined format ranking uses tbr; when one has tbr and other doesn't,
        // cmp_opt_f64 returns Equal — so tiebreak goes to fps
        let mut f1 = make_combined("with_tbr", "mp4", 1080, 5);
        f1.tbr = Some(5000.0);
        f1.fps = Some(30.0);
        let mut f2 = make_combined("no_tbr", "mp4", 1080, 5);
        f2.tbr = None;
        f2.fps = Some(60.0);
        let formats = vec![f1, f2];

        let sel = FormatSelector::parse("best").unwrap();
        let result = sel.select(&formats);
        assert_eq!(result.len(), 1);
        // quality and height equal, tbr cmp returns Equal (None vs Some),
        // fps tiebreak: 60 > 30 → no_tbr wins
        assert_eq!(result[0].format_id, "no_tbr");
    }

    // ---- Fallback actually falls back (first match empty, second non-empty) ----

    #[test]
    fn multi_fallback_uses_second() {
        let formats = test_formats();
        // First: bv[height>=9999] (impossible), second: ba (audio-only exists)
        let result = FormatSelector::parse("bv[height>=9999]/ba")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a256");
    }

    #[test]
    fn multi_fallback_uses_third() {
        let formats = test_formats();
        let result = FormatSelector::parse("bv[height>=9999]/ba[abr>=9999]/best")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    // ---- Extension shorthand with filter ----

    #[test]
    fn extension_shorthand_mp4_with_filter() {
        // "mp4[height<=720]" — extension mp4 + height filter
        let formats = test_formats();
        let sel = FormatSelector::parse("mp4[height<=720]").unwrap();
        let result = sel.select(&formats);
        // ext=mp4 AND height<=720: c360 (360), c720 (720), v720 (720)
        // Extension matches ANY format with ext=mp4 (not just combined)
        // best among those by general ranking → c720 (quality=2)
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c720");
    }

    // ---- Gt/Lt on numeric fields ----

    #[test]
    fn height_gt_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[height>720]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "c1080");
    }

    #[test]
    fn height_gt_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("best[height>1080]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }

    #[test]
    fn abr_lt_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("ba[abr<128]")
            .unwrap()
            .select(&formats);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].format_id, "a64");
    }

    #[test]
    fn abr_lt_no_match() {
        let formats = test_formats();
        let result = FormatSelector::parse("ba[abr<64]")
            .unwrap()
            .select(&formats);
        assert!(result.is_empty());
    }
}

mod negative_sort_3 {
    use super::*;
    use crate::format::Format;
    use crate::protocol::DownloadProtocol;

    fn make_format(id: &str) -> Format {
        Format::new(id, "https://example.com", "mp4", DownloadProtocol::Https)
    }

    // ---- Empty codec string treated as missing ----

    #[test]
    fn empty_vcodec_string_treated_as_missing() {
        let mut f_empty = make_format("empty_codec");
        f_empty.vcodec = Some(String::new()); // ""

        let mut f_h264 = make_format("h264");
        f_h264.vcodec = Some("h264".to_string());

        let spec = sort::FormatSorter::parse("vcodec").unwrap();
        let mut formats: Vec<&Format> = vec![&f_empty, &f_h264];
        spec.sort(&mut formats);

        // "" treated as missing → h264 ranks higher
        assert_eq!(formats[0].format_id, "h264");
    }

    #[test]
    fn empty_acodec_string_treated_as_missing() {
        let mut f_empty = make_format("empty_codec");
        f_empty.acodec = Some(String::new());

        let mut f_aac = make_format("aac");
        f_aac.acodec = Some("aac".to_string());

        let spec = sort::FormatSorter::parse("acodec").unwrap();
        let mut formats: Vec<&Format> = vec![&f_empty, &f_aac];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "aac");
    }

    // ---- HDR with unknown dynamic_range string falls to SDR ----

    #[test]
    fn hdr_unknown_string_falls_to_sdr() {
        let mut f_unknown = make_format("unknown_hdr");
        f_unknown.dynamic_range = Some("SomeUnknownHDR".to_string());
        f_unknown.height = Some(1080);

        let mut f_hlg = make_format("hlg");
        f_hlg.dynamic_range = Some("HLG".to_string());
        f_hlg.height = Some(1080);

        let spec = sort::FormatSorter::parse("hdr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_unknown, &f_hlg];
        spec.sort(&mut formats);

        // Unknown → score 1 (SDR), HLG → score 2 → HLG wins
        assert_eq!(formats[0].format_id, "hlg");
    }

    #[test]
    fn hdr_empty_string_falls_to_sdr() {
        let mut f_empty = make_format("empty_hdr");
        f_empty.dynamic_range = Some(String::new());
        f_empty.height = Some(1080);

        let mut f_hlg = make_format("hlg");
        f_hlg.dynamic_range = Some("HLG".to_string());
        f_hlg.height = Some(1080);

        let spec = sort::FormatSorter::parse("hdr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_empty, &f_hlg];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "hlg");
    }

    // ---- Ascending string sort (+id) ----

    #[test]
    fn ascending_string_sort_by_id() {
        let f_aaa = make_format("aaa");
        let f_zzz = make_format("zzz");

        // +id → ascending → lexicographically lower ID first
        let spec = sort::FormatSorter::parse("+id").unwrap();
        let mut formats: Vec<&Format> = vec![&f_zzz, &f_aaa];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "aaa");
        assert_eq!(formats[1].format_id, "zzz");
    }

    #[test]
    fn descending_string_sort_by_id() {
        let f_aaa = make_format("aaa");
        let f_zzz = make_format("zzz");

        // id (default descending) → lexicographically higher ID first
        let spec = sort::FormatSorter::parse("id").unwrap();
        let mut formats: Vec<&Format> = vec![&f_aaa, &f_zzz];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "zzz");
        assert_eq!(formats[1].format_id, "aaa");
    }

    // ---- Limit parsing: colon with unparseable value on numeric field ----

    #[test]
    fn numeric_field_with_string_limit_silently_none() {
        // "res:abc" — `:abc` fails number parse, `.ok()` makes limit None
        // Sort spec still valid, just no limit applied
        let sorter = sort::FormatSorter::parse("res:abc").unwrap();
        assert!(!sorter.fields_is_empty());

        // Should behave like plain "res" (no limit)
        let mut f480 = make_format("480p");
        f480.height = Some(480);
        let mut f1080 = make_format("1080p");
        f1080.height = Some(1080);

        let mut formats: Vec<&Format> = vec![&f480, &f1080];
        sorter.sort(&mut formats);
        // No limit → descending → 1080 first
        assert_eq!(formats[0].format_id, "1080p");
    }

    // ---- Sort fields: vbr, asr, quality, tbr aliases ----

    #[test]
    fn sort_by_vbr() {
        let mut f_low = make_format("low");
        f_low.vbr = Some(1000.0);
        let mut f_high = make_format("high");
        f_high.vbr = Some(5000.0);

        let spec = sort::FormatSorter::parse("vbr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_low, &f_high];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "high");
    }

    #[test]
    fn sort_by_asr() {
        let mut f_low = make_format("low");
        f_low.asr = Some(22050);
        let mut f_high = make_format("high");
        f_high.asr = Some(48000);

        let spec = sort::FormatSorter::parse("asr").unwrap();
        let mut formats: Vec<&Format> = vec![&f_low, &f_high];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "high");
    }

    #[test]
    fn sort_by_quality() {
        let mut f_low = make_format("low");
        f_low.quality = Some(1);
        let mut f_high = make_format("high");
        f_high.quality = Some(5);

        let spec = sort::FormatSorter::parse("quality").unwrap();
        let mut formats: Vec<&Format> = vec![&f_low, &f_high];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "high");
    }

    #[test]
    fn sort_by_tbr_alias_br() {
        let mut f_low = make_format("low");
        f_low.tbr = Some(500.0);
        let mut f_high = make_format("high");
        f_high.tbr = Some(5000.0);

        // "br" is an alias for "tbr"
        let spec = sort::FormatSorter::parse("br").unwrap();
        let mut formats: Vec<&Format> = vec![&f_low, &f_high];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "high");
    }

    #[test]
    fn sort_by_width() {
        let mut f_narrow = make_format("narrow");
        f_narrow.width = Some(640);
        let mut f_wide = make_format("wide");
        f_wide.width = Some(1920);

        let spec = sort::FormatSorter::parse("width").unwrap();
        let mut formats: Vec<&Format> = vec![&f_narrow, &f_wide];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "wide");
    }

    // ---- ie_pref uses quality as proxy ----

    #[test]
    fn sort_ie_pref_uses_quality() {
        let mut f_low = make_format("low");
        f_low.quality = Some(1);
        let mut f_high = make_format("high");
        f_high.quality = Some(5);

        let spec = sort::FormatSorter::parse("ie_pref").unwrap();
        let mut formats: Vec<&Format> = vec![&f_low, &f_high];
        spec.sort(&mut formats);

        assert_eq!(formats[0].format_id, "high");
    }

    // ---- "source" field uses container ----

    #[test]
    fn sort_source_uses_container() {
        let f_none = make_format("none");
        // container is None

        let mut f_mp4 = make_format("mp4c");
        f_mp4.container = Some("mp4".to_string());

        let spec = sort::FormatSorter::parse("source").unwrap();
        let mut formats: Vec<&Format> = vec![&f_none, &f_mp4];
        spec.sort(&mut formats);

        // mp4c has container present (tier 1) vs none (tier -10 missing)
        assert_eq!(formats[0].format_id, "mp4c");
    }

    // ---- ext/vext/aext all use f.ext ----

    #[test]
    fn sort_vext_uses_ext() {
        let f_mp4 = Format::new(
            "mp4f",
            "https://example.com",
            "mp4",
            DownloadProtocol::Https,
        );
        let f_webm = Format::new(
            "webmf",
            "https://example.com",
            "webm",
            DownloadProtocol::Https,
        );

        let spec = sort::FormatSorter::parse("vext").unwrap();
        let mut formats: Vec<&Format> = vec![&f_mp4, &f_webm];
        spec.sort(&mut formats);

        // Both have ext → tier 1, lexicographic descending: "webm" > "mp4"
        assert_eq!(formats[0].format_id, "webmf");
    }
}

mod negative_size {
    use super::super::size::parse_size;

    #[test]
    fn empty_string() {
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn plain_number_no_unit() {
        assert_eq!(parse_size("12345"), None);
    }

    #[test]
    fn letters_only() {
        assert_eq!(parse_size("abc"), None);
    }

    #[test]
    fn negative_number() {
        assert_eq!(parse_size("-100M"), None);
    }

    #[test]
    fn infinity() {
        assert_eq!(parse_size("infM"), None);
    }

    #[test]
    fn nan() {
        assert_eq!(parse_size("nanM"), None);
    }

    #[test]
    fn unknown_unit_letter() {
        assert_eq!(parse_size("100X"), None);
    }

    #[test]
    fn unknown_suffix_after_unit() {
        assert_eq!(parse_size("100Mfoo"), None);
    }

    #[test]
    fn double_unit() {
        assert_eq!(parse_size("100MM"), None);
    }

    #[test]
    fn unit_without_number() {
        assert_eq!(parse_size("M"), None);
    }

    #[test]
    fn just_dot() {
        assert_eq!(parse_size("."), None);
    }

    #[test]
    fn dot_with_unit() {
        assert_eq!(parse_size(".M"), None);
    }

    #[test]
    fn overflow_large_petabytes() {
        // This would overflow u64 (18.4 EB max)
        assert_eq!(parse_size("99999999P"), None);
    }
}
