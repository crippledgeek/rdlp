//! Tests for PostProcessOptions and NetworkOptions merge overrides

use super::*;
use std::path::PathBuf;

// --- PostProcessOptions ---

#[test]
fn test_postprocess_none_preserves_remux() {
    let mut config = Config {
        remux_container: Some(rdlp_types::ContainerFormat::Mkv),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(
        config.remux_container,
        Some(rdlp_types::ContainerFormat::Mkv)
    );
}

#[test]
fn test_postprocess_some_overrides_remux() {
    let mut config = Config {
        remux_container: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        remux: Some(rdlp_types::ContainerFormat::Mp4),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(
        config.remux_container,
        Some(rdlp_types::ContainerFormat::Mp4)
    );
}

#[test]
fn test_postprocess_none_preserves_extract_audio() {
    let mut config = Config {
        extract_audio: true,
        audio_format: Some(rdlp_types::AudioFormat::Mp3),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.extract_audio);
    assert_eq!(config.audio_format, Some(rdlp_types::AudioFormat::Mp3));
}

#[test]
fn test_postprocess_some_overrides_extract_audio() {
    let mut config = Config {
        extract_audio: false,
        audio_format: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        extract_audio: Some(rdlp_types::AudioFormat::Aac),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.extract_audio);
    assert_eq!(config.audio_format, Some(rdlp_types::AudioFormat::Aac));
}

#[test]
fn test_postprocess_none_preserves_embed_metadata() {
    let mut config = Config {
        embed_metadata: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.embed_metadata);
}

#[test]
fn test_postprocess_some_overrides_embed_metadata() {
    let mut config = Config {
        embed_metadata: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        embed_metadata: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.embed_metadata);
}

#[test]
fn test_postprocess_none_preserves_embed_thumbnail() {
    let mut config = Config {
        embed_thumbnail: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.embed_thumbnail);
}

#[test]
fn test_postprocess_some_overrides_embed_thumbnail() {
    let mut config = Config {
        embed_thumbnail: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        embed_thumbnail: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.embed_thumbnail);
}

#[test]
fn test_postprocess_none_preserves_no_thumbnail() {
    let mut config = Config {
        embed_thumbnail: true,
        write_thumbnail: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.embed_thumbnail);
    assert!(config.write_thumbnail);
}

#[test]
fn test_postprocess_some_overrides_no_thumbnail() {
    let mut config = Config {
        embed_thumbnail: true,
        write_thumbnail: true,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        no_thumbnail: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(!config.embed_thumbnail);
    assert!(!config.write_thumbnail);
}

#[test]
fn test_postprocess_none_preserves_write_thumbnail() {
    let mut config = Config {
        write_thumbnail: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.write_thumbnail);
}

#[test]
fn test_postprocess_some_overrides_write_thumbnail() {
    let mut config = Config {
        write_thumbnail: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        write_thumbnail: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.write_thumbnail);
}

#[test]
fn test_postprocess_none_preserves_normalize_audio() {
    let mut config = Config {
        normalize_audio: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.normalize_audio);
}

#[test]
fn test_postprocess_some_overrides_normalize_audio() {
    let mut config = Config {
        normalize_audio: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        normalize_audio: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.normalize_audio);
}

#[test]
fn test_postprocess_none_preserves_loudnorm() {
    let mut config = Config {
        loudnorm: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.loudnorm);
}

#[test]
fn test_postprocess_some_overrides_loudnorm() {
    let mut config = Config {
        loudnorm: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.loudnorm);
}

#[test]
fn test_postprocess_none_preserves_loudnorm_preset() {
    let mut config = Config {
        loudnorm_preset: Some("broadcast".into()),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_preset.as_deref(), Some("broadcast"));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_preset() {
    let mut config = Config {
        loudnorm_preset: Some("broadcast".into()),
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm_preset: Some("streaming".into()),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_preset.as_deref(), Some("streaming"));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_target_i() {
    let mut config = Config {
        loudnorm_target_i: Some(-16.0),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_target_i, Some(-16.0));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_target_i() {
    let mut config = Config {
        loudnorm_target_i: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm_target_i: Some(-23.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_target_i, Some(-23.0));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_target_tp() {
    let mut config = Config {
        loudnorm_target_tp: Some(-1.0),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_target_tp, Some(-1.0));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_target_tp() {
    let mut config = Config {
        loudnorm_target_tp: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm_target_tp: Some(-2.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_target_tp, Some(-2.0));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_target_lra() {
    let mut config = Config {
        loudnorm_target_lra: Some(7.0),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_target_lra, Some(7.0));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_target_lra() {
    let mut config = Config {
        loudnorm_target_lra: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm_target_lra: Some(11.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.loudnorm_target_lra, Some(11.0));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_dynamic() {
    let mut config = Config {
        loudnorm_dynamic: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.loudnorm_dynamic);
}

#[test]
fn test_postprocess_some_overrides_loudnorm_dynamic() {
    let mut config = Config {
        loudnorm_dynamic: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm_dynamic: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.loudnorm_dynamic);
}

#[test]
fn test_postprocess_none_preserves_loudnorm_precompress() {
    let mut config = Config {
        loudnorm_precompress: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.loudnorm_precompress);
}

#[test]
fn test_postprocess_some_overrides_loudnorm_precompress() {
    let mut config = Config {
        loudnorm_precompress: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        loudnorm_precompress: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.loudnorm_precompress);
}

#[test]
fn test_postprocess_none_preserves_normalize_boost() {
    let mut config = Config {
        normalize_boost: true,
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.normalize_boost);
}

#[test]
fn test_postprocess_some_overrides_normalize_boost() {
    let mut config = Config {
        normalize_boost: false,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        normalize_boost: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.normalize_boost);
}

#[test]
fn test_postprocess_none_preserves_normalize_boost_db() {
    let mut config = Config {
        normalize_boost_db: Some(8.0),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.normalize_boost_db, Some(8.0));
}

#[test]
fn test_postprocess_some_overrides_normalize_boost_db() {
    let mut config = Config {
        normalize_boost_db: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        normalize_boost_db: Some(12.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.normalize_boost_db, Some(12.0));
}

#[test]
fn test_postprocess_none_preserves_recode_video() {
    let mut config = Config {
        recode_video: Some(rdlp_types::ContainerFormat::Mkv),
        ..Config::default()
    };
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.recode_video, Some(rdlp_types::ContainerFormat::Mkv));
}

#[test]
fn test_postprocess_some_overrides_recode_video() {
    let mut config = Config {
        recode_video: None,
        ..Config::default()
    };
    let opts = PostProcessOptions {
        recode_video: Some(rdlp_types::ContainerFormat::Mp4),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.recode_video, Some(rdlp_types::ContainerFormat::Mp4));
}

// --- NetworkOptions ---

#[test]
fn test_network_none_preserves_retries() {
    let mut config = Config {
        retries: 42,
        ..Config::default()
    };
    let opts = NetworkOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.retries, 42);
}

#[test]
fn test_network_some_overrides_retries() {
    let mut config = Config {
        retries: 10,
        ..Config::default()
    };
    let opts = NetworkOptions {
        retries: Some(3),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.retries, 3);
}

#[test]
fn test_network_none_preserves_timeout_secs() {
    let mut config = Config {
        socket_timeout: Some(99),
        ..Config::default()
    };
    let opts = NetworkOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.socket_timeout, Some(99));
}

#[test]
fn test_network_some_overrides_timeout_secs() {
    let mut config = Config {
        socket_timeout: Some(30),
        ..Config::default()
    };
    let opts = NetworkOptions {
        timeout_secs: Some(120),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.socket_timeout, Some(120));
}

#[test]
fn test_network_none_preserves_concurrent_fragments() {
    let mut config = Config {
        concurrent_fragments: 16,
        ..Config::default()
    };
    let opts = NetworkOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.concurrent_fragments, 16);
}

#[test]
fn test_network_some_overrides_concurrent_fragments() {
    let mut config = Config {
        concurrent_fragments: 4,
        ..Config::default()
    };
    let opts = NetworkOptions {
        concurrent_fragments: Some(8),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.concurrent_fragments, 8);
}

#[test]
fn test_network_none_preserves_rate_limit() {
    let mut config = Config {
        rate_limit: Some(1_000_000),
        ..Config::default()
    };
    let opts = NetworkOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.rate_limit, Some(1_000_000));
}

#[test]
fn test_network_some_overrides_rate_limit() {
    let mut config = Config {
        rate_limit: None,
        ..Config::default()
    };
    let opts = NetworkOptions {
        rate_limit: Some(500_000),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.rate_limit, Some(500_000));
}

#[test]
fn test_network_none_preserves_cookies_from_browser() {
    let mut config = Config {
        cookies_from_browser: Some(rdlp_types::BrowserType::Firefox),
        ..Config::default()
    };
    let opts = NetworkOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(
        config.cookies_from_browser,
        Some(rdlp_types::BrowserType::Firefox)
    );
}

#[test]
fn test_network_some_overrides_cookies_from_browser() {
    let mut config = Config {
        cookies_from_browser: None,
        ..Config::default()
    };
    let opts = NetworkOptions {
        cookies_from_browser: Some(rdlp_types::BrowserType::Chrome),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(
        config.cookies_from_browser,
        Some(rdlp_types::BrowserType::Chrome)
    );
}

#[test]
fn test_network_none_preserves_cookies_file() {
    let mut config = Config {
        cookies_file: Some(PathBuf::from("/kept/cookies.txt")),
        ..Config::default()
    };
    let opts = NetworkOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(
        config.cookies_file,
        Some(PathBuf::from("/kept/cookies.txt"))
    );
}

#[test]
fn test_network_some_overrides_cookies_file() {
    let mut config = Config {
        cookies_file: None,
        ..Config::default()
    };
    let opts = NetworkOptions {
        cookies_file: Some(PathBuf::from("/new/cookies.txt")),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.cookies_file, Some(PathBuf::from("/new/cookies.txt")));
}
