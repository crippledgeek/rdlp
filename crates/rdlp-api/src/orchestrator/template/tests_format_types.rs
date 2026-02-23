//! Format-type-specific renderer tests — JSON, list, bytes, sanitize, quote, etc.

use super::test_helpers::{test_ctx, test_format, test_info};
use super::*;

#[test]
fn test_render_json_format() {
    let t = OutputTemplate::parse("%(title)j").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "_My Video Title_");
}

#[test]
fn test_render_json_pretty() {
    let t = OutputTemplate::parse("%(tags)#j").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert!(result.contains('\n'));
    assert!(result.contains("tag1"));
}

#[test]
fn test_render_list_format() {
    let t = OutputTemplate::parse("%(tags)l").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "tag1, tag2, tag3");
}

#[test]
fn test_render_list_newline() {
    let t = OutputTemplate::parse("%(tags)#l").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "tag1\ntag2\ntag3");
}

#[test]
fn test_render_bytes_format() {
    let t = OutputTemplate::parse("%(filesize)B").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "590.00MB");
}

#[test]
fn test_render_bytes_binary() {
    let t = OutputTemplate::parse("%(filesize)#B").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "562.67MiB");
}

#[test]
fn test_render_decimal_format() {
    let t = OutputTemplate::parse("%(view_count)D").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "12.3K");
}

#[test]
fn test_render_sanitize_format() {
    let t = OutputTemplate::parse("%(title)S").unwrap();
    let info = {
        let mut i = test_info();
        i.title = "My: Video / Title".to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "My_ Video _ Title");
}

#[test]
fn test_render_sanitize_ascii_only() {
    let t = OutputTemplate::parse("%(title)#S").unwrap();
    let info = {
        let mut i = test_info();
        i.title = "日本語/Test".to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "____Test");
}

#[test]
fn test_render_quote_format() {
    let t = OutputTemplate::parse("%(title)q").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "'My Video Title'");
}

#[test]
fn test_render_quote_with_single_quote() {
    let t = OutputTemplate::parse("%(title)q").unwrap();
    let info = {
        let mut i = test_info();
        i.title = "It's a test".to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "'It'_''s a test'");
}

#[test]
fn test_render_html_format() {
    let t = OutputTemplate::parse("%(title)h").unwrap();
    let info = {
        let mut i = test_info();
        i.title = "<b>Bold & \"Fun\"</b>".to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "&lt;b&gt;Bold &amp; &quot;Fun&quot;&lt;_b&gt;");
}

#[test]
fn test_render_char_format() {
    let t = OutputTemplate::parse("%(view_count)c").unwrap();
    let info = {
        let mut i = test_info();
        i.view_count = Some(65); // ASCII 'A'
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "A");
}

#[test]
fn test_render_thousands_separator() {
    let t = OutputTemplate::parse("%(view_count)R").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "12,345");
}

#[test]
fn test_render_thousands_large_number() {
    let t = OutputTemplate::parse("%(filesize)R").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "590,000,000");
}

#[test]
fn test_render_epoch() {
    let t = OutputTemplate::parse("%(epoch)d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "1700000000");
}

#[test]
fn test_render_autonumber() {
    let t = OutputTemplate::parse("%(autonumber)05d").unwrap();
    let ctx = RenderContext {
        epoch: 0,
        autonumber: 42,
    };
    let result = t.render(&test_info(), &test_format(), "mp4", &ctx).unwrap();
    assert_eq!(result, "00042");
}

#[test]
fn test_render_width_string() {
    let t = OutputTemplate::parse("%(id)10s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "    abc123");
}

#[test]
fn test_render_precision_string() {
    let t = OutputTemplate::parse("%(title).5s").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "My Vi");
}

#[test]
fn test_render_integer_width_no_pad() {
    let t = OutputTemplate::parse("%(playlist_index)5d").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "    5");
}

#[test]
fn test_render_unicode_passthrough() {
    let t = OutputTemplate::parse("%(title)U").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "My Video Title");
}

#[test]
fn test_render_bytes_small() {
    let t = OutputTemplate::parse("%(view_count)B").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "12.35KB");
}

#[test]
fn test_render_decimal_millions() {
    let t = OutputTemplate::parse("%(filesize)D").unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "590M");
}

#[test]
fn test_backward_compat_complex() {
    let t =
        OutputTemplate::parse("%(extractor)s/%(playlist_index)03d - %(title)s [%(id)s].%(ext)s")
            .unwrap();
    let result = t
        .render(&test_info(), &test_format(), "mp4", &test_ctx())
        .unwrap();
    assert_eq!(result, "TestExtractor/005 - My Video Title [abc123].mp4");
}

#[test]
fn test_render_field_sanitizes_windows_chars() {
    let t = OutputTemplate::parse("%(title)s").unwrap();
    let info = {
        let mut i = test_info();
        i.title = r#"a/b\c:d*e?f"g<h>i|j"#.to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "a_b_c_d_e_f_g_h_i_j");
}

#[test]
fn test_render_literal_slash_preserved_as_directory_separator() {
    let t = OutputTemplate::parse("%(extractor)s/%(title)s.%(ext)s").unwrap();
    let info = {
        let mut i = test_info();
        i.title = "safe-title".to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "TestExtractor/safe-title.mp4");
}

#[test]
fn test_render_field_strips_null_and_control_chars() {
    let t = OutputTemplate::parse("%(title)s").unwrap();
    let info = {
        let mut i = test_info();
        i.title = "hello\0world\x01\x1f!".to_string();
        i
    };
    let result = t.render(&info, &test_format(), "mp4", &test_ctx()).unwrap();
    assert_eq!(result, "helloworld!");
}
