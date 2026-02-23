//! Multi-strategy and JSON-LD metadata extraction tests

use super::*;
use scraper::Html;

// ========================================================================
// Multi-Strategy Extraction Tests
// ========================================================================

#[test]
fn test_extract_title_with_json_ld() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "JSON-LD Title"
        }
        </script>
        <input name="title" value="Fallback Title">
        "#,
    );

    // Should prefer JSON-LD over input field
    assert_eq!(base.extract_title(&html), Some("JSON-LD Title".to_string()));
}

#[test]
fn test_extract_title_with_open_graph() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <meta property="og:title" content="Open Graph Title">
        <input name="title" value="Fallback Title">
        "#,
    );

    // Should prefer Open Graph over input field
    assert_eq!(
        base.extract_title(&html),
        Some("Open Graph Title".to_string())
    );
}

#[test]
fn test_extract_title_with_html_title_fallback() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(r#"<title>HTML Title Tag</title>"#);

    // Should fall back to HTML title tag
    assert_eq!(
        base.extract_title(&html),
        Some("HTML Title Tag".to_string())
    );
}

#[test]
fn test_extract_description_with_json_ld() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "description": "JSON-LD Description"
        }
        </script>
        <input name="description" value="Fallback Description">
        "#,
    );

    // Should prefer JSON-LD over input field
    assert_eq!(
        base.extract_description(&html),
        Some("JSON-LD Description".to_string())
    );
}

#[test]
fn test_extract_description_with_open_graph() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <meta property="og:description" content="Open Graph Description">
        <input name="description" value="Fallback Description">
        "#,
    );

    // Should prefer Open Graph over input field
    assert_eq!(
        base.extract_description(&html),
        Some("Open Graph Description".to_string())
    );
}

#[test]
fn test_extract_description_with_meta_tag() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <meta name="description" content="Meta Description">
        <input name="description" value="Fallback Description">
        "#,
    );

    // Should prefer meta description over input field
    assert_eq!(
        base.extract_description(&html),
        Some("Meta Description".to_string())
    );
}

#[test]
fn test_extract_uploader_with_json_ld() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "author": {
                "@type": "Person",
                "name": "JSON-LD Author"
            }
        }
        </script>
        <input name="username" value="fallbackuser">
        "#,
    );

    // Should prefer JSON-LD author over input field
    assert_eq!(
        base.extract_uploader(&html),
        Some("JSON-LD Author".to_string())
    );
}

#[test]
fn test_extract_thumbnail_with_json_ld_single() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "thumbnailUrl": "https://example.com/jsonld-thumb.jpg"
        }
        </script>
        <meta property="og:image" content="https://example.com/og-thumb.jpg">
        "#,
    );

    // Should prefer JSON-LD thumbnail over Open Graph
    assert_eq!(
        base.extract_thumbnail(&html),
        Some("https://example.com/jsonld-thumb.jpg".to_string())
    );
}

#[test]
fn test_extract_thumbnail_with_json_ld_multiple() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "thumbnailUrl": [
                "https://example.com/thumb1.jpg",
                "https://example.com/thumb2.jpg"
            ]
        }
        </script>
        "#,
    );

    // Should use first thumbnail from array
    assert_eq!(
        base.extract_thumbnail(&html),
        Some("https://example.com/thumb1.jpg".to_string())
    );
}

#[test]
fn test_extract_thumbnail_with_twitter_card() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <meta name="twitter:image" content="https://example.com/twitter-thumb.jpg">
        <link rel="image_src" href="https://example.com/link-thumb.jpg">
        "#,
    );

    // Should prefer Twitter card over link rel
    assert_eq!(
        base.extract_thumbnail(&html),
        Some("https://example.com/twitter-thumb.jpg".to_string())
    );
}

