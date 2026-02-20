//! JSON-LD parsing for TNAFlix network sites
//!
//! Provides structures and parsing logic for JSON-LD VideoObject metadata
//! commonly found in video site HTML.

use scraper::{Html, Selector};
use serde::Deserialize;

use std::sync::LazyLock;

/// Selector for JSON-LD script tags: <script type="application/ld+json">
pub(crate) static JSONLD_SELECTOR: LazyLock<Selector> = LazyLock::new(|| {
    Selector::parse(r#"script[type="application/ld+json"]"#).expect("Valid JSON-LD selector")
});

// ============================================================================
// JSON-LD Structures
// ============================================================================

/// JSON-LD VideoObject structure for structured metadata extraction
#[derive(Debug, Deserialize)]
pub(crate) struct JsonLdVideo {
    #[serde(rename = "@type")]
    pub json_type: String,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "thumbnailUrl")]
    pub thumbnail_url: Option<JsonLdThumbnail>,
    #[serde(rename = "uploadDate")]
    pub upload_date: Option<String>,
    pub duration: Option<String>,
    pub author: Option<JsonLdAuthor>,
    #[serde(rename = "interactionStatistic")]
    pub interaction_statistic: Option<JsonLdInteractionStatistic>,
    pub keywords: Option<JsonLdKeywords>,
    pub genre: Option<JsonLdGenre>,
    #[serde(rename = "contentUrl")]
    #[allow(dead_code)] // Deserialized from JSON-LD but not currently used
    pub content_url: Option<String>,
}

/// JSON-LD thumbnail can be string or array
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdThumbnail {
    Single(String),
    Multiple(Vec<String>),
}

/// JSON-LD author can be a string or an object
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdAuthor {
    /// Simple string author (e.g., "Hope Heaven")
    String(String),
    /// Object author with name, url, etc.
    Object(JsonLdAuthorObject),
}

/// JSON-LD author object structure
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
    /// Get the author name regardless of format
    pub fn name(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(s) => Some(s.clone()),
            JsonLdAuthor::Object(obj) => obj.name.clone(),
        }
    }

    /// Get the author URL if available
    pub fn url(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(_) => None,
            JsonLdAuthor::Object(obj) => obj.url.clone(),
        }
    }

    /// Get the author ID if available
    pub fn id(&self) -> Option<String> {
        match self {
            JsonLdAuthor::String(_) => None,
            JsonLdAuthor::Object(obj) => obj.id.clone(),
        }
    }
}

/// JSON-LD interaction statistic for view counts, likes, etc.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdInteractionStatistic {
    Single(JsonLdInteraction),
    Multiple(Vec<JsonLdInteraction>),
}

/// Individual interaction statistic entry
#[derive(Debug, Deserialize)]
pub(crate) struct JsonLdInteraction {
    #[serde(rename = "@type")]
    pub interaction_type: String,
    #[serde(rename = "interactionType")]
    pub interaction_type_url: Option<String>,
    #[serde(rename = "userInteractionCount")]
    pub user_interaction_count: Option<u64>,
}

/// JSON-LD keywords can be string (comma-separated) or array
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdKeywords {
    Single(String),
    Multiple(Vec<String>),
}

/// JSON-LD genre can be string or array
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum JsonLdGenre {
    Single(String),
    Multiple(Vec<String>),
}

// ============================================================================
// JSON-LD Extraction Functions
// ============================================================================

/// Extract JSON-LD VideoObject from HTML
pub(crate) fn extract_json_ld(html: &Html) -> Option<JsonLdVideo> {
    for script_elem in html.select(&JSONLD_SELECTOR) {
        let json_text = script_elem.text().collect::<String>();

        if let Ok(json_ld) = serde_json::from_str::<JsonLdVideo>(&json_text) {
            if json_ld.json_type == "VideoObject" {
                return Some(json_ld);
            }
        }
    }
    None
}

/// Extract view count from JSON-LD interaction statistics
pub(crate) fn extract_view_count(json_ld: &JsonLdVideo) -> Option<u64> {
    extract_interaction_count(json_ld, "WatchAction")
}

/// Extract like count from JSON-LD interaction statistics
pub(crate) fn extract_like_count(json_ld: &JsonLdVideo) -> Option<u64> {
    extract_interaction_count(json_ld, "LikeAction")
}

/// Extract interaction count by action type from JSON-LD
fn extract_interaction_count(json_ld: &JsonLdVideo, action_type: &str) -> Option<u64> {
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

/// Extract tags/keywords from JSON-LD
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

/// Extract categories/genres from JSON-LD
pub(crate) fn extract_categories(json_ld: &JsonLdVideo) -> Option<Vec<String>> {
    json_ld.genre.as_ref().map(|genre| match genre {
        JsonLdGenre::Single(s) => vec![s.clone()],
        JsonLdGenre::Multiple(vec) => vec.clone(),
    })
}

/// Create thumbnail list from JSON-LD thumbnailUrl field
pub(crate) fn extract_thumbnails(json_ld: &JsonLdVideo) -> Option<Vec<rdlp_core::Thumbnail>> {
    json_ld.thumbnail_url.as_ref().map(|thumb| {
        let urls: &[String] = match thumb {
            JsonLdThumbnail::Single(url) => std::slice::from_ref(url),
            JsonLdThumbnail::Multiple(urls) => urls,
        };

        urls.iter()
            .enumerate()
            .filter(|(_, url)| !url.is_empty())
            .map(|(idx, url)| rdlp_core::Thumbnail {
                url: url.clone(),
                id: Some(format!("jsonld_{idx}")),
                width: None,
                height: None,
                preference: Some(-(idx as i32)),
            })
            .collect()
    })
}

/// Get first thumbnail URL from JSON-LD
pub(crate) fn get_thumbnail_url(json_ld: &JsonLdVideo) -> Option<String> {
    json_ld.thumbnail_url.as_ref().and_then(|thumb| {
        let url = match thumb {
            JsonLdThumbnail::Single(url) => url,
            JsonLdThumbnail::Multiple(urls) => urls.first()?,
        };
        (!url.is_empty()).then(|| url.clone())
    })
}
