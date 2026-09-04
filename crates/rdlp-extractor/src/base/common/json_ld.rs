//! Shared JSON-LD `VideoObject` parsing for all extractors.
//!
//! Extracts structured data from `<script type="application/ld+json">` blocks.
//! Site-specific extractors can use the parsed [`JsonLdVideo`] to extract
//! additional metadata (interaction stats, tags, etc.) beyond what the
//! shared helpers provide.

use scraper::Html;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::selectors::JSONLD_SELECTOR;

/// Deserialize a tolerant optional field: an unrecognised shape costs THAT
/// FIELD only, never the object containing it.
///
/// [`JsonLdVideo`] is deserialized inside an untagged [`JsonLdRoot`], where a
/// sub-field serde cannot shape-match aborts the ENTIRE deserialization. The
/// untagged enum then falls through to its next variant and the page reads as
/// though it carried no JSON-LD at all — silently losing title, duration,
/// description, thumbnail and tags because of one unmodelled corner. Real
/// pages hit this: schema.org lets any property be an array, counts appear as
/// strings, and `@type` is routinely absent.
///
/// Buffering into a `serde_json::Value` always succeeds for well-formed JSON;
/// only the typed conversion is allowed to fail, and it fails into `None`.
/// This is what makes [`JsonLdVideo`]'s "all fields are optional to handle
/// partial/malformed data gracefully" contract actually hold.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// Deserialize an optional count that may arrive as a number OR as a
/// number-shaped string.
///
/// schema.org types `userInteractionCount` as an Integer, but sites commonly
/// emit `"21400"`. Under [`lenient`] alone that shape is merely *tolerated* —
/// the enclosing object survives, but the count silently reads as absent,
/// which is the same invisible metadata loss in miniature. One coercion
/// avoids it.
///
/// Fail-soft is preserved: this both coerces and swallows, because a field can
/// carry only one `deserialize_with`, so the two cannot be layered. Any shape
/// that is neither a number nor a parseable string costs this field only.
fn lenient_count<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

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
pub struct JsonLdVideo {
    /// `@type` field — must be "VideoObject" (or array containing it)
    #[serde(rename = "@type", default, deserialize_with = "lenient")]
    pub json_type: Option<JsonLdType>,

    /// Video title
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,

    /// Video description
    #[serde(default, deserialize_with = "lenient")]
    pub description: Option<String>,

    /// Thumbnail URL(s)
    #[serde(rename = "thumbnailUrl", default, deserialize_with = "lenient")]
    pub thumbnail_url: Option<JsonLdThumbnail>,

    /// Upload date (ISO 8601)
    #[serde(rename = "uploadDate", default, deserialize_with = "lenient")]
    pub upload_date: Option<String>,

    /// Duration (ISO 8601, e.g., "PT1H2M3S")
    #[serde(default, deserialize_with = "lenient")]
    pub duration: Option<String>,

    /// Direct media URL
    #[serde(rename = "contentUrl", default, deserialize_with = "lenient")]
    pub content_url: Option<String>,

    /// MIME type of `contentUrl` (e.g. `"video/webm"`).
    ///
    /// Used as a fallback to derive the file extension when `contentUrl`
    /// itself has no recognizable extension (issue #496).
    #[serde(rename = "encodingFormat", default, deserialize_with = "lenient")]
    pub encoding_format: Option<String>,

    /// Embed/player URL (often an iframe, not direct media)
    #[serde(rename = "embedUrl", default, deserialize_with = "lenient")]
    pub embed_url: Option<String>,

    /// Author/uploader
    #[serde(default, deserialize_with = "lenient")]
    pub author: Option<JsonLdAuthor>,

    /// Interaction statistics (views, likes, etc.)
    #[serde(rename = "interactionStatistic", default, deserialize_with = "lenient")]
    pub interaction_statistic: Option<JsonLdInteractionStatistic>,

    /// Keywords/tags
    #[serde(default, deserialize_with = "lenient")]
    pub keywords: Option<JsonLdKeywords>,

    /// Genre/categories
    #[serde(default, deserialize_with = "lenient")]
    pub genre: Option<JsonLdGenre>,
}

// ============================================================================
// Flexible enum types for JSON-LD fields
// ============================================================================

