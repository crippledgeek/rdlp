//! Tests for common base extractor utilities

use super::*;
use rdlp_security::is_private_host;
use rdlp_types::Codec;
use regex::Regex;
use scraper::Html;

// ========================================================================
// URL ID Extraction Tests
// ========================================================================

#[test]
fn test_extract_id_from_url_named() {
    let pattern = Regex::new(r"example\.com/video/(?P<id>\d+)").unwrap();

    assert_eq!(
        BaseExtractor::extract_id_from_url("https://example.com/video/12345", &pattern, "id"),
        Some("12345".to_string())
    );

    assert_eq!(
        BaseExtractor::extract_id_from_url("https://other.com/video/12345", &pattern, "id"),
        None
    );
}

#[test]
fn test_extract_id_positional() {
    let pattern = Regex::new(r"example\.com/(?:video/(\d+)|v/(\d+))").unwrap();

    assert_eq!(
        BaseExtractor::extract_id_positional("https://example.com/video/123", &pattern, &[1, 2]),
        Some("123".to_string())
    );

    assert_eq!(
        BaseExtractor::extract_id_positional("https://example.com/v/456", &pattern, &[1, 2]),
        Some("456".to_string())
    );
}

// ========================================================================
// Quality Parsing Tests
// ========================================================================

#[test]
fn test_parse_quality_height() {
    assert_eq!(BaseExtractor::parse_quality_height("720p"), Some(720));
    assert_eq!(BaseExtractor::parse_quality_height("1080P"), Some(1080));
    assert_eq!(BaseExtractor::parse_quality_height("720"), Some(720));
    assert_eq!(BaseExtractor::parse_quality_height("invalid"), None);
}

#[test]
fn test_parse_quality_from_url() {
    assert_eq!(
        BaseExtractor::parse_quality_from_url("https://example.com/video_720p.mp4"),
        Some(720)
    );
    assert_eq!(
        BaseExtractor::parse_quality_from_url("https://example.com/1080P_video.mp4"),
        Some(1080)
    );
    assert_eq!(
        BaseExtractor::parse_quality_from_url("https://example.com/video.mp4"),
        None
    );
}

#[test]
fn test_width_from_height() {
    // Standard resolutions (lookup table)
    assert_eq!(BaseExtractor::width_from_height(240), 426);
    assert_eq!(BaseExtractor::width_from_height(360), 640);
    assert_eq!(BaseExtractor::width_from_height(480), 854);
    assert_eq!(BaseExtractor::width_from_height(720), 1280);
    assert_eq!(BaseExtractor::width_from_height(1080), 1920);
    assert_eq!(BaseExtractor::width_from_height(1440), 2560);
    assert_eq!(BaseExtractor::width_from_height(2160), 3840);
    // Non-standard height falls back to integer math
    assert_eq!(BaseExtractor::width_from_height(900), (900 * 16) / 9);
}

// ========================================================================
// URL Security Tests
// ========================================================================

#[test]
fn test_validate_url_security_valid() {
    assert!(BaseExtractor::validate_url_security("https://example.com/video.mp4").is_ok());
    assert!(BaseExtractor::validate_url_security("http://cdn.example.com/file").is_ok());
}

#[test]
fn test_validate_url_security_invalid_scheme() {
    assert!(BaseExtractor::validate_url_security("ftp://example.com/file").is_err());
    assert!(BaseExtractor::validate_url_security("file:///etc/passwd").is_err());
}

#[test]
fn test_validate_url_security_private_ip() {
    assert!(BaseExtractor::validate_url_security("http://localhost/file").is_err());
    assert!(BaseExtractor::validate_url_security("http://127.0.0.1/file").is_err());
    assert!(BaseExtractor::validate_url_security("http://192.168.1.1/file").is_err());
    assert!(BaseExtractor::validate_url_security("http://10.0.0.1/file").is_err());
}

#[test]
fn test_is_private_host() {
    // Private hosts
    assert!(is_private_host("localhost"));
    assert!(is_private_host("127.0.0.1"));
    assert!(is_private_host("192.168.1.1"));
    assert!(is_private_host("10.0.0.1"));
    assert!(is_private_host("172.16.0.1"));
    assert!(is_private_host("::1"));
    assert!(is_private_host("myhost.local"));

    // Public hosts
    assert!(!is_private_host("example.com"));
    assert!(!is_private_host("8.8.8.8"));
    assert!(!is_private_host("cdn.example.com"));
}

