//! Basic renderer tests for `OutputTemplate` — field resolution, fallbacks, defaults

use super::test_helpers::{test_ctx, test_format, test_info};
use super::*;

#[test]
fn test_render_default_template() {
    let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "My Video Title.mp4");
}

#[test]
fn test_render_ext_is_computed() {
    let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mkv", &test_ctx())
        .unwrap();
    assert_eq!(result, "My Video Title.mkv");
}

#[test]
fn test_render_info_dict_fields() {
    let t = OutputTemplate::parse("%(id)s - %(title)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "abc123 - My Video Title");
}

#[test]
fn test_render_format_fields() {
    let t = OutputTemplate::parse("%(format_id)s_%(height)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "720p_720");
}

#[test]
fn test_render_missing_field_with_default() {
    let t = OutputTemplate::parse("%(channel|Unknown Channel)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "Unknown Channel");
}

#[test]
fn test_render_missing_field_defaults_to_na() {
    let t = OutputTemplate::parse("%(nonexistent)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "NA");
}

/// godresource regression: API returns `title: null` and the plugin's
/// `_real_extract` sets `'title': ''` (empty string). `lookup_field`
/// previously returned `Some("")` for empty strings — so the
/// `|Unknown` pipe-default never fired and the rendered filename was
/// `mp4.mkv` (empty title + dot + ext, leading dot stripped). yt-dlp
/// treats empty strings as missing for default-fallback purposes; we
/// must match.
#[test]
fn test_render_empty_title_triggers_pipe_default() {
    let mut info = test_info();
    info.title = String::new();
    let t = OutputTemplate::parse("%(title|Unknown)s.%(ext)s").unwrap();
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "Unknown.mp4");
}

/// Companion to the above: empty title with NO pipe-default falls
/// through to the unconditional "NA" sentinel — also yt-dlp parity.
#[test]
fn test_render_empty_title_no_default_falls_to_na() {
    let mut info = test_info();
    info.title = String::new();
    let t = OutputTemplate::parse("%(title)s.%(ext)s").unwrap();
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "NA.mp4");
}

/// Whitespace-only string is also "missing" — yt-dlp's heuristic is
/// stricter than just `len == 0`, but we match the practical case
/// (titles like "   " from misconfigured extractors).
#[test]
fn test_render_whitespace_title_falls_to_default() {
    let mut info = test_info();
    info.title = "   ".to_string();
    let t = OutputTemplate::parse("%(title|Unknown)s").unwrap();
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "Unknown");
}

#[test]
fn test_render_numeric_padding() {
    let t = OutputTemplate::parse("%(playlist_index)03d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "005");
}

#[test]
fn test_render_subdirectory() {
    let t = OutputTemplate::parse("%(extractor)s/%(title)s.%(ext)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "TestExtractor/My Video Title.mp4");
}

#[test]
fn test_render_complex_template() {
    let t =
        OutputTemplate::parse("%(extractor)s/%(playlist_index)03d - %(title)s [%(id)s].%(ext)s")
            .unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "TestExtractor/005 - My Video Title [abc123].mp4");
}

#[test]
fn test_render_present_optional_over_default() {
    let t = OutputTemplate::parse("%(uploader|Fallback)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "TestUploader");
}

#[test]
fn test_render_view_count_as_string() {
    let t = OutputTemplate::parse("%(view_count)s views").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "12345 views");
}

#[test]
fn test_render_view_count_as_integer() {
    let t = OutputTemplate::parse("%(view_count)d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "12345");
}

#[test]
fn test_render_literal_percent() {
    let t = OutputTemplate::parse("100%% - %(title)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "100% - My Video Title");
}

#[test]
fn test_render_fallback_first_exists() {
    let t = OutputTemplate::parse("%(uploader,channel,title)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "TestUploader");
}

#[test]
fn test_render_fallback_skips_missing() {
    let t = OutputTemplate::parse("%(channel,channel_id,title)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "My Video Title");
}

#[test]
fn test_render_fallback_all_missing_uses_default() {
    let t = OutputTemplate::parse("%(channel,channel_id|Unknown)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "Unknown");
}

#[test]
fn test_render_fallback_all_missing_na() {
    let t = OutputTemplate::parse("%(channel,channel_id)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "NA");
}

#[test]
fn test_render_conditional_replacement_field_exists() {
    let t = OutputTemplate::parse("%(uploader)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "TestUploader");
}

#[test]
fn test_render_conditional_replacement_field_missing() {
    let t = OutputTemplate::parse("%(nonexistent&replacement text)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "");
}

#[test]
fn test_render_date_formatting() {
    let t = OutputTemplate::parse("%(upload_date>%Y-%m-%d)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "2023-11-14");
}

#[test]
fn test_render_date_formatting_year_only() {
    let t = OutputTemplate::parse("%(upload_date>%Y)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "2023");
}

#[test]
fn test_render_arithmetic_add() {
    let t = OutputTemplate::parse("%(playlist_index+10)d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "15");
}

#[test]
fn test_render_arithmetic_div() {
    let t = OutputTemplate::parse("%(duration/60).0f").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    // 3661.5 / 60 = 61.025
    assert_eq!(result, "61");
}

#[test]
fn test_render_arithmetic_sub() {
    let t = OutputTemplate::parse("%(playlist_index-1)d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "4");
}

#[test]
fn test_render_arithmetic_mul() {
    let t = OutputTemplate::parse("%(playlist_index*2)d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "10");
}

#[test]
fn test_render_object_traversal_index() {
    let t = OutputTemplate::parse("%(tags.0)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "tag1");
}

#[test]
fn test_render_object_traversal_negative_index() {
    let t = OutputTemplate::parse("%(tags.-1)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "tag3");
}

#[test]
fn test_render_string_slice_from_start() {
    let t = OutputTemplate::parse("%(title.:8)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "My Video");
}

#[test]
fn test_render_string_slice_from_end() {
    let t = OutputTemplate::parse("%(title.-5:)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "Title");
}

#[test]
fn test_render_string_slice_range() {
    let t = OutputTemplate::parse("%(title.3:8)s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "Video");
}

#[test]
fn test_render_float_format() {
    let t = OutputTemplate::parse("%(duration).2f").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "3661.50");
}

#[test]
fn test_render_float_format_zero_precision() {
    let t = OutputTemplate::parse("%(duration).0f").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "3662");
}