/// `@type` can be a single string or an array of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdType {
    /// Single `@type` string value.
    Single(String),
    /// Array of `@type` string values.
    Multiple(Vec<String>),
}

impl JsonLdType {
    /// Check if this type includes "VideoObject".
    #[must_use]
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
pub enum JsonLdThumbnail {
    /// Single thumbnail URL string.
    Single(String),
    /// Array of thumbnail URL strings.
    Multiple(Vec<String>),
    /// Structured ImageObject with a `url` field.
    Object(JsonLdImageObject),
}

/// ImageObject with a `url` field.
#[derive(Debug, Deserialize)]
pub struct JsonLdImageObject {
    /// Canonical URL of the image.
    pub url: Option<String>,
}

impl JsonLdThumbnail {
    /// Get the first thumbnail URL.
    #[must_use]
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
pub enum JsonLdAuthor {
    /// Author represented as a plain name string.
    String(String),
    /// Author represented as a structured object with name, URL, and id.
    Object(JsonLdAuthorObject),
}

/// Structured author object from JSON-LD.
#[derive(Debug, Deserialize)]
pub struct JsonLdAuthorObject {
    /// `@type` of the author entity (e.g. "Person", "Organization").
    #[serde(rename = "@type")]
    #[allow(dead_code)]
    pub author_type: Option<String>,
    /// Display name of the author.
    pub name: Option<String>,
    /// Profile URL of the author.
    pub url: Option<String>,
    /// `@id` identifier for the author entity.
    #[serde(rename = "@id")]
    pub id: Option<String>,
}

impl JsonLdAuthor {
    /// Returns the author's display name.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(s) => Some(s.clone()),
            JsonLdAuthor::Object(obj) => obj.name.clone(),
        }
    }

    /// Returns the author's profile URL, if the author is an object.
    #[must_use]
    pub fn url(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(_) => None,
            JsonLdAuthor::Object(obj) => obj.url.clone(),
        }
    }

    /// Returns the author's `@id` identifier, if the author is an object.
    #[must_use]
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
pub enum JsonLdInteractionStatistic {
    // Multiple must be first, for the same reason `JsonLdRoot` puts `Graph`
    // first: serde tries untagged variants in declaration order, and serde
    // will deserialize a struct from a SEQUENCE positionally. Since every
    // `JsonLdInteraction` field is optional and defaulted, `Single` would
    // otherwise swallow a two-entry array — binding the entries to the wrong
    // fields and silently losing the WatchAction counter.
    /// Multiple interaction statistic entries.
    Multiple(Vec<JsonLdInteraction>),
    /// Single interaction statistic entry.
    Single(JsonLdInteraction),
}

/// The kind of interaction being counted.
///
/// schema.org permits `interactionType` to be either a URL/name string or an
/// embedded Action object, and real sites emit both (PornoXO uses the object
/// form). Accepting only one shape is not a cosmetic limitation: because
/// `interactionStatistic` hangs off [`JsonLdVideo`], a deserialization failure
/// here discards the whole `VideoObject` — title, duration and tags included —
/// leaving the page looking as though it carried no JSON-LD at all.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdInteractionType {
    /// URL or bare name, e.g. `"https://schema.org/WatchAction"`.
    Url(String),
    /// Embedded Action object, e.g. `{"@type": "WatchAction"}`.
    Object(JsonLdInteractionTypeObject),
}

/// The embedded-object form of `interactionType`.
#[derive(Debug, Deserialize)]
pub struct JsonLdInteractionTypeObject {
    /// `@type` naming the action, e.g. `"WatchAction"`.
    #[serde(rename = "@type")]
    pub json_type: Option<String>,
}

impl JsonLdInteractionType {
    /// Whether this names `action_type` (e.g. `"WatchAction"`), in either shape.
    ///
    /// The two arms match differently on purpose. `Url` holds a URI
    /// (`"https://schema.org/WatchAction"`), so the action name is a substring
    /// of it. `Object` holds a bare `@type` name, which is the action name
    /// exactly — so it is compared exactly. Substring-matching a bare name
    /// would be looseness with no reason, in the matcher that decides which
    /// counter is the view count.
    #[must_use]
    pub fn matches(&self, action_type: &str) -> bool {
        match self {
            Self::Url(url) => url.contains(action_type),
            Self::Object(obj) => obj.json_type.as_deref() == Some(action_type),
        }
    }
}

