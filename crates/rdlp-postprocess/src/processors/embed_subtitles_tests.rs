//! Tests for the subtitle embedding post-processor.

use super::*;
use std::fs;
use tempfile::TempDir;

// -- supports_subtitles ------------------------------------------------------

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

// -- subtitle_codec_for_container --------------------------------------------

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

// -- should_run --------------------------------------------------------------

#[test]
fn test_should_run_when_enabled() {
    // FFmpegRunner requires FFmpeg library; skip if unavailable
    let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
        Ok(f) => std::sync::Arc::new(f),
        Err(_) => return,
    };
    let processor = EmbedSubtitles::new(ffmpeg);
    let info = InfoDict::new("id", "title", "extractor", "https://example.com");
    let config = PostProcessConfig {
        embed_subtitles: true,
        ..Default::default()
    };
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

// -- find_subtitle_files -----------------------------------------------------

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

// -- stem_candidates ---------------------------------------------------------

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

// -- process (async) ---------------------------------------------------------

#[tokio::test]
async fn test_process_no_files_returns_unchanged() {
    let ffmpeg = match rdlp_ffmpeg::FFmpegRunner::new() {
        Ok(f) => std::sync::Arc::new(f),
        Err(_) => return,
    };
    let processor = EmbedSubtitles::new(ffmpeg);
    let info = InfoDict::new("id", "title", "extractor", "https://example.com");
    let config = PostProcessConfig::default();

    let result = processor
        .process(&info, vec![], &config, None)
        .await
        .unwrap();
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
    let config = PostProcessConfig {
        embed_subtitles: true,
        ..Default::default()
    };

    let dir = TempDir::new().unwrap();
    let video = dir.path().join("video.avi");
    fs::write(&video, b"fake").unwrap();

    let result = processor
        .process(&info, vec![video.clone()], &config, None)
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
    let config = PostProcessConfig {
        embed_subtitles: true,
        ..Default::default()
    };

    let dir = TempDir::new().unwrap();
    let video = dir.path().join("video.mp4");
    fs::write(&video, b"fake").unwrap();

    let result = processor
        .process(&info, vec![video.clone()], &config, None)
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
    let config = PostProcessConfig {
        embed_subtitles: true,
        write_subtitles: false,
        ..Default::default()
    };

    let dir = TempDir::new().unwrap();
    let video = dir.path().join("video.mp4");
    let sub = dir.path().join("video.en.srt");
    fs::write(&video, b"fake video").unwrap();
    fs::write(&sub, b"subtitle").unwrap();

    let result = processor
        .process(&info, vec![video.clone()], &config, None)
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
    let config = PostProcessConfig {
        embed_subtitles: true,
        write_subtitles: true,
        ..Default::default()
    };

    let dir = TempDir::new().unwrap();
    let video = dir.path().join("video.mp4");
    let sub = dir.path().join("video.en.srt");
    fs::write(&video, b"fake video").unwrap();
    fs::write(&sub, b"subtitle").unwrap();

    let result = processor
        .process(&info, vec![video.clone()], &config, None)
        .await
        .unwrap();
    assert_eq!(result.files, vec![video]);
    // No temp files -- subtitle should be kept
    assert!(result.temp_files.is_empty());
}
