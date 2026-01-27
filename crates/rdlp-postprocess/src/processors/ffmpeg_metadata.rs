//! FFmpeg metadata embedding post-processor.
//!
//! Embeds metadata (title, artist, album, etc.) and chapters into media files
//! using FFmpeg's metadata capabilities.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use crate::ffmpeg::FFmpegRunner;

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

    /// Build FFmpeg metadata arguments from InfoDict.
    fn build_metadata_args(&self, info: &InfoDict) -> Vec<String> {
        let mut args = Vec::new();

        // Title
        args.push("-metadata".to_string());
        args.push(format!("title={}", info.title));

        // Artist/Uploader
        if let Some(ref artist) = info.artist {
            args.push("-metadata".to_string());
            args.push(format!("artist={artist}"));
        } else if let Some(ref uploader) = info.uploader {
            args.push("-metadata".to_string());
            args.push(format!("artist={uploader}"));
        }

        // Album
        if let Some(ref album) = info.album {
            args.push("-metadata".to_string());
            args.push(format!("album={album}"));
        }

        // Track
        if let Some(ref track) = info.track {
            args.push("-metadata".to_string());
            args.push(format!("track={track}"));
        }

        // Date
        if let Some(ref date) = info.upload_date {
            // Convert YYYYMMDD to YYYY-MM-DD
            let formatted = if date.len() == 8 {
                format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
            } else {
                date.clone()
            };
            args.push("-metadata".to_string());
            args.push(format!("date={formatted}"));
        }

        // Year
        if let Some(year) = info.release_year {
            args.push("-metadata".to_string());
            args.push(format!("year={year}"));
        }

        // Description/Comment
        if let Some(ref description) = info.description {
            // Truncate very long descriptions
            let desc = if description.len() > 1000 {
                format!("{}...", &description[..997])
            } else {
                description.clone()
            };
            args.push("-metadata".to_string());
            args.push(format!("comment={desc}"));

            args.push("-metadata".to_string());
            args.push(format!("description={desc}"));
        }

        // Webpage URL
        args.push("-metadata".to_string());
        args.push(format!("purl={}", info.webpage_url));

        // Extractor
        args.push("-metadata".to_string());
        args.push(format!("encoder=rdlp via {}", info.extractor));

        args
    }

    /// Generate FFMETADATA1 format file content for chapters.
    fn generate_chapters_metadata(&self, info: &InfoDict) -> Option<String> {
        let chapters = info.chapters.as_ref()?;
        if chapters.is_empty() {
            return None;
        }

        let mut content = String::from(";FFMETADATA1\n");

        for chapter in chapters {
            content.push_str("\n[CHAPTER]\n");
            content.push_str("TIMEBASE=1/1000\n");
            content.push_str(&format!("START={}\n", (chapter.start_time * 1000.0) as i64));
            content.push_str(&format!("END={}\n", (chapter.end_time * 1000.0) as i64));
            content.push_str(&format!("title={}\n", chapter.title));
        }

        Some(content)
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

        // Build base arguments
        let mut args = vec!["-i".to_string(), input_file.to_string_lossy().to_string()];

        // Handle chapters if present
        let mut temp_metadata_file = None;
        if let Some(chapters_content) = self.generate_chapters_metadata(info) {
            debug!("Generating chapter metadata file");

            // Write chapters to temp file
            let chapters_path = input_file.with_extension("ffmetadata");
            let mut file = std::fs::File::create(&chapters_path)?;
            file.write_all(chapters_content.as_bytes())?;

            args.push("-i".to_string());
            args.push(chapters_path.to_string_lossy().to_string());
            args.push("-map_metadata".to_string());
            args.push("1".to_string());

            temp_metadata_file = Some(chapters_path);
        }

        // Add metadata arguments
        let metadata_args = self.build_metadata_args(info);
        args.extend(metadata_args);

        // Copy streams
        args.push("-c".to_string());
        args.push("copy".to_string());

        // Output
        args.push(temp_output.to_string_lossy().to_string());

        // Run FFmpeg
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.ffmpeg.run(&args_refs).await?;

        // Replace original with temp
        tokio::fs::rename(&temp_output, input_file).await?;

        // Cleanup chapter metadata file
        if let Some(chapters_file) = temp_metadata_file {
            let _ = tokio::fs::remove_file(&chapters_file).await;
        }

        info!(file:? = input_file.display(); "Metadata embedded");

        Ok(PostProcessResult::new(info.clone(), files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_metadata_args() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let processor = FFmpegMetadata::new(Arc::new(ffmpeg));

            let mut info = InfoDict::new(
                "test123".to_string(),
                "Test Video".to_string(),
                "TestExtractor".to_string(),
                "https://example.com/video".to_string(),
            );
            info.artist = Some("Test Artist".to_string());
            info.upload_date = Some("20240115".to_string());

            let args = processor.build_metadata_args(&info);

            assert!(args.contains(&"-metadata".to_string()));
            assert!(args.iter().any(|a| a.starts_with("title=")));
            assert!(args.iter().any(|a| a.starts_with("artist=")));
            assert!(args.iter().any(|a| a.starts_with("date=")));
        }
    }

    #[test]
    fn test_generate_chapters_metadata() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let processor = FFmpegMetadata::new(Arc::new(ffmpeg));

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

            let metadata = processor.generate_chapters_metadata(&info);
            assert!(metadata.is_some());

            let content = metadata.unwrap();
            assert!(content.contains(";FFMETADATA1"));
            assert!(content.contains("[CHAPTER]"));
            assert!(content.contains("title=Intro"));
            assert!(content.contains("title=Main"));
        }
    }
}