/// A single JSON-LD `InteractionCounter` entry.
#[derive(Debug, Deserialize)]
pub struct JsonLdInteraction {
    /// `@type` of this entry itself (e.g. "InteractionCounter").
    ///
    /// Optional, per [`JsonLdVideo`]'s contract: real pages omit it, and a
    /// missing `@type` must not cost the enclosing `VideoObject`.
    #[serde(rename = "@type", default, deserialize_with = "lenient")]
    pub json_type: Option<String>,
    /// The action this entry counts (e.g. "WatchAction"), as a URL or object.
    #[serde(rename = "interactionType", default, deserialize_with = "lenient")]
    pub interaction_type_ref: Option<JsonLdInteractionType>,
    /// Count of user interactions recorded for this entry.
    #[serde(
        rename = "userInteractionCount",
        default,
        deserialize_with = "lenient_count"
    )]
    pub user_interaction_count: Option<u64>,
}

/// Keywords — comma-separated string or array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdKeywords {
    /// Comma-separated keyword string.
    Single(String),
    /// Pre-split array of keyword strings.
    Multiple(Vec<String>),
}

/// Genre — string or array.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonLdGenre {
    /// Single genre string.
    Single(String),
    /// Array of genre strings.
    Multiple(Vec<String>),
}

// ============================================================================
// Core extraction functions
// ============================================================================

/// Extract the first JSON-LD `VideoObject` from HTML.
///
/// Searches all `<script type="application/ld+json">` blocks for a
/// `VideoObject`, including inside `@graph` arrays.
#[must_use]
pub fn extract_json_ld(html: &Html) -> Option<JsonLdVideo> {
    html.select(&JSONLD_SELECTOR)
        .find_map(|script_elem| parse_video_object(&script_elem.text().collect::<String>()))
}

/// Parse a JSON-LD text block and extract the first `VideoObject`.
fn parse_video_object(json_text: &str) -> Option<JsonLdVideo> {
    if let Ok(root) = serde_json::from_str::<JsonLdRoot>(json_text) {
        match root {
            JsonLdRoot::Single(obj) if obj.json_type.as_ref().is_some_and(|t| t.is_video()) => {
                return Some(obj);
            }
            JsonLdRoot::Graph(graph) => {
                return graph
                    .graph
                    .into_iter()
                    .find(|obj| obj.json_type.as_ref().is_some_and(|t| t.is_video()));
            }
            _ => {}
        }
    }
    None
}

/// Get the first thumbnail URL from a parsed `JsonLdVideo`.
#[must_use]
pub fn get_thumbnail_url(json_ld: &JsonLdVideo) -> Option<String> {
    json_ld
        .thumbnail_url
        .as_ref()
        .and_then(|t| t.first_url())
        .map(|s| s.to_string())
}

