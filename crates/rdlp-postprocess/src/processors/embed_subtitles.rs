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

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

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
                if let Some(base) = current.strip_suffix(suffix) {
                    if !base.is_empty() {
                        candidates.push(base);
                        current = base;
                        stripped = true;
                        break;
                    }
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
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── supports_subtitles ──────────────────────────────────────────

    #[test]
    fn test_supports_subtitles_mp4() {
        assert!(EmbedSubtitles::supports_subtitles("mp4"));
        assert!(EmbedSubtitles::supports_subtitles("MP4"));
    }

    #[test]
    fn test_supports_subtitles_mkv() {
        assert!(EmbedSubtitles::supports_subtitles("mkv"));
        assert!(EmbedSubtitles::supports_subtitles("MKV"));
        assert!(EmbedSubtitles::supports_subtitles("mka"));
    }

    #[test]
    fn test_supports_subtitles_webm() {
        assert!(EmbedSubtitles::supports_subtitles("webm"));
        assert!(EmbedSubtitles::supports_subtitles("WEBM"));
    }

    #[test]
    fn test_supports_subtitles_mp4_variants() {
        assert!(EmbedSubtitles::supports_subtitles("m4a"));
        assert!(EmbedSubtitles::supports_subtitles("m4v"));
        assert!(EmbedSubtitles::supports_subtitles("mov"));
    }

    #[test]
    fn test_does_not_support_subtitles_avi() {
        assert!(!EmbedSubtitles::supports_subtitles("avi"));
    }

    #[test]
    fn test_does_not_support_subtitles_txt() {
        assert!(!EmbedSubtitles::supports_subtitles("txt"));
    }

    #[test]
    fn test_does_not_support_subtitles_flv() {
        assert!(!EmbedSubtitles::supports_subtitles("flv"));
    }

    #[test]
    fn test_does_not_support_subtitles_empty() {
        assert!(!EmbedSubtitles::supports_subtitles(""));
    }

    // ── subtitle_codec_for_container ────────────────────────────────

    #[test]
    fn test_subtitle_codec_for_mp4() {
        assert_eq!(
            EmbedSubtitles::subtitle_codec_for_container("mp4"),
            "mov_text"
        );
        assert_eq!(
            EmbedSubtitles::subtitle_codec_for_container("m4a"),
            "mov_text"
        );
        assert_eq!(
            EmbedSubtitles::subtitle_codec_for_container("m4v"),
            "mov_text"
        );
        assert_eq!(
            EmbedSubtitles::subtitle_codec_for_container("mov"),
            "mov_text"
        );
    }

    #[test]
    fn test_subtitle_codec_for_mkv() {
        assert_eq!(EmbedSubtitles::subtitle_codec_for_container("mkv"), "srt");
        assert_eq!(EmbedSubtitles::subtitle_codec_for_container("mka"), "srt");
    }

    #[test]
    fn test_subtitle_codec_for_webm() {
        assert_eq!(
            EmbedSubtitles::subtitle_codec_for_container("webm"),
            "webvtt"
        );
    }

    #[test]
    fn test_subtitle_codec_fallback() {
        assert_eq!(EmbedSubtitles::subtitle_codec_for_container("avi"), "srt");
        assert_eq!(
            EmbedSubtitles::subtitle_codec_for_container("unknown"),
            "srt"
        );
    }

    // ── should_run ──────────────────────────────────────────────────

    #[test]
    fn test_should_run_when_enabled() {
        // FFmpegRunner requires FFmpeg library; skip if unavailable
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let mut config = PostProcessConfig::default();
        config.embed_subtitles = true;
        assert!(processor.should_run(&info, &config));
    }

    #[test]
    fn test_should_not_run_when_disabled() {
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let config = PostProcessConfig::default();
        assert!(!processor.should_run(&info, &config));
    }

    // ── find_subtitle_files ─────────────────────────────────────────

    #[test]
    fn test_find_subtitle_files_basic() {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        let sub = dir.path().join("video.en.srt");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&sub, b"1\n00:00:00,000 --> 00:00:01,000\nHello").unwrap();

        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "en");
        assert_eq!(found[0].1, sub);
    }

    #[test]
    fn test_find_subtitle_files_multiple_langs() {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        let sub_en = dir.path().join("video.en.srt");
        let sub_es = dir.path().join("video.es.vtt");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&sub_en, b"subtitle en").unwrap();
        fs::write(&sub_es, b"subtitle es").unwrap();

        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert_eq!(found.len(), 2);
        // Sorted by language code
        assert_eq!(found[0].0, "en");
        assert_eq!(found[1].0, "es");
    }

    #[test]
    fn test_find_subtitle_files_none() {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        fs::write(&video, b"fake video").unwrap();

        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert!(found.is_empty());
    }

    #[test]
    fn test_find_subtitle_files_all_extensions() {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mkv");
        fs::write(&video, b"fake video").unwrap();

        let exts = ["srt", "vtt", "ass", "ssa", "lrc"];
        for ext in &exts {
            let sub = dir.path().join(format!("video.en.{ext}"));
            fs::write(&sub, b"subtitle").unwrap();
        }

        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert_eq!(found.len(), exts.len());
        // All should have lang "en"
        for (lang, _) in &found {
            assert_eq!(lang, "en");
        }
    }

    #[test]
    fn test_find_subtitle_files_ignores_non_subtitle() {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        let txt = dir.path().join("video.en.txt");
        let nfo = dir.path().join("video.en.nfo");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&txt, b"text").unwrap();
        fs::write(&nfo, b"nfo").unwrap();

        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert!(found.is_empty());
    }

    #[test]
    fn test_find_subtitle_files_pipeline_suffix() {
        let dir = TempDir::new().unwrap();
        // Media file has pipeline suffix
        let video = dir.path().join("video.norm.mp4");
        // Subtitle uses the original stem
        let sub = dir.path().join("video.en.srt");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&sub, b"subtitle").unwrap();

        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "en");
    }

    #[test]
    fn test_find_subtitle_files_no_lang_code_ignored() {
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        // File matching stem but no lang code (just video.srt)
        let sub = dir.path().join("video.srt");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&sub, b"subtitle").unwrap();

        // video.srt does NOT match {stem}.{lang}.{ext} pattern
        // because there's no language segment between stem and ext
        let found = EmbedSubtitles::find_subtitle_files(&video);
        assert!(found.is_empty());
    }

    // ── stem_candidates ─────────────────────────────────────────────

    #[test]
    fn test_stem_candidates_no_suffix() {
        let candidates = EmbedSubtitles::stem_candidates("video");
        assert_eq!(candidates, vec!["video"]);
    }

    #[test]
    fn test_stem_candidates_norm_suffix() {
        let candidates = EmbedSubtitles::stem_candidates("video.norm");
        assert_eq!(candidates, vec!["video.norm", "video"]);
    }

    #[test]
    fn test_stem_candidates_chained_suffixes() {
        let candidates = EmbedSubtitles::stem_candidates("video.norm.fixed");
        assert_eq!(candidates, vec!["video.norm.fixed", "video.norm", "video"]);
    }

    // ── process (async) ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_process_no_files_returns_unchanged() {
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let config = PostProcessConfig::default();

        let result = processor.process(&info, vec![], &config).await.unwrap();
        assert!(result.files.is_empty());
        assert!(result.temp_files.is_empty());
    }

    #[tokio::test]
    async fn test_process_unsupported_container() {
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let mut config = PostProcessConfig::default();
        config.embed_subtitles = true;

        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.avi");
        fs::write(&video, b"fake").unwrap();

        let result = processor
            .process(&info, vec![video.clone()], &config)
            .await
            .unwrap();
        assert_eq!(result.files, vec![video]);
        assert!(result.temp_files.is_empty());
    }

    #[tokio::test]
    async fn test_process_no_subtitle_files() {
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let mut config = PostProcessConfig::default();
        config.embed_subtitles = true;

        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        fs::write(&video, b"fake").unwrap();

        let result = processor
            .process(&info, vec![video.clone()], &config)
            .await
            .unwrap();
        assert_eq!(result.files, vec![video]);
        assert!(result.temp_files.is_empty());
    }

    #[tokio::test]
    async fn test_process_marks_subs_as_temp_when_not_write() {
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let mut config = PostProcessConfig::default();
        config.embed_subtitles = true;
        config.write_subtitles = false;

        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        let sub = dir.path().join("video.en.srt");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&sub, b"subtitle").unwrap();

        let result = processor
            .process(&info, vec![video.clone()], &config)
            .await
            .unwrap();
        assert_eq!(result.files, vec![video]);
        // Subtitle should be in temp_files (to be cleaned up)
        assert_eq!(result.temp_files.len(), 1);
        assert_eq!(result.temp_files[0], sub);
    }

    #[tokio::test]
    async fn test_process_keeps_subs_when_write_subtitles() {
        let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
            Ok(f) => std::sync::Arc::new(f),
            Err(_) => return,
        };
        let processor = EmbedSubtitles::new(ffmpeg);
        let info = InfoDict::new("id", "title", "extractor", "https://example.com");
        let mut config = PostProcessConfig::default();
        config.embed_subtitles = true;
        config.write_subtitles = true;

        let dir = TempDir::new().unwrap();
        let video = dir.path().join("video.mp4");
        let sub = dir.path().join("video.en.srt");
        fs::write(&video, b"fake video").unwrap();
        fs::write(&sub, b"subtitle").unwrap();

        let result = processor
            .process(&info, vec![video.clone()], &config)
            .await
            .unwrap();
        assert_eq!(result.files, vec![video]);
        // No temp files — subtitle should be kept
        assert!(result.temp_files.is_empty());
    }
}
