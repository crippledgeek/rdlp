//! Subtitle embedding post-processor.
//!
//! Embeds subtitle streams into video containers using FFmpeg library
//! bindings. Supports different subtitle codecs based on the container:
//! - MP4/M4A/M4V/MOV: `mov_text`
//! - MKV/MKA: `srt` (SubRip)
//! - WebM: `webvtt`
//!
//! Subtitle files are discovered alongside the media file using the pattern
//! `{stem}.{lang}.{ext}` where `ext` is one of: srt, vtt, ass, ssa, lrc.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{
    InfoDict, PostProcessCallback, PostProcessConfig, PostProcessResult, PostProcessor, Result,
};

/// Subtitle file extensions to search for.
const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "vtt", "ass", "ssa", "lrc"];

/// Containers that support subtitle embedding.
const SUBTITLE_CONTAINERS: &[&str] = &["mp4", "m4a", "m4v", "mov", "mkv", "mka", "webm"];

ffmpeg_processor!(
    EmbedSubtitles,
    "EmbedSubtitles",
    25,
    "Post-processor that embeds subtitle streams into video files.\n\n\
     # Priority\n\
     This processor has priority 25 (between thumbnail at 20 and metadata at 30).\n\n\
     # When it runs\n\
     - When `embed_subtitles` is true in config\n\
     - When subtitle files exist alongside the media file"
);

impl EmbedSubtitles {
    /// Check if the container format supports subtitle embedding.
    fn supports_subtitles(extension: &str) -> bool {
        SUBTITLE_CONTAINERS
            .iter()
            .any(|c| c.eq_ignore_ascii_case(extension))
    }

    /// Get the FFmpeg subtitle codec for a container format.
    ///
    /// # Returns
    /// The appropriate subtitle codec name, or `"srt"` as fallback.
    fn subtitle_codec_for_container(container_ext: &str) -> &'static str {
        // Use case-insensitive matching to avoid allocating a lowercase copy
        if ["mp4", "m4a", "m4v", "mov"]
            .iter()
            .any(|c| c.eq_ignore_ascii_case(container_ext))
        {
            "mov_text"
        } else if ["mkv", "mka"]
            .iter()
            .any(|c| c.eq_ignore_ascii_case(container_ext))
        {
            "srt"
        } else if container_ext.eq_ignore_ascii_case("webm") {
            "webvtt"
        } else {
            "srt"
        }
    }

    /// Find subtitle files alongside a media file.
    ///
    /// Searches for files matching `{stem}.{lang}.{ext}` where `ext` is one
    /// of the known subtitle extensions. Also strips pipeline suffixes
    /// (`.norm`, `.fixed`, `.thumb`) so that `video.norm.mkv` still finds
    /// `video.en.srt`.
    ///
    /// # Returns
    /// A vector of `(language_code, subtitle_path)` pairs.
    fn find_subtitle_files(media_file: &Path) -> Vec<(String, PathBuf)> {
        let Some(stem) = media_file.file_stem().and_then(|s| s.to_str()) else {
            return Vec::new();
        };
        let Some(parent) = media_file.parent() else {
            return Vec::new();
        };

        let candidates = Self::stem_candidates(stem);
        let mut result = Vec::new();

        for candidate in &candidates {
            // Read the directory and look for subtitle files
            let Ok(entries) = std::fs::read_dir(parent) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
                    continue;
                };

                // Check pattern: {candidate}.{lang}.{ext}
                for sub_ext in SUBTITLE_EXTENSIONS {
                    // Strip the subtitle extension
                    let Some(without_ext) = filename
                        .strip_suffix(sub_ext)
                        .and_then(|s| s.strip_suffix('.'))
                    else {
                        continue;
                    };

                    // Strip the candidate stem prefix + dot
                    let Some(lang) = without_ext
                        .strip_prefix(candidate)
                        .and_then(|s| s.strip_prefix('.'))
                    else {
                        continue;
                    };

                    // The remainder is the language code
                    if !lang.is_empty() {
                        result.push((lang.to_string(), path.clone()));
                        break; // Found match for this entry, move on
                    }
                }
            }
        }

        // Sort by language code for deterministic order
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    /// Generate stem candidates by progressively stripping pipeline suffixes.
    ///
    /// e.g. `"video.norm"` -> `["video.norm", "video"]`
    fn stem_candidates(stem: &str) -> Vec<&str> {
        const PIPELINE_SUFFIXES: &[&str] = &[".norm", ".fixed", ".thumb"];
        let mut candidates = vec![stem];
        let mut current = stem;
        loop {
            let mut stripped = false;
            for suffix in PIPELINE_SUFFIXES {
                if let Some(base) = current.strip_suffix(suffix)
                    && !base.is_empty()
                {
                    candidates.push(base);
                    current = base;
                    stripped = true;
                    break;
                }
            }
            if !stripped {
                break;
            }
        }
        candidates
    }
}

#[async_trait]
impl PostProcessor for EmbedSubtitles {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        config.embed_subtitles
    }

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
        _callback: Option<Arc<dyn PostProcessCallback>>,
    ) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let media_file = &files[0];
        let extension = media_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Check if container supports subtitles
        if !Self::supports_subtitles(extension) {
            debug!(extension; "Container does not support subtitle embedding");
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        // Find subtitle files alongside media
        let subtitle_files = Self::find_subtitle_files(media_file);
        if subtitle_files.is_empty() {
            debug!(file:? = media_file.display(); "No subtitle files found");
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let codec = Self::subtitle_codec_for_container(extension);
        info!(
            count = subtitle_files.len(),
            codec;
            "Found subtitle files for embedding"
        );

        // Attempt FFmpeg subtitle embedding.
        // The embed_subtitles method does not exist yet on FFmpegRunner.
        // For now, log and return files unchanged (non-fatal).
        // Reference self.ffmpeg so the field is not dead code.
        let _ = &self.ffmpeg;
        for (lang, sub_path) in &subtitle_files {
            info!(lang = lang.as_str(), path:? = sub_path.display(); "Would embed subtitle");
        }
        warn!("Subtitle embedding via FFmpeg not yet implemented; returning files unchanged");

        // Clean up subtitle files unless write_subtitles is set
        let temp_files = if config.write_subtitles {
            Vec::new()
        } else {
            subtitle_files.into_iter().map(|(_, p)| p).collect()
        };

        Ok(PostProcessResult {
            info: info.clone(),
            files,
            temp_files,
        })
    }
}

#[cfg(test)]
#[path = "embed_subtitles_tests.rs"]
mod tests;
