//! MetadataStage — embeds metadata (title, artist, chapters, etc.) into media files.
//!
//! This stage runs at index 6 when `config.embed_metadata` is true.
//! Non-fatal: metadata failure logs a warning and passes through.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use log::info;

use rdlp_core::InfoDict;
use rdlp_ffmpeg::{ChapterEntry, FFmpegRunner};

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Embeds metadata (title, artist, chapters, etc.) into the primary current file.
///
/// `should_run` triggers when `config.embed_metadata` is true.
/// Non-fatal: failures push a warning and pass through unchanged.
pub struct MetadataStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl MetadataStage {
    /// Create a new `MetadataStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }

    /// Build a metadata `HashMap` from `InfoDict`.
    pub(crate) fn build_metadata(info: &InfoDict) -> HashMap<String, String> {
        let mut meta = HashMap::with_capacity(10);

        meta.insert("title".to_string(), info.title.clone());

        if let Some(ref artist) = info.artist {
            meta.insert("artist".to_string(), artist.clone());
        } else if let Some(ref uploader) = info.uploader {
            meta.insert("artist".to_string(), uploader.clone());
        }

        if let Some(ref album) = info.album {
            meta.insert("album".to_string(), album.clone());
        }

        if let Some(ref track) = info.track {
            meta.insert("track".to_string(), track.clone());
        }

        if let Some(ref date) = info.upload_date {
            let formatted = if date.len() == 8 {
                format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
            } else {
                date.clone()
            };
            meta.insert("date".to_string(), formatted);
        }

        if let Some(year) = info.release_year {
            meta.insert("year".to_string(), year.to_string());
        }

        if let Some(ref description) = info.description {
            let desc = if description.len() > 1000 {
                let truncated = match description.char_indices().nth(997) {
                    Some((byte_idx, _)) => &description[..byte_idx],
                    None => description,
                };
                format!("{truncated}...")
            } else {
                description.clone()
            };
            meta.insert("comment".to_string(), desc.clone());
            meta.insert("description".to_string(), desc);
        }

        meta.insert("purl".to_string(), info.webpage_url.clone());
        meta.insert(
            "encoder".to_string(),
            format!("rdlp via {}", info.extractor),
        );

        meta
    }

    /// Build chapter entries from `InfoDict`.
    pub(crate) fn build_chapters(info: &InfoDict) -> Vec<ChapterEntry> {
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
impl PipelineStage for MetadataStage {
    fn name(&self) -> &str {
        "MetadataStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.embed_metadata
    }

    fn is_fatal(&self) -> bool {
        false
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        let input_file = msg.tracker.primary();
        let ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4")
            .to_string();

        info!(
            "MetadataStage: embedding metadata into {}",
            input_file.display()
        );

        let temp_output = msg.tracker.temp_path(&input_file, &ext);

        let metadata = Self::build_metadata(&msg.info);
        let chapters = Self::build_chapters(&msg.info);

        let log_callback = if msg.config.verbose {
            msg.callback_factory.as_ref().map(|f| f(self.name()))
        } else {
            None
        };

        match self
            .ffmpeg
            .embed_metadata(
                &input_file,
                &temp_output,
                &metadata,
                &chapters,
                log_callback,
            )
            .await
        {
            Ok(()) => {
                info!(
                    "MetadataStage: metadata embedded into {}",
                    input_file.display()
                );
                msg.tracker.replace(vec![temp_output]);
            }
            Err(e) => {
                log::warn!("MetadataStage: failed to embed metadata: {e}");
                msg.warnings.push(format!("Metadata embedding failed: {e}"));
                msg.tracker.mark_temp(temp_output);
            }
        }

        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    use rdlp_core::{InfoDict, PostProcessConfig};

    use crate::pipeline::{FileTracker, PipelineError, TempRegistry};

    fn make_msg(files: Vec<PathBuf>, config: PostProcessConfig) -> PipelineMessage {
        let reg = Arc::new(TempRegistry::new());
        let (error_tx, _) = oneshot::channel::<PipelineError>();
        PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test Video".to_string(),
                "TestExtractor".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(files, reg),
            config: Arc::new(config),
            original_stem: "test".to_string(),
            is_hls: false,
            callback_factory: None,
            error_tx: Some(error_tx),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn should_run_when_embed_metadata() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MetadataStage::new(ffmpeg);

        let config = PostProcessConfig {
            embed_metadata: true,
            ..PostProcessConfig::default()
        };
        let msg = make_msg(vec![PathBuf::from("/tmp/video.mp4")], config);
        assert!(stage.should_run(&msg));
    }

    #[test]
    fn should_not_run_by_default() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MetadataStage::new(ffmpeg);
        let msg = make_msg(
            vec![PathBuf::from("/tmp/video.mp4")],
            PostProcessConfig::default(),
        );
        assert!(!stage.should_run(&msg));
    }

    #[test]
    fn is_not_fatal() {
        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = MetadataStage::new(ffmpeg);
        assert!(!stage.is_fatal());
    }

    #[test]
    fn build_metadata_basic() {
        let mut info = InfoDict::new(
            "test123".to_string(),
            "Test Video".to_string(),
            "TestExtractor".to_string(),
            "https://example.com/video".to_string(),
        );
        info.artist = Some("Test Artist".to_string());
        info.upload_date = Some("20240115".to_string());

        let meta = MetadataStage::build_metadata(&info);

        assert_eq!(meta.get("title").unwrap(), "Test Video");
        assert_eq!(meta.get("artist").unwrap(), "Test Artist");
        assert_eq!(meta.get("date").unwrap(), "2024-01-15");
        assert_eq!(meta.get("purl").unwrap(), "https://example.com/video");
        assert!(meta.get("encoder").unwrap().contains("TestExtractor"));
    }

    #[test]
    fn build_metadata_uploader_fallback() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        info.uploader = Some("Uploader Name".to_string());

        let meta = MetadataStage::build_metadata(&info);
        assert_eq!(meta.get("artist").unwrap(), "Uploader Name");
    }

    #[test]
    fn build_metadata_description_truncation() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        info.description = Some("a".repeat(2000));

        let meta = MetadataStage::build_metadata(&info);
        let desc = meta.get("description").unwrap();
        assert!(desc.len() <= 1003);
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn build_metadata_description_utf8_safe() {
        let mut info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        info.description = Some("日".repeat(500));

        let meta = MetadataStage::build_metadata(&info);
        let desc = meta.get("description").unwrap();
        assert!(desc.ends_with("..."));
        let _ = desc.chars().count();
    }

    #[test]
    fn build_chapters_basic() {
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

        let chapters = MetadataStage::build_chapters(&info);
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].id, 0);
        assert_eq!(chapters[0].start_ms, 0);
        assert_eq!(chapters[0].end_ms, 30000);
        assert_eq!(chapters[0].title, "Intro");
        assert_eq!(chapters[1].start_ms, 30000);
        assert_eq!(chapters[1].end_ms, 120000);
    }

    #[test]
    fn build_chapters_none() {
        let info = InfoDict::new(
            "test".to_string(),
            "Test".to_string(),
            "Test".to_string(),
            "https://example.com".to_string(),
        );
        let chapters = MetadataStage::build_chapters(&info);
        assert!(chapters.is_empty());
    }
}
