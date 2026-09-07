//! `InfoDict` and related types for video metadata

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

    /// Actors / performers / cast
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<String>,

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
    /// Decode HTML entities across every display string this carries.
    ///
    /// The single boundary for entity decoding on the extraction path (#698).
    /// The orchestrator calls it once on each `InfoDict` an extractor returns,
    /// so no extractor has to remember — which is the failure that put
    /// `&quot;` and `&#039;` into real filenames.
    ///
    /// Ids and the numeric fields are NOT touched: an id is an opaque site
    /// token rather than text a person reads.
    ///
    /// The METADATA url fields — `webpage_url`, `thumbnail`, `thumbnails[]`,
    /// `uploader_url`, `channel_url`, and subtitle `track.url` — do NOT get
    /// the display decoder: in a URL `&` is a query separator, and decoding
    /// `&sol;` or `&lt;` there would invent structure. They get
    /// `repair_url_entities` instead, which undoes the one entity an HTML
    /// attribute serializer forces on them.
    ///
    /// `formats[].url` and the fragment URLs are deliberately NOT repaired
    /// here. Those are produced by manifest and API parsers with their own
    /// escaping rules, so a repair at this boundary would double up wherever
    /// a producer already decoded, on the one path where a wrong URL is a
    /// failed download rather than a blemish. Where a producer genuinely does
    /// not decode — `MovieFap`'s regex-read XML — the repair is applied there,
    /// at the point that producer's encoding is known.
    ///
    /// `formats[].format_note` is also left alone: it is synthesized in-tree
    /// from resolution and codec data rather than scraped, so it carries no
    /// entities. That is an argument about provenance, not about the field
    /// being non-display — if a plugin ever sets it from page text, it
    /// belongs in the list above.
    pub fn decode_text_fields(&mut self) {
        self.webpage_url = crate::repair_url_entities(&self.webpage_url);
        self.thumbnail = self.thumbnail.as_deref().map(crate::repair_url_entities);
        self.uploader_url = self.uploader_url.as_deref().map(crate::repair_url_entities);
        self.channel_url = self.channel_url.as_deref().map(crate::repair_url_entities);
        if let Some(thumbnails) = self.thumbnails.as_mut() {
            for thumbnail in thumbnails {
                thumbnail.url = crate::repair_url_entities(&thumbnail.url);
            }
        }
        self.title = crate::decode_html_entities(&self.title);
        self.description = self.description.as_deref().map(crate::decode_html_entities);
        self.uploader = self.uploader.as_deref().map(crate::decode_html_entities);
        self.channel = self.channel.as_deref().map(crate::decode_html_entities);
        self.playlist = self.playlist.as_deref().map(crate::decode_html_entities);
        self.playlist_title = self
            .playlist_title
            .as_deref()
            .map(crate::decode_html_entities);
        self.artist = self.artist.as_deref().map(crate::decode_html_entities);
        self.album = self.album.as_deref().map(crate::decode_html_entities);
        self.track = self.track.as_deref().map(crate::decode_html_entities);
        crate::decode_each(&mut self.actors);
        if let Some(tags) = self.tags.as_mut() {
            crate::decode_each(tags);
        }
        if let Some(categories) = self.categories.as_mut() {
            crate::decode_each(categories);
        }
        // Nested display strings. `chapters[].title` is the one that reaches
        // a file: `MetadataStage::build_chapters` writes it into the output
        // container, so an entity there lands in the artifact — the same
        // failure #698 is about, one level down. No in-tree extractor
        // populates chapters today, but a plugin can.
        if let Some(chapters) = self.chapters.as_mut() {
            for chapter in chapters {
                chapter.title = crate::decode_html_entities(&chapter.title);
            }
        }
        for track in self
            .subtitles
            .iter_mut()
            .chain(self.automatic_captions.iter_mut())
            .flat_map(|m| m.values_mut())
            .flatten()
        {
            // `name` is display text and gets the decoder; `url` gets the
            // narrower URL repair, for the same reason every other URL field
            // does. `ext` is neither.
            track.name = track.name.as_deref().map(crate::decode_html_entities);
            track.url = crate::repair_url_entities(&track.url);
        }
    }

    /// Create a new `InfoDict` with required fields
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
            actors: Vec::new(),
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

