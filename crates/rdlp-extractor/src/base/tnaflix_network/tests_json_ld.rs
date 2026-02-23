//! JSON-LD data extraction tests (view count, tags, categories, thumbnails)

use super::*;
use scraper::Html;

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
