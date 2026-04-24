//! Shared JSON-LD `VideoObject` parsing for all extractors.
//!
//! Extracts structured data from `<script type="application/ld+json">` blocks.
//! Site-specific extractors can use the parsed [`JsonLdVideo`] to extract
//! additional metadata (interaction stats, tags, etc.) beyond what the
//! shared helpers provide.

use scraper::Html;
use serde::Deserialize;

use super::selectors::JSONLD_SELECTOR;

// ============================================================================
// Core Types
// ============================================================================

/// Top-level JSON-LD container — can be a single object or `@graph` array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
enum JsonLdRoot {
    // Graph must be first — with untagged enums, serde tries in order,
    // and Single(JsonLdVideo) with all-Option fields matches anything.
    Graph(JsonLdGraph),
    Single(JsonLdVideo),
}

#[derive(Debug, Deserialize)]
struct JsonLdGraph {
    #[serde(rename = "@graph")]
    graph: Vec<JsonLdVideo>,
}

/// Parsed JSON-LD `VideoObject`.
///
/// All fields are optional to handle partial/malformed data gracefully.
/// Used by both site-specific extractors (TNAFlix, RedTube) and the
/// generic fallback extractor.
#[derive(Debug, Deserialize)]
pub(crate) struct JsonLdVideo {
    /// `@type` field — must be "VideoObject" (or array containing it)
    #[serde(rename = "@type")]
    pub json_type: Option<JsonLdType>,

    /// Video title
    pub name: Option<String>,

    /// Video description
    pub description: Option<String>,

    /// Thumbnail URL(s)
    #[serde(rename = "thumbnailUrl")]
    pub thumbnail_url: Option<JsonLdThumbnail>,

    /// Upload date (ISO 8601)
    #[serde(rename = "uploadDate")]
    pub upload_date: Option<String>,

    /// Duration (ISO 8601, e.g., "PT1H2M3S")
    pub duration: Option<String>,

    /// Direct media URL
    #[serde(rename = "contentUrl")]
    pub content_url: Option<String>,

    /// Embed/player URL (often an iframe, not direct media)
    #[serde(rename = "embedUrl")]
    pub embed_url: Option<String>,

    /// Author/uploader
    pub author: Option<JsonLdAuthor>,

    /// Interaction statistics (views, likes, etc.)
    #[serde(rename = "interactionStatistic")]
    pub interaction_statistic: Option<JsonLdInteractionStatistic>,

    /// Keywords/tags
    pub keywords: Option<JsonLdKeywords>,

    /// Genre/categories
    pub genre: Option<JsonLdGenre>,
}

// ============================================================================
// Flexible enum types for JSON-LD fields
// ============================================================================

/// `@type` can be a single string or an array of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdType {
    Single(String),
    Multiple(Vec<String>),
}

impl JsonLdType {
    /// Check if this type includes "VideoObject".
    pub fn is_video(&self) -> bool {
        match self {
            JsonLdType::Single(t) => t == "VideoObject",
            JsonLdType::Multiple(types) => types.iter().any(|t| t == "VideoObject"),
        }
    }
}

/// Thumbnail can be a string, array of strings, or an ImageObject.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdThumbnail {
    Single(String),
    Multiple(Vec<String>),
    Object(JsonLdImageObject),
}

/// ImageObject with a `url` field.
#[derive(Debug, Deserialize)]
pub(crate) struct JsonLdImageObject {
    pub url: Option<String>,
}

impl JsonLdThumbnail {
    /// Get the first thumbnail URL.
    pub fn first_url(&self) -> Option<&str> {
        match self {
            JsonLdThumbnail::Single(url) if !url.is_empty() => Some(url.as_str()),
            JsonLdThumbnail::Multiple(urls) => {
                urls.iter().find(|u| !u.is_empty()).map(|s| s.as_str())
            }
            JsonLdThumbnail::Object(obj) => obj.url.as_deref().filter(|u| !u.is_empty()),
            _ => None,
        }
    }
}

/// Author can be a string or an object.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdAuthor {
    String(String),
    Object(JsonLdAuthorObject),
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonLdAuthorObject {
    #[serde(rename = "@type")]
    #[allow(dead_code)]
    pub author_type: Option<String>,
    pub name: Option<String>,
    pub url: Option<String>,
    #[serde(rename = "@id")]
    pub id: Option<String>,
}

