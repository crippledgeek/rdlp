//! Basic extraction tests for the TNAFlix network base extractor

use super::*;
use scraper::Html;

#[test]
fn test_extract_title() {
    let base = TnaFlixNetworkBase::new();

    // Test input field extraction
    let html1 = Html::parse_document(r#"<input name="title" value="Test Video">"#);
    assert_eq!(base.extract_title(&html1), Some("Test Video".to_string()));

    // Test H1 fallback
    let html2 = Html::parse_document(r#"<h1>Fallback Title</h1>"#);
    assert_eq!(
        base.extract_title(&html2),
        Some("Fallback Title".to_string())
    );

    // Test not found
    let html3 = Html::parse_document(r#"<p>No title here</p>"#);
    assert_eq!(base.extract_title(&html3), None);
}

#[test]
fn test_extract_description() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(r#"<input name="description" value="Test description">"#);
    assert_eq!(
        base.extract_description(&html),
        Some("Test description".to_string())
    );
}

#[test]
fn test_extract_uploader() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(r#"<input name="username" value="testuser">"#);
    assert_eq!(base.extract_uploader(&html), Some("testuser".to_string()));
}

#[test]
fn test_extract_thumbnail() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"<meta property="og:image" content="https://example.com/thumb.jpg">"#,
    );
    assert_eq!(
        base.extract_thumbnail(&html),
        Some("https://example.com/thumb.jpg".to_string())
    );
}

#[test]
fn test_extract_metadata() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <input name="title" value="Test Video">
        <input name="description" value="Test description">
        <input name="username" value="testuser">
        "#,
    );

    let metadata = base.extract_metadata(&html).unwrap();
    assert_eq!(metadata.title, "Test Video");
    assert_eq!(metadata.description, Some("Test description".to_string()));
    assert_eq!(metadata.uploader, Some("testuser".to_string()));
}

#[test]
fn test_extract_config_url_flashvars() {
    let base = TnaFlixNetworkBase::new();

    let html = r#"flashvars.config = escape("http://example.com/config.xml");"#;
    assert_eq!(
        base.extract_config_url(html),
        Some("http://example.com/config.xml".to_string())
    );
}

#[test]
fn test_extract_config_url_input() {
    let base = TnaFlixNetworkBase::new();

    let html = r#"<input name="config1" value="http://example.com/config.xml">"#;
    assert_eq!(
        base.extract_config_url(html),
        Some("http://example.com/config.xml".to_string())
    );
}

#[test]
fn test_extract_config_url_direct() {
    let base = TnaFlixNetworkBase::new();

    let html = r#"config = "http://example.com/config.xml";"#;
    assert_eq!(
        base.extract_config_url(html),
        Some("http://example.com/config.xml".to_string())
    );
}

#[test]
fn test_extract_cdn_url() {
    let base = TnaFlixNetworkBase::new();

    let html = r#"url: 'https://www.moviefap.com/cdn.php?file=abc123',"#;
    assert_eq!(
        base.extract_cdn_url(html),
        Some("https://www.moviefap.com/cdn.php?file=abc123".to_string())
    );
}

#[test]
fn test_parse_moviefap_xml() {
    let base = TnaFlixNetworkBase::new();

    let xml = r#"
        <quality>
            <item>
                <res>720p</res>
                <videoLink>http://example.com/video720.mp4</videoLink>
            </item>
            <item>
                <res>480p</res>
                <videoLink>http://example.com/video480.mp4</videoLink>
            </item>
        </quality>
    "#;

    let video_data = base.parse_moviefap_xml(xml);
    assert_eq!(video_data.len(), 2);

    let (format_id, url, ext, height, width) = &video_data[0];
    assert_eq!(format_id, "http-720");
    assert_eq!(url, "http://example.com/video720.mp4");
    assert_eq!(ext, "mp4");
    assert_eq!(*height, Some(720));
    assert_eq!(*width, Some(1280));
}

#[test]
fn test_parse_video_sources() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <source src="http://example.com/video720.mp4" type="video/mp4" size="720">
        <source src="http://example.com/video480.mp4" type="video/mp4" size="480">
        "#,
    );

    let video_data = base.parse_video_sources(&html);
    assert_eq!(video_data.len(), 2);

    let (format_id, url, ext, height, _) = &video_data[0];
    assert_eq!(format_id, "http-720");
    assert_eq!(url, "http://example.com/video720.mp4");
    assert_eq!(ext, "mp4");
    assert_eq!(*height, Some(720));
}

#[test]
fn test_parse_moviefap_xml_with_html_entities() {
    let base = TnaFlixNetworkBase::new();

    let xml = r#"
        <item>
            <res>720p</res>
            <videoLink>http://example.com/video.mp4?key=abc&amp;token=xyz</videoLink>
        </item>
    "#;

    let video_data = base.parse_moviefap_xml(xml);
    assert_eq!(video_data.len(), 1);

    let (_, url, _, _, _) = &video_data[0];
    assert_eq!(url, "http://example.com/video.mp4?key=abc&token=xyz");
}

/// A bare `&` earlier in the query must not stop a later `&amp;` decoding.
///
/// This is the case where swapping the hand-rolled `.replace("&amp;", "&")`
/// for the library was briefly a REGRESSION: on html-escape 0.2.13 the bare
/// `&` in `?a=1&b=2` swallowed the `&amp;` after it and the URL came back
/// unchanged, where the replace had always decoded it. Fixed by pinning
/// 0.2.15; this test fails on 0.2.13.
#[test]
fn test_parse_moviefap_xml_decodes_amp_after_a_bare_ampersand() {
    let base = TnaFlixNetworkBase::new();

    let xml = r#"
        <item>
            <res>720p</res>
            <videoLink>http://example.com/v.mp4?a=1&b=2&amp;c=3</videoLink>
        </item>
    "#;

    let video_data = base.parse_moviefap_xml(xml);
    assert_eq!(video_data.len(), 1);

    let (_, url, _, _, _) = &video_data[0];
    assert_eq!(url, "http://example.com/v.mp4?a=1&b=2&c=3");
}

/// The XML is read by regex, so the parser never decoded it — and a site is
/// free to write `&` as a NUMERIC reference rather than `&amp;`. The former
/// hand-rolled `.replace("&amp;", "&")` handled exactly one spelling and left
/// `&#38;` in the URL, which then goes to the network verbatim.
///
/// Named references that are also plausible query keys are checked here too:
/// `html_escape` requires the semicolon, so `&copy=` and `&times=` survive —
/// measured, because decoding those would corrupt a working URL.
#[test]
fn test_parse_moviefap_xml_decodes_numeric_and_spares_query_keys() {
    let base = TnaFlixNetworkBase::new();

    let xml = r#"
        <item>
            <res>720p</res>
            <videoLink>http://example.com/v.mp4?a=1&#38;copy=2&times=3</videoLink>
        </item>
    "#;

    let video_data = base.parse_moviefap_xml(xml);
    assert_eq!(video_data.len(), 1);

    let (_, url, _, _, _) = &video_data[0];
    assert_eq!(url, "http://example.com/v.mp4?a=1&copy=2&times=3");
}