#[cfg(test)]
mod decode_text_fields_tests {
    use super::InfoDict;

    /// The filename from #698, verbatim off the filesystem.
    #[test]
    fn decodes_the_title_that_reached_a_real_filename() {
        let mut info = InfoDict::new(
            "2914100",
            "&quot;PLEASE, JUST DON&#039;T TELL MY PARENTS!&quot; My stepsister",
            "pornoxo",
            "https://example.com/v",
        );
        info.decode_text_fields();
        assert_eq!(
            info.title,
            "\"PLEASE, JUST DON'T TELL MY PARENTS!\" My stepsister"
        );
    }

    /// `uploader` and `playlist_title` become DIRECTORY components under the
    /// default output template, so the same defect lands in a folder name.
    #[test]
    fn decodes_the_fields_that_become_directories() {
        let mut info = InfoDict::new("id", "t", "e", "https://example.com/v");
        info.uploader = Some("Tom &amp; Jerry&#039;s Studio".to_string());
        info.playlist_title = Some("Season &#8211; One".to_string());
        info.decode_text_fields();
        assert_eq!(info.uploader.as_deref(), Some("Tom & Jerry's Studio"));
        assert_eq!(info.playlist_title.as_deref(), Some("Season \u{2013} One"));
    }

    /// The six fields the other tests do not reach. Without this, deleting
    /// any of their lines from `decode_text_fields` leaves the suite green —
    /// and `description` and `channel` are both operator-visible and
    /// addressable from the output template.
    #[test]
    fn decodes_every_remaining_display_field() {
        let mut info = InfoDict::new("id", "t", "e", "https://example.com/v");
        info.description = Some("A &amp; B".to_string());
        info.channel = Some("Chan &#039;n Co".to_string());
        info.playlist = Some("List &amp; More".to_string());
        info.artist = Some("Artist &amp; Co".to_string());
        info.album = Some("Album &#8211; One".to_string());
        info.track = Some("Track &quot;X&quot;".to_string());
        info.decode_text_fields();
        assert_eq!(info.description.as_deref(), Some("A & B"));
        assert_eq!(info.channel.as_deref(), Some("Chan 'n Co"));
        assert_eq!(info.playlist.as_deref(), Some("List & More"));
        assert_eq!(info.artist.as_deref(), Some("Artist & Co"));
        assert_eq!(info.album.as_deref(), Some("Album \u{2013} One"));
        assert_eq!(info.track.as_deref(), Some("Track \"X\""));
    }

    /// Lists of display text decode too.
    #[test]
    fn decodes_actors_tags_and_categories() {
        let mut info = InfoDict::new("id", "t", "e", "https://example.com/v");
        info.actors = vec!["A &amp; B".to_string()];
        info.tags = Some(vec!["rock &amp; roll".to_string()]);
        info.categories = Some(vec!["mom &#039;n pop".to_string()]);
        info.decode_text_fields();
        assert_eq!(info.actors, vec!["A & B".to_string()]);
        assert_eq!(info.tags.as_deref(), Some(&["rock & roll".to_string()][..]));
        assert_eq!(
            info.categories.as_deref(),
            Some(&["mom 'n pop".to_string()][..])
        );
    }