impl JsonLdAuthor {
    pub fn name(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(s) => Some(s.clone()),
            JsonLdAuthor::Object(obj) => obj.name.clone(),
        }
    }

    pub fn url(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(_) => None,
            JsonLdAuthor::Object(obj) => obj.url.clone(),
        }
    }

    pub fn id(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(_) => None,
            JsonLdAuthor::Object(obj) => obj.id.clone(),
        }
    }
}

/// Interaction statistics — single or multiple entries.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdInteractionStatistic {
    Single(JsonLdInteraction),
    Multiple(Vec<JsonLdInteraction>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonLdInteraction {
    #[serde(rename = "@type")]
    pub interaction_type: String,
    #[serde(rename = "interactionType")]
    pub interaction_type_url: Option<String>,
    #[serde(rename = "userInteractionCount")]
    pub user_interaction_count: Option<u64>,
}

/// Keywords — comma-separated string or array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdKeywords {
    Single(String),
    Multiple(Vec<String>),
}

/// Genre — string or array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdGenre {
    Single(String),
    Multiple(Vec<String>),
}

// ============================================================================
// Core extraction functions
// ============================================================================

/// Extract the first JSON-LD `VideoObject` from HTML.
///
/// Searches all `<script type="application/ld+json">` blocks for a
/// `VideoObject`, including inside `@graph` arrays.
pub(crate) fn extract_json_ld(html: &Html) -> Option<JsonLdVideo> {
    for script_elem in html.select(&JSONLD_SELECTOR) {
        let json_text = script_elem.text().collect::<String>();
        if let Some(video) = parse_video_object(&json_text) {
            return Some(video);
        }
    }
    None
}

