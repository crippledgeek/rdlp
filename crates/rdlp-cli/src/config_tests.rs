//! Tests for configuration building and merging.

use super::*;
use crate::args::Args;
use rdlp_core::{AudioFormat, Config, SubtitleFormat};

/// Helper: create default Args for testing (all fields at defaults).
fn default_args() -> Args {
    Args {
        url: None,
        output: None,
        output_dir: None,
        format: None,
        audio_multistreams: false,
        quiet: false,
        verbose: false,
        list_extractors: false,
        list_downloaders: false,
        list_codecs: false,
        simulate: false,
        dump_json: false,
        print: None,
        interactive: false,
        extract_audio: false,
        audio_format: None,
        audio_quality: None,
        embed_metadata: false,
        no_thumbnail: false,
        write_thumbnail: false,
        write_subtitles: false,
        write_auto_subtitles: false,
        sub_langs: None,
        sub_format: None,
        embed_subtitles: false,
        list_subs: false,
        list_subs_only: false,
        strict_subs: false,
        verify_sub_urls: false,
        retry_subs: false,
        recode_video: None,
        remux: None,
        normalize_audio: false,
        loudnorm: false,
        audio_gain_target: None,
        loudnorm_preset: None,
        loudnorm_i: None,
        loudnorm_tp: None,
        loudnorm_lra: None,
        loudnorm_dynamic: false,
        loudnorm_precompress: false,
        normalize_boost: false,
        normalize_boost_db: None,
        keep_video: false,
        ffmpeg_location: None,
        proxy: None,
        limit_rate: None,
        cookies_from_browser: None,
        cookies: None,
        download_archive: None,
        search: None,
        search_site: None,
        search_filter: vec![],
        ignore_config: false,
        config_location: None,
        video_encoder: None,
        list_encoders: false,
        recode_container: None,
        recode_audio: "copy".to_string(),
    }
}

/// Helper: no-op interactive values (nothing interactive selected).
fn no_interactive() -> ResolvedInteractiveValues {
    ResolvedInteractiveValues {
        audio_format: None,
        recode_video: None,
        remux_container: None,
    }
}

// === Subtitle config merge tests ===

#[test]
fn test_merge_config_write_subtitles() {
    let mut args = default_args();
    args.write_subtitles = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.write_subtitles);
    assert!(!config.write_auto_subtitles);
    assert!(!config.embed_subtitles);
}

#[test]
fn test_merge_config_write_auto_subtitles() {
    let mut args = default_args();
    args.write_auto_subtitles = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.write_auto_subtitles);
}

#[test]
fn test_merge_config_sub_langs_parsing() {
    let mut args = default_args();
    args.sub_langs = Some("en, es , fr".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(
        config.subtitle_langs,
        vec!["en".to_string(), "es".to_string(), "fr".to_string()]
    );
}

#[test]
fn test_merge_config_sub_langs_single() {
    let mut args = default_args();
    args.sub_langs = Some("en".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.subtitle_langs, vec!["en".to_string()]);
}

#[test]
fn test_merge_config_sub_langs_all() {
    let mut args = default_args();
    args.sub_langs = Some("all".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.subtitle_langs, vec!["all".to_string()]);
}

#[test]
fn test_merge_config_sub_format_parsing() {
    let mut args = default_args();
    args.sub_format = Some("srt".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.subtitle_format, Some(SubtitleFormat::Srt));
}

#[test]
fn test_merge_config_sub_format_vtt() {
    let mut args = default_args();
    args.sub_format = Some("vtt".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.subtitle_format, Some(SubtitleFormat::Vtt));
}

#[test]
fn test_merge_config_sub_format_ass() {
    let mut args = default_args();
    args.sub_format = Some("ass".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.subtitle_format, Some(SubtitleFormat::Ass));
}

#[test]
fn test_merge_config_sub_format_invalid() {
    let mut args = default_args();
    args.sub_format = Some("invalid_format".to_string());

    let result = merge_config(&args, Config::default(), no_interactive());
    assert!(result.is_err(), "Invalid subtitle format should fail");
}

#[test]
fn test_merge_config_embed_implies_write() {
    let mut args = default_args();
    args.embed_subtitles = true;
    // write_subtitles is NOT explicitly set

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.embed_subtitles);
    assert!(
        config.write_subtitles,
        "--embed-subtitles should imply --write-subtitles"
    );
}

#[test]
fn test_merge_config_embed_with_write_already_set() {
    let mut args = default_args();
    args.embed_subtitles = true;
    args.write_subtitles = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.embed_subtitles);
    assert!(config.write_subtitles);
}

#[test]
fn test_merge_config_subtitle_defaults_are_off() {
    let args = default_args();

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(!config.write_subtitles);
    assert!(!config.write_auto_subtitles);
    assert!(!config.embed_subtitles);
    assert!(config.subtitle_langs.is_empty());
    assert!(config.subtitle_format.is_none());
}

