//! InfoDict and related types for video metadata

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::Format;

/// Central metadata structure flowing through the pipeline
///
/// This structure contains all information about a video/audio that has been extracted.
/// It flows through the extraction -> download -> post-processing pipeline, with each
/// stage potentially adding or modifying information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoDict {
    // === Required fields ===
    /// Unique video identifier
    pub id: String,

    /// Video title
    pub title: String,

    /// Name of the extractor that provided this info
    pub extractor: String,

    /// Original webpage URL
    pub webpage_url: String,

    // === Optional common fields ===
    /// Video description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Video duration in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    /// URL of the best thumbnail
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,

    /// List of all available thumbnails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnails: Option<Vec<Thumbnail>>,

    /// Uploader name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,

    /// Uploader ID (username or channel ID)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader_id: Option<String>,

    /// Uploader URL (profile or channel page)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader_url: Option<String>,

    /// Channel name (may differ from uploader)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,

    /// Channel ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,

    /// Channel URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_url: Option<String>,

    /// Upload date in YYYYMMDD format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_date: Option<String>,

    /// View count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_count: Option<u64>,

    /// Like count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub like_count: Option<u64>,

    /// Dislike count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dislike_count: Option<u64>,

    /// Comment count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<u64>,

    /// Average rating (scale depends on site, often 0-5 or 0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_rating: Option<f64>,

    /// Age restriction (0 = no restriction)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_limit: Option<u8>,

    /// Video tags/keywords
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    /// Video categories
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,

    // === Format information ===
    /// All available formats
    pub formats: Vec<Format>,

    /// Formats selected by the user (after format selection)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_formats: Option<Vec<Format>>,

    // === Playlist information ===
    /// Playlist name (if this video is part of a playlist)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist: Option<String>,

    /// Playlist ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,

    /// Playlist title
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_title: Option<String>,

    /// Position in playlist (1-indexed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_index: Option<usize>,

    /// Total number of videos in playlist
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_count: Option<usize>,

    // === Subtitles ===
    /// Available subtitles (language code -> list of subtitle formats)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<HashMap<String, Vec<Subtitle>>>,

    /// Automatically generated captions (language code -> list of subtitle formats)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automatic_captions: Option<HashMap<String, Vec<Subtitle>>>,

    // === Chapters ===
    /// Video chapters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapters: Option<Vec<Chapter>>,

    // === Live stream information ===
    /// Whether this is a live stream
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_live: Option<bool>,

    /// Whether this is a live stream that has ended
    #[serde(skip_serializing_if = "Option::is_none")]
    pub was_live: Option<bool>,

    // === Additional metadata ===
    /// Release year
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_year: Option<u16>,

    /// Artist name (for music videos)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,

    /// Album name (for music videos)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,

    /// Track name (for music videos)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,

    /// Extractor-specific additional data
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl InfoDict {
    /// Create a new InfoDict with required fields
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        extractor: impl Into<String>,
        webpage_url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            extractor: extractor.into(),
            webpage_url: webpage_url.into(),
            description: None,
            duration: None,
            thumbnail: None,
            thumbnails: None,
            uploader: None,
            uploader_id: None,
            uploader_url: None,
            channel: None,
            channel_id: None,
            channel_url: None,
            upload_date: None,
            view_count: None,
            like_count: None,
            dislike_count: None,
            comment_count: None,
            average_rating: None,
            age_limit: None,
            tags: None,
            categories: None,
            formats: Vec::new(),
            requested_formats: None,
            playlist: None,
            playlist_id: None,
            playlist_title: None,
            playlist_index: None,
            playlist_count: None,
            subtitles: None,
            automatic_captions: None,
            chapters: None,
            is_live: None,
            was_live: None,
            release_year: None,
            artist: None,
            album: None,
            track: None,
            extra: HashMap::new(),
        }
    }

    /// Get the best format (highest quality video with audio, or best video+audio)
    #[must_use]
    pub fn best_format(&self) -> Option<&Format> {
        self.formats
            .iter()
            .filter(|f| f.has_video() && f.has_audio())
            .max_by(|a, b| {
                // Compare by quality, then resolution, then bitrate
                a.quality
                    .cmp(&b.quality)
                    .then(a.height.cmp(&b.height))
                    .then(
                        a.tbr
                            .partial_cmp(&b.tbr)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
            })
    }

    /// Get the best video-only format
    #[must_use]
    pub fn best_video(&self) -> Option<&Format> {
        self.formats
            .iter()
            .filter(|f| f.has_video())
            .max_by(|a, b| {
                a.quality
                    .cmp(&b.quality)
                    .then(a.height.cmp(&b.height))
                    .then(
                        a.vbr
                            .partial_cmp(&b.vbr)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
            })
    }

    /// Propagate video-level duration to all formats that lack it.
    ///
    /// Call this after setting both `self.duration` and `self.formats`.
    /// Formats that already have a duration (e.g., from HLS segment sums) are not overwritten.
    pub fn propagate_duration(&mut self) {
        if let Some(duration) = self.duration {
            for f in &mut self.formats {
                if f.duration.is_none() {
                    f.duration = Some(duration);
                }
            }
        }
    }

    /// Get the best audio-only format
    #[must_use]
    pub fn best_audio(&self) -> Option<&Format> {
        self.formats
            .iter()
            .filter(|f| f.has_audio())
            .max_by(|a, b| {
                a.abr
                    .partial_cmp(&b.abr)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

/// Thumbnail information
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    /// Thumbnail URL
    pub url: String,

    /// Thumbnail ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Width in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,

    /// Height in pixels
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,

    /// Quality preference (higher is better)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preference: Option<i32>,
}

/// Subtitle information
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subtitle {
    /// Subtitle URL
    pub url: String,

    /// Subtitle format (e.g., "vtt", "srt", "ass")
    pub ext: String,

    /// Human-readable name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Chapter information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    /// Chapter title
    pub title: String,

    /// Start time in seconds
    pub start_time: f64,

    /// End time in seconds
    pub end_time: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_info_dict_creation() {
        let info = InfoDict::new(
            "test123",
            "Test Video",
            "TestExtractor",
            "https://example.com/watch?v=test123",
        );

        assert_eq!(info.id, "test123");
        assert_eq!(info.title, "Test Video");
        assert_eq!(info.extractor, "TestExtractor");
        assert!(info.formats.is_empty());
    }

    #[test]
    fn test_serialize_deserialize() {
        let info = InfoDict::new(
            "test123",
            "Test Video",
            "TestExtractor",
            "https://example.com/watch?v=test123",
        );

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: InfoDict = serde_json::from_str(&json).unwrap();

        assert_eq!(info.id, deserialized.id);
        assert_eq!(info.title, deserialized.title);
    }
}