/// Create a thumbnail list from the `thumbnailUrl` field.
#[must_use]
pub fn extract_thumbnails(json_ld: &JsonLdVideo) -> Option<Vec<rdlp_types::Thumbnail>> {
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
#[must_use]
pub fn extract_interaction_count(json_ld: &JsonLdVideo, action_type: &str) -> Option<u64> {
    json_ld.interaction_statistic.as_ref().and_then(|stats| {
        let interactions = match stats {
            JsonLdInteractionStatistic::Single(interaction) => vec![interaction],
            JsonLdInteractionStatistic::Multiple(interactions) => interactions.iter().collect(),
        };

        for interaction in interactions {
            if interaction.json_type.as_deref() == Some(action_type)
                || interaction
                    .interaction_type_ref
                    .as_ref()
                    .is_some_and(|kind| kind.matches(action_type))
            {
                return interaction.user_interaction_count;
            }
        }

        None
    })
}

/// Extract view count from JSON-LD interaction statistics.
#[must_use]
pub fn extract_view_count(json_ld: &JsonLdVideo) -> Option<u64> {
    extract_interaction_count(json_ld, "WatchAction")
}

/// Extract like count from JSON-LD interaction statistics.
#[must_use]
pub fn extract_like_count(json_ld: &JsonLdVideo) -> Option<u64> {
    extract_interaction_count(json_ld, "LikeAction")
}

/// Extract tags/keywords from JSON-LD.
#[must_use]
pub fn extract_tags(json_ld: &JsonLdVideo) -> Option<Vec<String>> {
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
#[must_use]
pub fn extract_categories(json_ld: &JsonLdVideo) -> Option<Vec<String>> {
    json_ld.genre.as_ref().map(|genre| match genre {
        JsonLdGenre::Single(s) => vec![s.clone()],
        JsonLdGenre::Multiple(vec) => vec.clone(),
    })
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

    /// schema.org allows `interactionType` to be either a URL string or an
    /// embedded Action object. The object form must not break the parse:
    /// because `interactionStatistic` sits on `JsonLdVideo`, a deserialization
    /// failure there loses the ENTIRE `VideoObject` (title, duration, tags),
    /// not merely the view count. PornoXO emits the object form.
    #[test]
    fn interaction_type_as_embedded_object() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@graph": [{"@type": "VideoObject", "name": "Video", "duration": "PT15M30S",
          "interactionStatistic": {"@type": "InteractionCounter",
            "interactionType": {"@type": "WatchAction"}, "userInteractionCount": 21}}]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("object-form interactionType must still parse");
        assert_eq!(video.name.as_deref(), Some("Video"));
        assert_eq!(video.duration.as_deref(), Some("PT15M30S"));
        assert_eq!(extract_view_count(&video), Some(21));
    }

    // ------------------------------------------------------------------
    // Cascade guards.
    //
    // `JsonLdVideo` is deserialized inside an untagged `JsonLdRoot`, so any
    // sub-field serde cannot shape-match used to abort the WHOLE
    // deserialization — costing title, duration, description, thumbnail and
    // tags, and leaving the page looking as though it carried no JSON-LD.
    // Each test below feeds one plausible-but-unmodelled shape and asserts the
    // unrelated fields SURVIVE. They pin the fail-soft mechanism, not the
    // individual shapes.
    // ------------------------------------------------------------------

    /// JSON-LD permits an array for any property.
    ///
    /// Uses a TWO-element array deliberately: serde will deserialize a
    /// one-element sequence positionally into the single-field `Object`
    /// variant, so a one-element array does not exercise the mismatch at all.
    #[test]
    fn array_valued_interaction_type_does_not_discard_the_video() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@graph": [{"@type": "VideoObject", "name": "Video", "duration": "PT15M30S",
          "interactionStatistic": {"@type": "InteractionCounter",
            "interactionType": ["https://schema.org/WatchAction",
                                "https://schema.org/LikeAction"],
            "userInteractionCount": 21}}]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("array interactionType must not lose the video");
        assert_eq!(video.name.as_deref(), Some("Video"));
        assert_eq!(video.duration.as_deref(), Some("PT15M30S"));
    }

    /// String-typed counts are common in the wild.
    #[test]
    fn string_typed_interaction_count_does_not_discard_the_video() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@graph": [{"@type": "VideoObject", "name": "Video", "duration": "PT15M30S",
          "interactionStatistic": {"@type": "InteractionCounter",
            "interactionType": {"@type": "WatchAction"},
            "userInteractionCount": "21400"}}]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("string count must not lose the video");
        assert_eq!(video.name.as_deref(), Some("Video"));
        assert_eq!(video.duration.as_deref(), Some("PT15M30S"));
        // Fail-soft alone would turn a total loss into a SILENT field loss.
        // The count is coerced, so a string-encoded count is still readable.
        assert_eq!(extract_view_count(&video), Some(21400));
    }

    /// The coercion does not cost fail-soft: a count that is neither a number
    /// nor a parseable string still costs only that field.
    #[test]
    fn uncoercible_interaction_count_costs_only_that_field() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@type": "VideoObject", "name": "Video", "duration": "PT15M30S",
         "interactionStatistic": {"@type": "InteractionCounter",
           "interactionType": {"@type": "WatchAction"},
           "userInteractionCount": {"value": "lots"}}}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("odd count must not lose the video");
        assert_eq!(video.name.as_deref(), Some("Video"));
        assert_eq!(video.duration.as_deref(), Some("PT15M30S"));
        assert_eq!(extract_view_count(&video), None);
    }

    /// `@type` is absent — `JsonLdVideo`'s docstring promises every field is
    /// optional, so a counter entry without one must not be fatal.
    #[test]
    fn interaction_entry_without_type_does_not_discard_the_video() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@graph": [{"@type": "VideoObject", "name": "Video", "duration": "PT15M30S",
          "interactionStatistic": {
            "interactionType": {"@type": "WatchAction"},
            "userInteractionCount": 21}}]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("missing @type must not lose the video");
        assert_eq!(video.name.as_deref(), Some("Video"));
        assert_eq!(video.duration.as_deref(), Some("PT15M30S"));
        // The entry is still usable: only the unmodelled part is dropped.
        assert_eq!(extract_view_count(&video), Some(21));
    }

    /// The mechanism is not specific to `interactionStatistic`: an unmodelled
    /// shape on any tolerant field costs that field alone.
    #[test]
    fn unmodelled_shape_on_another_field_does_not_discard_the_video() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@graph": [{"@type": "VideoObject", "name": "Video", "duration": "PT15M30S",
          "keywords": {"unexpected": "object"},
          "thumbnailUrl": 12345}]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("odd sub-field shapes must not lose the video");
        assert_eq!(video.name.as_deref(), Some("Video"));
        assert_eq!(video.duration.as_deref(), Some("PT15M30S"));
        assert!(extract_tags(&video).is_none());
        assert!(get_thumbnail_url(&video).is_none());
    }

    /// The object arm carries a bare `@type` NAME, so it matches exactly; the
    /// URL arm carries a URI, so the name is a substring of it. A bare name
    /// that merely CONTAINS the action must not be treated as that action.
    #[test]
    fn object_arm_matches_exactly_url_arm_matches_by_substring() {
        let obj = JsonLdInteractionType::Object(JsonLdInteractionTypeObject {
            json_type: Some("WatchAction".to_string()),
        });
        assert!(obj.matches("WatchAction"));
        assert!(!obj.matches("Watch"), "bare @type must match exactly");

        let url = JsonLdInteractionType::Url("https://schema.org/WatchAction".to_string());
        assert!(url.matches("WatchAction"));
        assert!(!url.matches("LikeAction"));
    }

    /// Guards the variant ORDER of `JsonLdInteractionStatistic`.
    ///
    /// serde deserializes a struct from a sequence positionally, so once every
    /// `JsonLdInteraction` field became optional and defaulted, a `Single`
    /// declared first would swallow a two-entry array — binding entry 0 to
    /// `@type` and entry 1 to `interactionType`, and losing the WatchAction
    /// counter. Reordering is the fix; this test fails if it is undone.
    #[test]
    fn multi_entry_statistics_are_not_swallowed_by_the_single_variant() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@type": "VideoObject", "name": "Video",
         "interactionStatistic": [
           {"@type": "InteractionCounter", "interactionType": {"@type": "LikeAction"},
            "userInteractionCount": 500},
           {"@type": "InteractionCounter", "interactionType": {"@type": "WatchAction"},
            "userInteractionCount": 98765}]}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("video must parse");
        assert_eq!(extract_view_count(&video), Some(98765));
        assert_eq!(extract_like_count(&video), Some(500));
    }

    /// A near-miss `@type` must not be counted as the view count.
    #[test]
    fn view_count_ignores_a_substring_only_object_type() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@type": "VideoObject", "name": "Video",
         "interactionStatistic": {"@type": "InteractionCounter",
           "interactionType": {"@type": "WatchActionExtended"},
           "userInteractionCount": 99}}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("video must parse");
        assert_eq!(extract_view_count(&video), None);
    }

    /// The string form stays supported — this is a widening, not a swap.
    #[test]
    fn interaction_type_as_url_string() {
        let html_str = r#"<html><head><script type="application/ld+json">
        {"@type": "VideoObject", "name": "Video",
         "interactionStatistic": {"@type": "InteractionCounter",
           "interactionType": "https://schema.org/WatchAction", "userInteractionCount": 7}}
        </script></head></html>"#;
        let html = Html::parse_document(html_str);
        let video = extract_json_ld(&html).expect("string-form interactionType must parse");
        assert_eq!(extract_view_count(&video), Some(7));
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