    /// Nested display strings decode too. `chapters[].title` is written into
    /// the output container by the metadata stage, so an entity there reaches
    /// the artifact on disk.
    #[test]
    fn decodes_nested_display_strings() {
        use super::{Chapter, Subtitle};
        use std::collections::HashMap;

        let mut info = InfoDict::new("id", "t", "e", "https://example.com/v");
        info.chapters = Some(vec![Chapter {
            title: "Intro &amp; Credits".to_string(),
            start_time: 0.0,
            end_time: 1.0,
        }]);
        let mut subs = HashMap::new();
        subs.insert(
            "en".to_string(),
            vec![Subtitle {
                url: "https://x.test/s.vtt?a=1&amp;b=2".to_string(),
                ext: "vtt".to_string(),
                name: Some("English &#039;full&#039;".to_string()),
            }],
        );
        info.subtitles = Some(subs);

        info.decode_text_fields();

        let chapter = info
            .chapters
            .as_ref()
            .and_then(|c| c.first())
            .expect("chapter present");
        assert_eq!(chapter.title, "Intro & Credits");
        let track = info
            .subtitles
            .as_ref()
            .and_then(|m| m.get("en"))
            .and_then(|v| v.first())
            .expect("track present");
        assert_eq!(track.name.as_deref(), Some("English 'full'"));
        // the track URL is not display text: it gets the narrow repair, so the
        // separator is a real `&` and never a decoded `&sol;`
        assert_eq!(track.url, "https://x.test/s.vtt?a=1&b=2");
    }

    /// Ids are NOT touched at all: an id is an opaque site token rather than
    /// text a person reads.
    #[test]
    fn leaves_ids_alone() {
        let mut info = InfoDict::new("a&amp;b", "t", "e", "https://x.test/v");
        info.uploader_id = Some("u&amp;v".to_string());
        info.channel_id = Some("c&amp;d".to_string());
        info.decode_text_fields();
        assert_eq!(info.id, "a&amp;b");
        assert_eq!(info.uploader_id.as_deref(), Some("u&amp;v"));
        assert_eq!(info.channel_id.as_deref(), Some("c&amp;d"));
    }

    /// URLs get the narrow repair, not the display decoder. Every scraped URL
    /// carries `&amp;` because it was read out of an HTML attribute, and
    /// before this ran here each extractor had to remember — which is the
    /// failure #698 is about, applied to the fields that boundary skipped.
    #[test]
    fn repairs_the_ampersand_in_every_url_field() {
        let mut info = InfoDict::new("id", "t", "e", "https://x.test/?a=1&amp;b=2");
        info.thumbnail = Some("https://x.test/t.jpg?a=1&amp;b=2".to_string());
        info.uploader_url = Some("https://x.test/u?a=1&amp;b=2".to_string());
        info.channel_url = Some("https://x.test/c?a=1&amp;b=2".to_string());
        info.decode_text_fields();
        assert_eq!(info.webpage_url, "https://x.test/?a=1&b=2");
        assert_eq!(
            info.thumbnail.as_deref(),
            Some("https://x.test/t.jpg?a=1&b=2")
        );
        assert_eq!(
            info.uploader_url.as_deref(),
            Some("https://x.test/u?a=1&b=2")
        );
        assert_eq!(
            info.channel_url.as_deref(),
            Some("https://x.test/c?a=1&b=2")
        );
    }

    /// The repair is NOT the display decoder, and the difference is the point:
    /// `&sol;` would invent a path separator and `&lt;` a character a URL may
    /// not carry unencoded. Only `&` is touched.
    #[test]
    fn does_not_run_the_display_decoder_over_a_url() {
        let mut info = InfoDict::new("id", "t", "e", "https://x.test/?p=a&sol;b&amp;q=&lt;");
        info.decode_text_fields();
        assert_eq!(info.webpage_url, "https://x.test/?p=a&sol;b&q=&lt;");
    }

    /// Running the boundary over already-decoded text is a no-op, which is
    /// what makes it safe to apply unconditionally to every extractor's
    /// output — including the majority whose titles the HTML parser already
    /// decoded.
    #[test]
    fn is_a_no_op_for_text_with_nothing_to_decode() {
        let mut info = InfoDict::new("id", "Tom & Jerry: 100% fun", "e", "https://example.com/v");
        info.decode_text_fields();
        assert_eq!(info.title, "Tom & Jerry: 100% fun");
    }
}
