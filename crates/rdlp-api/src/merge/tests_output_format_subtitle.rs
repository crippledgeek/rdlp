//! Tests for OutputOptions, FormatOptions, and SubtitleOptions merge overrides

use super::*;
use std::path::PathBuf;

// --- OutputOptions ---

#[test]
fn test_output_none_preserves_output_dir() {
    let mut config = Config {
        output_directory: PathBuf::from("/kept"),
        ..Config::default()
    };
    let opts = OutputOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.output_directory, PathBuf::from("/kept"));
}

#[test]
fn test_output_some_overrides_output_dir() {
    let mut config = Config {
        output_directory: PathBuf::from("/old"),
        ..Config::default()
    };
    let opts = OutputOptions {
        output_dir: Some(PathBuf::from("/new")),
        ..OutputOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.output_directory, PathBuf::from("/new"));
}

#[test]
fn test_output_none_preserves_template() {
    let mut config = Config {
        output_template: "kept.%(ext)s".to_string(),
        ..Config::default()
    };
    let opts = OutputOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.output_template, "kept.%(ext)s");
}

#[test]
fn test_output_stdout_disables_incompatible_defaults() {
    let mut config = Config {
        embed_thumbnail: true,
        ..Config::default()
    };
    let opts = OutputOptions {
        stdout: Some(true),
        ..OutputOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.output_to_stdout);
    assert!(config.quiet);
    assert!(!config.progress);
    assert!(!config.embed_thumbnail);
}

#[test]
fn test_output_some_overrides_template() {
    let mut config = Config {
        output_template: "old.%(ext)s".to_string(),
        ..Config::default()
    };
    let opts = OutputOptions {
        template: Some("new.%(ext)s".into()),
        ..OutputOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.output_template, "new.%(ext)s");
}

// --- FormatOptions ---

#[test]
fn test_format_none_preserves_selector() {
    let mut config = Config {
        format: Some("kept".to_string()),
        ..Config::default()
    };
    let opts = FormatOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.format, Some("kept".to_string()));
}

#[test]
fn test_format_some_overrides_selector() {
    let mut config = Config {
        format: Some("old".to_string()),
        ..Config::default()
    };
    let opts = FormatOptions {
        selector: Some("bestaudio".into()),
        ..FormatOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.format, Some("bestaudio".to_string()));
}

#[test]
fn test_format_audio_multistreams_none_preserves() {
    let mut config = Config {
        audio_multistreams: true,
        ..Config::default()
    };
    let opts = FormatOptions::default();
    opts.merge_into(&mut config);
    assert!(config.audio_multistreams);
}

#[test]
fn test_format_audio_multistreams_some_overrides() {
    let mut config = Config {
        audio_multistreams: false,
        ..Config::default()
    };
    let opts = FormatOptions {
        audio_multistreams: Some(true),
        ..FormatOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.audio_multistreams);
}

// --- SubtitleOptions ---

#[test]
fn test_subtitle_none_preserves_write_subs() {
    let mut config = Config {
        write_subtitles: true,
        ..Config::default()
    };
    let opts = SubtitleOptions::default();
    opts.merge_into(&mut config);
    assert!(config.write_subtitles);
}

#[test]
fn test_subtitle_some_overrides_write_subs() {
    let mut config = Config {
        write_subtitles: false,
        ..Config::default()
    };
    let opts = SubtitleOptions {
        write_subs: Some(true),
        ..SubtitleOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.write_subtitles);
}

#[test]
fn test_subtitle_none_preserves_write_auto_subs() {
    let mut config = Config {
        write_auto_subtitles: true,
        ..Config::default()
    };
    let opts = SubtitleOptions::default();
    opts.merge_into(&mut config);
    assert!(config.write_auto_subtitles);
}

#[test]
fn test_subtitle_some_overrides_write_auto_subs() {
    let mut config = Config {
        write_auto_subtitles: false,
        ..Config::default()
    };
    let opts = SubtitleOptions {
        write_auto_subs: Some(true),
        ..SubtitleOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.write_auto_subtitles);
}

#[test]
fn test_subtitle_empty_preserves_sub_langs() {
    let mut config = Config {
        subtitle_langs: vec!["en".into(), "ja".into()],
        ..Config::default()
    };
    let opts = SubtitleOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.subtitle_langs, vec!["en", "ja"]);
}

#[test]
fn test_subtitle_nonempty_overrides_sub_langs() {
    let mut config = Config {
        subtitle_langs: vec!["en".into()],
        ..Config::default()
    };
    let opts = SubtitleOptions {
        sub_langs: vec!["de".into(), "fr".into()],
        ..SubtitleOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.subtitle_langs, vec!["de", "fr"]);
}

#[test]
fn test_subtitle_none_preserves_sub_format() {
    let mut config = Config {
        subtitle_format: Some(rdlp_core::SubtitleFormat::Vtt),
        ..Config::default()
    };
    let opts = SubtitleOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.subtitle_format, Some(rdlp_core::SubtitleFormat::Vtt));
}

#[test]
fn test_subtitle_some_overrides_sub_format() {
    let mut config = Config {
        subtitle_format: Some(rdlp_core::SubtitleFormat::Vtt),
        ..Config::default()
    };
    let opts = SubtitleOptions {
        sub_format: Some(rdlp_core::SubtitleFormat::Srt),
        ..SubtitleOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.subtitle_format, Some(rdlp_core::SubtitleFormat::Srt));
}

#[test]
fn test_subtitle_none_preserves_embed_subs() {
    let mut config = Config {
        embed_subtitles: true,
        ..Config::default()
    };
    let opts = SubtitleOptions::default();
    opts.merge_into(&mut config);
    assert!(config.embed_subtitles);
}

#[test]
fn test_subtitle_some_overrides_embed_subs() {
    let mut config = Config {
        embed_subtitles: false,
        ..Config::default()
    };
    let opts = SubtitleOptions {
        embed_subs: Some(true),
        ..SubtitleOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.embed_subtitles);
}

#[test]
fn test_subtitle_none_preserves_strict_subs() {
    let mut config = Config {
        strict_subs: true,
        ..Config::default()
    };
    let opts = SubtitleOptions::default();
    opts.merge_into(&mut config);
    assert!(config.strict_subs);
}

#[test]
fn test_subtitle_some_overrides_strict_subs() {
    let mut config = Config {
        strict_subs: false,
        ..Config::default()
    };
    let opts = SubtitleOptions {
        strict_subs: Some(true),
        ..SubtitleOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.strict_subs);
}
