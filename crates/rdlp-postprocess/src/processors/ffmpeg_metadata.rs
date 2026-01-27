//! FFmpeg metadata embedding post-processor.
//!
//! Embeds metadata (title, artist, album, etc.) and chapters into media files
//! using `ffmpeg-the-third` library bindings (no CLI process spawning).
//! No temporary FFMETADATA1 file is needed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::info;
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::ffmpeg::{ChapterEntry, FFmpegRunner};

/// Post-processor that embeds metadata into media files.
///
/// # Priority
/// This processor has priority 30 (runs after merging and conversion).
///
/// # When it runs
/// - When `embed_metadata` is true in config
///
/// # Supported metadata
/// - title, artist, album, date, description
/// - genre, track, comment
/// - Chapters (if available in InfoDict)
pub struct FFmpegMetadata {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FFmpegMetadata {
    /// Create a new metadata embedding processor.
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build a metadata `HashMap` from `InfoDict`.
    fn build_metadata(info: &InfoDict) -> HashMap<String, String> {
        let mut meta = HashMap::new();

        // Title
        meta.insert("title".to_string(), info.title.clone());

        // Artist/Uploader
        if let Some(ref artist) = info.artist {
            meta.insert("artist".to_string(), artist.clone());
        } else if let Some(ref uploader) = info.uploader {
            meta.insert("artist".to_string(), uploader.clone());
        }

        // Album
        if let Some(ref album) = info.album {
            meta.insert("album".to_string(), album.clone());
        }

        // Track
        if let Some(ref track) = info.track {
            meta.insert("track".to_string(), track.clone());
        }

        // Date
        if let Some(ref date) = info.upload_date {
            // Convert YYYYMMDD to YYYY-MM-DD
            let formatted = if date.len() == 8 {
                format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
            } else {
                date.clone()
            };
            meta.insert("date".to_string(), formatted);
        }

        // Year
        if let Some(year) = info.release_year {
            meta.insert("year".to_string(), year.to_string());
        }

        // Description/Comment — truncate safely at UTF-8 char boundary
        if let Some(ref description) = info.description {
            let desc = if description.len() > 1000 {
                let truncated = match description.char_indices().nth(997) {
                    Some((byte_idx, _)) => &description[..byte_idx],
                    None => description, // fewer than 997 chars, no truncation needed
                };
                format!("{truncated}...")
            } else {
                description.clone()
            };
            meta.insert("comment".to_string(), desc.clone());
            meta.insert("description".to_string(), desc);
        }

        // Webpage URL
        meta.insert("purl".to_string(), info.webpage_url.clone());

        // Extractor
        meta.insert(
            "encoder".to_string(),
            format!("rdlp via {}", info.extractor),
        );

        meta
    }

    /// Build chapter entries from `InfoDict`.
    fn build_chapters(info: &InfoDict) -> Vec<ChapterEntry> {
        let Some(chapters) = info.chapters.as_ref() else {
            return Vec::new();
        };

        chapters
            .iter()
            .enumerate()
            .map(|(i, ch)| ChapterEntry {
                id: i as i64,
                start_ms: (ch.start_time * 1000.0) as i64,
                end_ms: (ch.end_time * 1000.0) as i64,
                title: ch.title.clone(),
            })
            .collect()
    }
}

#[async_trait]
impl PostProcessor for FFmpegMetadata {
    fn name(&self) -> &str {
        "FFmpegMetadata"
    }

    fn priority(&self) -> i32 {
        30 // After merging and conversion
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.embed_metadata
    }

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        _config: &PostProcessConfig,
    ) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let input_file = &files[0];
        let extension = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4");

        info!(file:? = input_file.display(); "Embedding metadata");

        // Create temp output file
        let temp_output = input_file.with_extension(format!("temp.{extension}"));

        // Build metadata and chapters
        let metadata = Self::build_metadata(info);
        let chapters = Self::build_chapters(info);

        // Embed via library bindings (stream copy + metadata + chapters)
        self.ffmpeg
            .embed_metadata(input_file, &temp_output, &metadata, &chapters)
            .await?;

        // Replace original with temp
        tokio::fs::rename(&temp_output, input_file).await?;

        info!(file:? = input_file.display(); "Metadata embedded");

        Ok(PostProcessResult::new(info.clone(), files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_metadata() {
        let mut info = InfoDict::new(
            "test123".to_string(),
            "Test Video".to_string(),
            "TestExtractor".to_string(),
            "https://example.com/video".to_string(),
        );
        info.artist = Some("Test Artist".to_string());
        info.upload_date = Some("20240115".to_string());

        let meta = FFmpegMetadata::build_metadata(&info);

        assert_eq!(meta.get("title").unwrap(), "Test Video");
        assert_eq!(meta.get("artist").unwrap(), "Test Artist");
        assert_eq!(meta.get("date").unwrap(), "2024-01-15");
        assert_eq!(meta.get("purl").unwrap(), "https://example.com/video");
        assert!(meta.get("encoder").unwrap().contains("TestExtractor"));
    }

    #[test]
    fn test_build_metadata_description_truncation() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        // Create a long description (> 1000 chars)
        info.description = Some("a".repeat(2000));

        let meta = FFmpegMetadata::build_metadata(&info);
        let desc = meta.get("description").unwrap();
        assert!(desc.len() <= 1003); // 997 chars + "..."
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn test_build_metadata_description_utf8_safe() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        // Use multi-byte UTF-8 characters (each '日' is 3 bytes)
        info.description = Some("日".repeat(500)); // 1500 bytes, 500 chars

        let meta = FFmpegMetadata::build_metadata(&info);
        let desc = meta.get("description").unwrap();
        // Should truncate at char boundary, not byte boundary
        assert!(desc.ends_with("..."));
        // Verify it's valid UTF-8 (would panic if not)
        let _ = desc.chars().count();
    }

    #[test]
    fn test_build_metadata_uploader_fallback() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        info.uploader = Some("Uploader Name".to_string());

        let meta = FFmpegMetadata::build_metadata(&info);
        assert_eq!(meta.get("artist").unwrap(), "Uploader Name");
    }

    #[test]
    fn test_build_chapters() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        info.chapters = Some(vec![
            rdlp_core::Chapter {
                title: "Intro".to_string(),
                start_time: 0.0,
                end_time: 30.0,
            },
            rdlp_core::Chapter {
                title: "Main".to_string(),
                start_time: 30.0,
                end_time: 120.0,
            },
        ]);

        let chapters = FFmpegMetadata::build_chapters(&info);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].id, 0);
        assert_eq!(chapters[0].start_ms, 0);
        assert_eq!(chapters[0].end_ms, 30000);
        assert_eq!(chapters[0].title, "Intro");
        assert_eq!(chapters[1].id, 1);
        assert_eq!(chapters[1].start_ms, 30000);
        assert_eq!(chapters[1].end_ms, 120000);
        assert_eq!(chapters[1].title, "Main");
    }

    #[test]
    fn test_build_chapters_none() {
        let info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );

        let chapters = FFmpegMetadata::build_chapters(&info);
        assert!(chapters.is_empty());
    }
}