/// Parse a JSON-LD text block and extract the first `VideoObject`.
fn parse_video_object(json_text: &str) -> Option<JsonLdVideo> {
    if let Ok(root) = serde_json::from_str::<JsonLdRoot>(json_text) {
        match root {
            JsonLdRoot::Single(obj) if obj.json_type.as_ref().is_some_and(|t| t.is_video()) => {
                return Some(obj);
            }
            JsonLdRoot::Graph(graph) => {
                for obj in graph.graph {
                    if obj.json_type.as_ref().is_some_and(|t| t.is_video()) {
                        return Some(obj);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Get the first thumbnail URL from a parsed `JsonLdVideo`.
pub(crate) fn get_thumbnail_url(json_ld: &JsonLdVideo) -> Option<String> {
    json_ld
        .thumbnail_url
        .as_ref()
        .and_then(|t| t.first_url())
        .map(|s| s.to_string())
}

/// Create a thumbnail list from the `thumbnailUrl` field.
pub(crate) fn extract_thumbnails(json_ld: &JsonLdVideo) -> Option<Vec<rdlp_types::Thumbnail>> {
    json_ld.thumbnail_url.as_ref().map(|thumb| {
        let urls: Vec<&str> = match thumb {
            JsonLdThumbnail::Single(url) => vec![url.as_str()],
            JsonLdThumbnail::Multiple(urls) => urls.iter().map(|s| s.as_str()).collect(),
            JsonLdThumbnail::Object(obj) => obj.url.as_deref().map(|u| vec![u]).unwrap_or_default(),
        };

        urls.iter()
            .enumerate()
            .filter(|(_, url)| !url.is_empty())
            .map(|(idx, url)| rdlp_types::Thumbnail {
                url: (*url).to_string(),
                id: Some(format!("jsonld_{idx}")),
                width: None,
                height: None,
                preference: Some(-(idx as i32)),
            })
            .collect()
    })
}

/// Extract interaction count by action type (e.g., "WatchAction", "LikeAction").
pub(crate) fn extract_interaction_count(json_ld: &JsonLdVideo, action_type: &str) -> Option<u64> {
    json_ld.interaction_statistic.as_ref().and_then(|stats| {
        let interactions = match stats {
            JsonLdInteractionStatistic::Single(interaction) => vec![interaction],
            JsonLdInteractionStatistic::Multiple(interactions) => interactions.iter().collect(),
        };

        for interaction in interactions {
            if interaction.interaction_type == action_type
                || interaction
                    .interaction_type_url
                    .as_ref()
                    .is_some_and(|url| url.contains(action_type))
            {
                return interaction.user_interaction_count;
            }
        }

        None
    })
}

/// Extract view count from JSON-LD interaction statistics.
pub(crate) fn extract_view_count(json_ld: &JsonLdVideo) -> Option<u64> {
    extract_interaction_count(json_ld, "WatchAction")
}

/// Extract like count from JSON-LD interaction statistics.
pub(crate) fn extract_like_count(json_ld: &JsonLdVideo) -> Option<u64> {
    extract_interaction_count(json_ld, "LikeAction")
}

/// Extract tags/keywords from JSON-LD.
pub(crate) fn extract_tags(json_ld: &JsonLdVideo) -> Option<Vec<String>> {
    json_ld.keywords.as_ref().map(|keywords| match keywords {
        JsonLdKeywords::Single(s) => s
            .split(',')
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect(),
        JsonLdKeywords::Multiple(vec) => vec.clone(),
    })
}

/// Extract categories/genres from JSON-LD.
pub(crate) fn extract_categories(json_ld: &JsonLdVideo) -> Option<Vec<String>> {
    json_ld.genre.as_ref().map(|genre| match genre {
        JsonLdGenre::Single(s) => vec![s.clone()],
        JsonLdGenre::Multiple(vec) => vec.clone(),
    })
}

/// Parse ISO 8601 duration (e.g., "PT1H2M3S") to seconds.
pub(crate) fn parse_iso8601_duration(duration: &str) -> Option<f64> {
    use super::selectors::ISO8601_DURATION_PATTERN;

    let caps = ISO8601_DURATION_PATTERN.captures(duration)?;
    let hours: f64 = caps
        .get(1)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0.0);
    let minutes: f64 = caps
        .get(2)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0.0);
    let seconds: f64 = caps
        .get(3)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0.0);

    let total = hours * 3600.0 + minutes * 60.0 + seconds;
    if total > 0.0 { Some(total) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single_video_object() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@type": "VideoObject", "name": "Test", "contentUrl": "https://cdn.example.com/v.mp4"}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).unwrap();
        assert_eq!(video.name.as_deref(), Some("Test"));
        assert_eq!(
            video.content_url.as_deref(),
            Some("https://cdn.example.com/v.mp4")
        );
    }

    #[test]
    fn extract_graph_array() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@graph": [
            {"@type": "WebPage", "name": "Page"},
            {"@type": "VideoObject", "name": "Video", "contentUrl": "https://cdn.example.com/v.mp4"}
        ]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).unwrap();
        assert_eq!(video.name.as_deref(), Some("Video"));
    }

    #[test]
    fn non_video_object_ignored() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@type": "Article", "name": "Not a video"}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        assert!(extract_json_ld(&html).is_none());
    }

    #[test]
    fn thumbnail_single_string() {
        let thumb = JsonLdThumbnail::Single("https://example.com/thumb.jpg".to_string());
        assert_eq!(thumb.first_url(), Some("https://example.com/thumb.jpg"));
    }

    #[test]
    fn thumbnail_image_object() {
        let thumb = JsonLdThumbnail::Object(JsonLdImageObject {
            url: Some("https://example.com/thumb.jpg".to_string()),
        });
        assert_eq!(thumb.first_url(), Some("https://example.com/thumb.jpg"));
    }

    #[test]
    fn thumbnail_empty_rejected() {
        let thumb = JsonLdThumbnail::Single(String::new());
        assert_eq!(thumb.first_url(), None);
    }

    #[test]
    fn iso8601_duration() {
        assert_eq!(parse_iso8601_duration("PT1H2M3S"), Some(3723.0));
        assert_eq!(parse_iso8601_duration("PT30S"), Some(30.0));
        assert_eq!(parse_iso8601_duration("PT1.5S"), Some(1.5));
        assert_eq!(parse_iso8601_duration("invalid"), None);
    }

    #[test]
    fn author_name_from_string() {
        let author = JsonLdAuthor::String("John".to_string());
        assert_eq!(author.name(), Some("John".to_string()));
        assert_eq!(author.url(), None);
    }

    #[test]
    fn author_name_from_object() {
        let author = JsonLdAuthor::Object(JsonLdAuthorObject {
            author_type: None,
            name: Some("Jane".to_string()),
            url: Some("https://example.com/jane".to_string()),
            id: None,
        });
        assert_eq!(author.name(), Some("Jane".to_string()));
        assert_eq!(author.url(), Some("https://example.com/jane".to_string()));
    }
}
