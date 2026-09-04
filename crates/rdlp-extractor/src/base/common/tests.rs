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

// ========================================================================
// fetch_webpage_with_retry
// ========================================================================
//
// The retry-aware fetch helper unifies the hand-rolled
// "GET-with-backon-retry → check status → read body" blocks that four
// site extractors had each re-implemented (pornhub ×2, redtube, xhamster).
// Those copies ADDED retry but DROPPED the URL-length check and the
// `fetch_capped_text` body cap that `fetch_webpage` enforces — reading
// bodies with an uncapped `response.text()`. This helper restores
// safety parity: same URL-length guard and same streaming body cap as
// `fetch_webpage`, PLUS transient-failure retry.
//
// These tests assert the two safety guards fire on the retry path (the
// exact behavior the hand-rolled copies lacked) alongside the happy path
// and status propagation.

#[tokio::test]
async fn fetch_webpage_with_retry_returns_body_on_200() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(200)
        .with_header("content-type", "text/html; charset=utf-8")
        .with_body("<html>ok</html>")
        .create_async()
        .await;

    let ctx = crate::hls::test_support::test_ctx();
    let body = BaseExtractor::fetch_webpage_with_retry(&server.url(), &ctx)
        .await
        .expect("200 response must yield the body");

    assert_eq!(body, "<html>ok</html>");
    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_webpage_with_retry_propagates_5xx_as_error() {
    // A 5xx is a *successful* HTTP exchange returning a bad status — it is
    // NOT a timeout/connect error, so the backon `.when(...)` predicate must
    // not retry it; it should fail fast via `check_http_response`.
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(503)
        .with_body("service unavailable")
        .expect(1) // proves 5xx is not retried by the transient-error predicate
        .create_async()
        .await;

    let ctx = crate::hls::test_support::test_ctx();
    let result = BaseExtractor::fetch_webpage_with_retry(&server.url(), &ctx).await;

    assert!(result.is_err(), "503 must propagate as an error");
    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_webpage_with_retry_rejects_overlong_url() {
    // The URL-length guard `fetch_webpage` has — which the hand-rolled
    // retry copies dropped. An over-cap URL must be rejected as an
    // `Extraction` error BEFORE any network call. The URL points at an
    // unroutable loopback path: if the guard failed to fire, the attempted
    // fetch would surface as a `Network` error instead, so asserting the
    // `Extraction` variant proves the guard short-circuited pre-network.
    let overlong = format!("http://127.0.0.1/{}", "a".repeat(MAX_URL_LENGTH));
    assert!(overlong.len() > MAX_URL_LENGTH);

    let ctx = crate::hls::test_support::test_ctx();
    let err = BaseExtractor::fetch_webpage_with_retry(&overlong, &ctx)
        .await
        .expect_err("an over-cap URL must be rejected");

    assert!(
        matches!(err, RdlpError::Extraction { .. }),
        "expected Extraction (URL too long), got: {err:?}"
    );
}

#[tokio::test]
async fn fetch_webpage_with_retry_enforces_body_cap() {
    // The body cap `fetch_webpage` enforces via `fetch_capped_text` — which
    // the hand-rolled retry copies dropped in favour of an uncapped
    // `response.text()`. A body one byte over `MAX_WEBPAGE_BYTES` must abort
    // mid-stream with a Network error rather than buffer to completion.
    // Intentionally large (~50 MB) — the sole functional guard that the
    // retry path routes through the streaming cap; freed immediately.
    let mut server = mockito::Server::new_async().await;
    let oversized = vec![b'a'; MAX_WEBPAGE_BYTES + 1];
    let mock = server
        .mock("GET", mockito::Matcher::Any)
        .with_status(200)
        .with_body(oversized)
        .create_async()
        .await;

    let ctx = crate::hls::test_support::test_ctx();
    let result = BaseExtractor::fetch_webpage_with_retry(&server.url(), &ctx).await;

    assert!(
        matches!(result, Err(RdlpError::Network { .. })),
        "a body over the cap must be rejected as a Network error, got: {result:?}"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_webpage_with_retry_recovers_after_a_transient_timeout() {
    // Proves the retry machinery itself recovers a *transient* failure — the
    // feature the helper is named for. Deterministic (no wall-clock race): a
    // raw-TCP server accepts the first attempt and never responds, so that
    // attempt can ONLY end via the client-side timeout (a retryable
    // `is_timeout()` error); the server's second `accept()` then blocks until
    // backon actually retries, and serves a 200. mockito can't express this —
    // a returned HTTP status is not a timeout/connect error, so the retry
    // predicate would skip it.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Client timeout well below the (unbounded) stall on attempt 1, and the
    // second attempt responds immediately, so neither bound is timing-fragile.
    const CLIENT_TIMEOUT_MS: u64 = 400;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback listener");
    let addr = listener.local_addr().expect("listener addr");
    let url = format!("http://{addr}/");

    let server = tokio::spawn(async move {
        let mut scratch = [0u8; 1024];

        // Attempt 1: read the request, then hold the connection WITHOUT
        // responding. The client's per-request timeout is the only way out.
        let (mut first, _) = listener.accept().await.expect("accept attempt 1");
        let _ = first.read(&mut scratch).await;

        // Attempt 2 (the retry): blocks here until backon reconnects, so the
        // sequencing is driven by the client, not a sleep.
        let (mut second, _) = listener.accept().await.expect("accept attempt 2");
        let _ = second.read(&mut scratch).await;
        second
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nrecovered!!",
            )
            .await
            .expect("write 200 on retry");
        let _ = second.shutdown().await;
    });

    // A dedicated client with a short per-request timeout so attempt 1's stall
    // surfaces as `is_timeout()` (the retry predicate's trigger).
    let client = wreq::Client::builder()
        .redirect(wreq::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(CLIENT_TIMEOUT_MS))
        .build()
        .expect("client build must succeed in tests");
    let ctx = rdlp_core::ExtractionContext::new(
        std::sync::Arc::new(client),
        std::sync::Arc::new(crate::hls::test_support::NoOpJsEngine),
        std::sync::Arc::new(crate::hls::test_support::NoOpCookieJar),
        std::sync::Arc::new(rdlp_types::Config {
            verbose: false,
            ..rdlp_types::Config::default()
        }),
    );

    let body = BaseExtractor::fetch_webpage_with_retry(&url, &ctx)
        .await
        .expect("a transient first-attempt timeout must be retried, then succeed");

    assert_eq!(body, "recovered!!");
    server.await.expect("server task must not panic");
}

// ========================================================================
// Duration Parsing Tests
// ========================================================================

/// The colon vocabulary (`H:MM:SS` / `M:SS`) that listing-card labels use.
/// `parse_duration` had no coverage at all before PornoXO's grid parser began
/// depending on it; without this, a regression here would surface only as a
/// wrong duration in one site's search results.
#[test]
fn test_parse_duration_colon_formats() {
    assert_eq!(BaseExtractor::parse_duration("1:43:11"), Some(6191.0));
    assert_eq!(BaseExtractor::parse_duration("15:30"), Some(930.0));
    assert_eq!(BaseExtractor::parse_duration("0:07"), Some(7.0));
    // Surrounding whitespace is normal: the label is pretty-printed HTML text.
    assert_eq!(BaseExtractor::parse_duration("\n  15:30  \n"), Some(930.0));
}

#[test]
fn test_parse_duration_rejects_non_durations() {
    assert_eq!(BaseExtractor::parse_duration(""), None);
    assert_eq!(BaseExtractor::parse_duration("soon"), None);
    // Four components is not a time; it must not be read as the leading three.
    assert_eq!(BaseExtractor::parse_duration("1:2:3:4"), None);
    assert_eq!(BaseExtractor::parse_duration("1:xx"), None);
}