// ========================================================================
// Metadata Extraction Tests
// ========================================================================

#[test]
fn test_extract_meta_content() {
    let html = Html::parse_document(r#"<meta property="og:title" content="Test Title">"#);
    assert_eq!(
        BaseExtractor::extract_meta_content(&html, &OG_TITLE_SELECTOR),
        Some("Test Title".to_string())
    );
}

#[test]
fn test_extract_meta_content_empty() {
    let html = Html::parse_document(r#"<meta property="og:title" content="">"#);
    assert_eq!(
        BaseExtractor::extract_meta_content(&html, &OG_TITLE_SELECTOR),
        None
    );
}

#[test]
fn test_extract_element_text() {
    let html = Html::parse_document("<h1>  Test Heading  </h1>");
    assert_eq!(
        BaseExtractor::extract_element_text(&html, &H1_SELECTOR),
        Some("Test Heading".to_string())
    );
}

#[test]
fn test_extract_title_multi_strategy() {
    // Test OG title (highest priority)
    let html1 = Html::parse_document(
        r#"
        <meta property="og:title" content="OG Title">
        <title>HTML Title</title>
        "#,
    );
    assert_eq!(
        BaseExtractor::extract_title_multi_strategy(&html1),
        Some("OG Title".to_string())
    );

    // Test fallback to HTML title
    let html2 = Html::parse_document("<title>HTML Title Only</title>");
    assert_eq!(
        BaseExtractor::extract_title_multi_strategy(&html2),
        Some("HTML Title Only".to_string())
    );

    // Test fallback to H1
    let html3 = Html::parse_document("<h1>H1 Title</h1>");
    assert_eq!(
        BaseExtractor::extract_title_multi_strategy(&html3),
        Some("H1 Title".to_string())
    );
}

// ========================================================================
// Date/Time Parsing Tests
// ========================================================================

#[test]
fn test_parse_iso8601_duration() {
    assert_eq!(
        BaseExtractor::parse_iso8601_duration("PT1H2M3S"),
        Some(3723.0)
    );
    assert_eq!(BaseExtractor::parse_iso8601_duration("PT30M"), Some(1800.0));
    assert_eq!(BaseExtractor::parse_iso8601_duration("PT45S"), Some(45.0));
    assert_eq!(BaseExtractor::parse_iso8601_duration("PT2H"), Some(7200.0));
    assert_eq!(BaseExtractor::parse_iso8601_duration("PT30.5S"), Some(30.5));
    // Day-prefixed form (e.g. KoreanPornMovie's `P0DT1H1M14S`).
    assert_eq!(
        BaseExtractor::parse_iso8601_duration("P0DT1H1M14S"),
        Some(3674.0)
    );
    assert_eq!(
        BaseExtractor::parse_iso8601_duration("P0DT0H1M57S"),
        Some(117.0)
    );
    // Reject malformed shapes.
    assert_eq!(BaseExtractor::parse_iso8601_duration("1H2M3S"), None); // Missing PT
    assert_eq!(BaseExtractor::parse_iso8601_duration("invalid"), None);
}

#[test]
fn test_parse_iso8601_date() {
    assert_eq!(
        BaseExtractor::parse_iso8601_date("2024-01-15"),
        Some("20240115".to_string())
    );
    assert_eq!(
        BaseExtractor::parse_iso8601_date("2024-01-15T10:30:00Z"),
        Some("20240115".to_string())
    );
    assert_eq!(BaseExtractor::parse_iso8601_date("invalid"), None);
}

#[test]
fn test_parse_human_count() {
    // Bare digits + thousands separators
    assert_eq!(BaseExtractor::parse_human_count("0"), Some(0));
    assert_eq!(BaseExtractor::parse_human_count("42"), Some(42));
    assert_eq!(BaseExtractor::parse_human_count("1234"), Some(1234));
    assert_eq!(BaseExtractor::parse_human_count("1,234"), Some(1234));
    assert_eq!(
        BaseExtractor::parse_human_count("1,234,567"),
        Some(1_234_567)
    );

    // SI suffixes (case-insensitive)
    assert_eq!(BaseExtractor::parse_human_count("5K"), Some(5_000));
    assert_eq!(BaseExtractor::parse_human_count("5k"), Some(5_000));
    assert_eq!(BaseExtractor::parse_human_count("1.5M"), Some(1_500_000));
    assert_eq!(
        BaseExtractor::parse_human_count("1.2B"),
        Some(1_200_000_000)
    );
    assert_eq!(BaseExtractor::parse_human_count("940K"), Some(940_000));

    // Trailing labels and extra whitespace
    assert_eq!(BaseExtractor::parse_human_count("1,234 views"), Some(1234));
    assert_eq!(BaseExtractor::parse_human_count("  5.6K  "), Some(5_600));

    // Unparseable / empty
    assert_eq!(BaseExtractor::parse_human_count(""), None);
    assert_eq!(BaseExtractor::parse_human_count("   "), None);
    assert_eq!(BaseExtractor::parse_human_count("nope"), None);
    assert_eq!(BaseExtractor::parse_human_count("N/A"), None);
    assert_eq!(BaseExtractor::parse_human_count("abc.defK"), None);
    assert_eq!(BaseExtractor::parse_human_count("xyzM"), None);
    // Stripping only the trailing suffix once → "5k5k" → "5k5" → fails
    assert_eq!(BaseExtractor::parse_human_count("5K5K"), None);
}

// ========================================================================
// String Utility Tests
// ========================================================================

#[test]
fn test_truncate_string() {
    assert_eq!(
        BaseExtractor::truncate_string("Hello World".to_string(), 5),
        "Hello"
    );
    assert_eq!(BaseExtractor::truncate_string("Hi".to_string(), 10), "Hi");
    // Unicode handling
    assert_eq!(
        BaseExtractor::truncate_string("Hello 世界".to_string(), 7),
        "Hello 世"
    );
}

#[test]
fn test_sanitize_for_logging() {
    assert_eq!(
        BaseExtractor::sanitize_for_logging("url?token=secret123&other=value"),
        "url?token=***&other=value"
    );
    assert_eq!(
        BaseExtractor::sanitize_for_logging("url?key=abc&password=xyz"),
        "url?key=***&password=***"
    );
}

// ========================================================================
// Format Building Tests
// ========================================================================

#[test]
fn test_build_format() {
    let format = BaseExtractor::build_format(
        "http-720".to_string(),
        "https://example.com/video.mp4".to_string(),
        "mp4".to_string(),
        Some(720),
    );

    assert_eq!(format.format_id, "http-720");
    assert_eq!(format.height, Some(720));
    assert_eq!(format.width, Some(1280));
    assert_eq!(format.format_note, Some("720p".to_string()));
    assert_eq!(format.vcodec, Codec::Present("h264".to_string()));
    assert_eq!(format.acodec, Codec::Present("aac".to_string()));
}

#[test]
fn test_build_format_webm() {
    let format = BaseExtractor::build_format(
        "webm-1080".to_string(),
        "https://example.com/video.webm".to_string(),
        "webm".to_string(),
        Some(1080),
    );

    assert_eq!(format.vcodec, Codec::Present("vp9".to_string()));
    assert_eq!(format.acodec, Codec::Present("opus".to_string()));
}

// ========================================================================
// Body-cap regression guards
// ========================================================================
//
// These tests lock the value of `MAX_WEBPAGE_BYTES` in two directions: it
// must be (a) generous enough to fit any realistic HTML+JSON-LD
// extractor target, and (b) tight enough to prevent OOM on a 32-GB host
// even with several concurrent extractors running. Bumping the cap in
// either direction MUST be a deliberate change with this test updated.

#[test]
fn webpage_cap_is_in_safe_range() {
    let cap = super::MAX_WEBPAGE_BYTES;
    // Realistic extractor pages are <5MB even with embedded JSON-LD.
    // Below 1MB would clip live sites; above 200MB risks OOM under
    // concurrency. Keep the cap inside a justifiable middle band.
    assert!(
        (1024 * 1024..=200 * 1024 * 1024).contains(&cap),
        "MAX_WEBPAGE_BYTES = {cap} is outside the safe 1..=200 MB band"
    );
}

#[test]
fn webpage_cap_documented_value() {
    // Pin the documented cap. Bumping is fine but should be intentional.
    assert_eq!(super::MAX_WEBPAGE_BYTES, 50 * 1024 * 1024);
}