#[test]
fn test_extract_thumbnail_with_link_rel() {
    let base = TnaFlixNetworkBase::new();

    let html =
        Html::parse_document(r#"<link rel="image_src" href="https://example.com/link-thumb.jpg">"#);

    // Should use link rel as last fallback
    assert_eq!(
        base.extract_thumbnail(&html),
        Some("https://example.com/link-thumb.jpg".to_string())
    );
}

#[test]
fn test_json_ld_parsing_ignores_non_video_objects() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "WebPage",
            "name": "Should be ignored"
        }
        </script>
        <input name="title" value="Fallback Title">
        "#,
    );

    // Should ignore non-VideoObject JSON-LD and fall back to input field
    assert_eq!(
        base.extract_title(&html),
        Some("Fallback Title".to_string())
    );
}

#[test]
fn test_extract_metadata_filters_empty_strings() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <meta property="og:title" content="">
        <input name="title" value="Non-empty Title">
        "#,
    );

    // Should skip empty Open Graph and fall back to input field
    assert_eq!(
        base.extract_title(&html),
        Some("Non-empty Title".to_string())
    );
}

// ========================================================================
// Phase 3: Enhanced JSON-LD Support Tests
// ========================================================================

#[test]
fn test_parse_iso8601_duration_full() {
    let base = TnaFlixNetworkBase::new();

    // 1 hour, 2 minutes, 3 seconds = 3723 seconds
    assert_eq!(base.parse_iso8601_duration("PT1H2M3S"), Some(3723.0));
}

#[test]
fn test_parse_iso8601_duration_minutes_only() {
    let base = TnaFlixNetworkBase::new();

    // 30 minutes = 1800 seconds
    assert_eq!(base.parse_iso8601_duration("PT30M"), Some(1800.0));
}

#[test]
fn test_parse_iso8601_duration_seconds_only() {
    let base = TnaFlixNetworkBase::new();

    // 45 seconds
    assert_eq!(base.parse_iso8601_duration("PT45S"), Some(45.0));
}

#[test]
fn test_parse_iso8601_duration_hours_only() {
    let base = TnaFlixNetworkBase::new();

    // 2 hours = 7200 seconds
    assert_eq!(base.parse_iso8601_duration("PT2H"), Some(7200.0));
}

#[test]
fn test_parse_iso8601_duration_fractional_seconds() {
    let base = TnaFlixNetworkBase::new();

    // 30.5 seconds
    assert_eq!(base.parse_iso8601_duration("PT30.5S"), Some(30.5));
}

#[test]
fn test_parse_iso8601_duration_invalid() {
    let base = TnaFlixNetworkBase::new();

    // Invalid format
    assert_eq!(base.parse_iso8601_duration("1H2M3S"), None); // Missing PT prefix
    assert_eq!(base.parse_iso8601_duration("PT1X2M"), None); // Invalid character
}

#[test]
fn test_parse_iso8601_date_basic() {
    let base = TnaFlixNetworkBase::new();

    assert_eq!(
        base.parse_iso8601_date("2024-01-15"),
        Some("20240115".to_string())
    );
}

#[test]
fn test_parse_iso8601_date_with_time() {
    let base = TnaFlixNetworkBase::new();

    // Should extract just the date part
    assert_eq!(
        base.parse_iso8601_date("2024-01-15T10:30:00Z"),
        Some("20240115".to_string())
    );
}

#[test]
fn test_parse_iso8601_date_with_timezone() {
    let base = TnaFlixNetworkBase::new();

    // Should extract just the date part
    assert_eq!(
        base.parse_iso8601_date("2024-01-15T10:30:00+00:00"),
        Some("20240115".to_string())
    );
}

#[test]
fn test_parse_iso8601_date_invalid() {
    let base = TnaFlixNetworkBase::new();

    assert_eq!(base.parse_iso8601_date("2024-1-5"), None); // Wrong format
    assert_eq!(base.parse_iso8601_date("24-01-15"), None); // Wrong year length
    assert_eq!(base.parse_iso8601_date("2024/01/15"), None); // Wrong separator
}