#[test]
fn test_merge_config_subtitle_combined_options() {
    let mut args = default_args();
    args.write_subtitles = true;
    args.write_auto_subtitles = true;
    args.sub_langs = Some("en,es".to_string());
    args.sub_format = Some("vtt".to_string());
    args.embed_subtitles = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.write_subtitles);
    assert!(config.write_auto_subtitles);
    assert!(config.embed_subtitles);
    assert_eq!(
        config.subtitle_langs,
        vec!["en".to_string(), "es".to_string()]
    );
    assert_eq!(config.subtitle_format, Some(SubtitleFormat::Vtt));
}

#[test]
fn test_merge_config_list_subs() {
    let mut args = default_args();
    args.list_subs = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(
        config.write_subtitles,
        "--list-subs should imply --write-subtitles"
    );
    assert!(config.list_subs, "--list-subs should set list_subs");
}

#[test]
fn test_merge_config_list_subs_only() {
    let mut args = default_args();
    args.list_subs_only = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(
        config.write_subtitles,
        "--list-subs-only should imply --write-subtitles"
    );
    assert!(config.list_subs, "--list-subs-only should set list_subs");
}

// === Existing config merge tests for non-subtitle features ===

#[test]
fn test_merge_config_defaults() {
    let args = default_args();
    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.output_template, "%(title)s.%(ext)s");
    assert!(!config.quiet);
    assert!(!config.verbose);
}

#[test]
fn test_merge_config_output_template() {
    let mut args = default_args();
    args.output = Some("%(id)s.%(ext)s".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.output_template, "%(id)s.%(ext)s");
}

#[test]
fn test_merge_config_extract_audio() {
    let mut args = default_args();
    args.extract_audio = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.extract_audio);
    // extract_audio without format defaults to mp3
    assert_eq!(config.audio_format, Some(AudioFormat::Mp3));
}

#[test]
fn test_merge_config_audio_format_direct() {
    let mut args = default_args();
    args.audio_format = Some("flac".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.audio_format, Some(AudioFormat::Flac));
}

#[test]
fn test_merge_config_normalize_boost_implies_loudnorm() {
    let mut args = default_args();
    args.normalize_boost = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.normalize_boost);
    assert!(config.loudnorm, "normalize_boost should imply loudnorm");
    assert!(
        config.normalize_audio,
        "normalize_boost should imply normalize_audio"
    );
}

// === Search CLI tests ===

#[test]
fn test_search_args_parse() {
    use clap::Parser;
    let args = Args::try_parse_from([
        "rdlp",
        "--search",
        "test query",
        "--search-site",
        "xhamster",
        "--search-filter",
        "quality=1080p",
        "--search-filter",
        "sort=newest",
    ])
    .unwrap();
    assert_eq!(args.search.as_deref(), Some("test query"));
    assert_eq!(args.search_site.as_deref(), Some("xhamster"));
    assert_eq!(args.search_filter.len(), 2);
}

#[test]
fn test_search_filter_parsing() {
    let raw = "quality=1080p";
    let (key, value) = raw.split_once('=').unwrap();
    assert_eq!(key, "quality");
    assert_eq!(value, "1080p");
}

// === Audio multistreams tests ===

#[test]
fn test_merge_config_audio_multistreams() {
    let mut args = default_args();
    args.audio_multistreams = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.audio_multistreams);
}

#[test]
fn test_merge_config_audio_multistreams_default_off() {
    let args = default_args();

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(!config.audio_multistreams);
}

// === Stdout streaming tests ===

#[test]
fn test_merge_config_stdout_sets_flags() {
    let mut args = default_args();
    args.output = Some("-".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.output_to_stdout);
    assert!(config.quiet);
    assert!(!config.progress);
    assert!(!config.embed_thumbnail);
}

#[test]
fn test_merge_config_stdout_does_not_set_output_template() {
    let mut args = default_args();
    args.output = Some("-".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    // output_template should remain the default, not "-"
    assert_eq!(config.output_template, "%(title)s.%(ext)s");
}

#[test]
fn test_merge_config_stdout_with_remux_fails() {
    let mut args = default_args();
    args.output = Some("-".to_string());
    args.remux = Some("mp4".to_string());

    let result = merge_config(&args, Config::default(), no_interactive());
    assert!(result.is_err(), "-o - with --remux should fail validation");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("remux"),
        "Error should mention remux: {err_msg}"
    );
}

#[test]
fn test_merge_config_stdout_with_extract_audio_fails() {
    let mut args = default_args();
    args.output = Some("-".to_string());
    args.extract_audio = true;

    let result = merge_config(&args, Config::default(), no_interactive());
    assert!(
        result.is_err(),
        "-o - with --extract-audio should fail validation"
    );
}

#[test]
fn test_merge_config_stdout_with_embed_metadata_fails() {
    let mut args = default_args();
    args.output = Some("-".to_string());
    args.embed_metadata = true;

    let result = merge_config(&args, Config::default(), no_interactive());
    assert!(
        result.is_err(),
        "-o - with --embed-metadata should fail validation"
    );
}

#[test]
fn test_merge_config_normal_output_not_stdout() {
    let mut args = default_args();
    args.output = Some("%(title)s.%(ext)s".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(!config.output_to_stdout);
    assert_eq!(config.output_template, "%(title)s.%(ext)s");
}
