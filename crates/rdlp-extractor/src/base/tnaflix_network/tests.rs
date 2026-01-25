//! Tests for the TNAFlix network base extractor

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

#[test]
fn test_extract_view_count_single() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "interactionStatistic": {
                "@type": "InteractionCounter",
                "interactionType": "WatchAction",
                "userInteractionCount": 12345
            }
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    assert_eq!(extract_view_count(&json_ld), Some(12345));
}

#[test]
fn test_extract_view_count_multiple() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "interactionStatistic": [
                {
                    "@type": "InteractionCounter",
                    "interactionType": "LikeAction",
                    "userInteractionCount": 500
                },
                {
                    "@type": "InteractionCounter",
                    "interactionType": "WatchAction",
                    "userInteractionCount": 98765
                }
            ]
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    // Should extract WatchAction count, not LikeAction
    assert_eq!(extract_view_count(&json_ld), Some(98765));
}

#[test]
fn test_extract_tags_from_string() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "keywords": "tag1, tag2, tag3"
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    let tags = extract_tags(&json_ld).unwrap();
    assert_eq!(tags, vec!["tag1", "tag2", "tag3"]);
}

#[test]
fn test_extract_tags_from_array() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "keywords": ["tag1", "tag2", "tag3"]
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    let tags = extract_tags(&json_ld).unwrap();
    assert_eq!(tags, vec!["tag1", "tag2", "tag3"]);
}

#[test]
fn test_extract_categories_from_string() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "genre": "Action"
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    let categories = extract_categories(&json_ld).unwrap();
    assert_eq!(categories, vec!["Action"]);
}

#[test]
fn test_extract_categories_from_array() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "genre": ["Action", "Adventure", "Comedy"]
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    let categories = extract_categories(&json_ld).unwrap();
    assert_eq!(categories, vec!["Action", "Adventure", "Comedy"]);
}

#[test]
fn test_extract_thumbnails_creates_thumbnail_list() {
    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Test",
            "thumbnailUrl": [
                "https://example.com/thumb1.jpg",
                "https://example.com/thumb2.jpg",
                "https://example.com/thumb3.jpg"
            ]
        }
        </script>
        "#,
    );

    let json_ld = extract_json_ld(&html).unwrap();
    let thumbnails = extract_thumbnails(&json_ld).unwrap();

    assert_eq!(thumbnails.len(), 3);
    assert_eq!(thumbnails[0].url, "https://example.com/thumb1.jpg");
    assert_eq!(thumbnails[0].id, Some("jsonld_0".to_string()));
    assert_eq!(thumbnails[0].preference, Some(0)); // First is most preferred

    assert_eq!(thumbnails[1].url, "https://example.com/thumb2.jpg");
    assert_eq!(thumbnails[1].id, Some("jsonld_1".to_string()));
    assert_eq!(thumbnails[1].preference, Some(-1));

    assert_eq!(thumbnails[2].url, "https://example.com/thumb3.jpg");
    assert_eq!(thumbnails[2].id, Some("jsonld_2".to_string()));
    assert_eq!(thumbnails[2].preference, Some(-2));
}

#[test]
fn test_extract_metadata_with_full_json_ld() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <script type="application/ld+json">
        {
            "@type": "VideoObject",
            "name": "Awesome Video",
            "description": "This is a great video",
            "thumbnailUrl": ["https://example.com/thumb1.jpg", "https://example.com/thumb2.jpg"],
            "uploadDate": "2024-01-15T10:30:00Z",
            "duration": "PT1H30M45S",
            "author": {
                "@type": "Person",
                "name": "John Doe"
            },
            "interactionStatistic": {
                "@type": "InteractionCounter",
                "interactionType": "WatchAction",
                "userInteractionCount": 50000
            },
            "keywords": "tag1, tag2, tag3",
            "genre": ["Comedy", "Action"]
        }
        </script>
        "#,
    );

    let metadata = base.extract_metadata(&html).unwrap();

    // Check all fields were extracted
    assert_eq!(metadata.title, "Awesome Video");
    assert_eq!(
        metadata.description,
        Some("This is a great video".to_string())
    );
    assert_eq!(metadata.uploader, Some("John Doe".to_string()));
    assert_eq!(
        metadata.thumbnail,
        Some("https://example.com/thumb1.jpg".to_string())
    );
    assert_eq!(metadata.thumbnails.as_ref().unwrap().len(), 2);
    assert_eq!(metadata.duration, Some(5445.0)); // 1h 30m 45s = 5445 seconds
    assert_eq!(metadata.upload_date, Some("20240115".to_string()));
    assert_eq!(metadata.view_count, Some(50000));
    assert_eq!(
        metadata.tags,
        Some(vec![
            "tag1".to_string(),
            "tag2".to_string(),
            "tag3".to_string()
        ])
    );
    assert_eq!(
        metadata.categories,
        Some(vec!["Comedy".to_string(), "Action".to_string()])
    );
}

#[test]
fn test_extract_metadata_without_json_ld() {
    let base = TnaFlixNetworkBase::new();

    let html = Html::parse_document(
        r#"
        <input name="title" value="Basic Title">
        <input name="description" value="Basic description">
        <input name="username" value="BasicUser">
        "#,
    );

    let metadata = base.extract_metadata(&html).unwrap();

    // Should still extract basic fields without JSON-LD
    assert_eq!(metadata.title, "Basic Title");
    assert_eq!(metadata.description, Some("Basic description".to_string()));
    assert_eq!(metadata.uploader, Some("BasicUser".to_string()));

    // Enhanced fields should be None without JSON-LD
    assert_eq!(metadata.duration, None);
    assert_eq!(metadata.upload_date, None);
    assert_eq!(metadata.view_count, None);
    assert_eq!(metadata.tags, None);
    assert_eq!(metadata.categories, None);
}
