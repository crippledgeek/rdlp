//! Tests for `PostProcessOptions` and `NetworkOptions` merge overrides

use super::*;
use std::path::PathBuf;

// --- PostProcessOptions ---

#[test]
fn test_postprocess_none_preserves_remux() {
    let mut config = Config::default();
    config.postprocess.remux_container = Some(rdlp_types::ContainerFormat::Mkv);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(
        config.postprocess.remux_container,
        Some(rdlp_types::ContainerFormat::Mkv)
    );
}

#[test]
fn test_postprocess_some_overrides_remux() {
    let mut config = Config::default();
    config.postprocess.remux_container = None;
    let opts = PostProcessOptions {
        remux: Some(rdlp_types::ContainerFormat::Mp4),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(
        config.postprocess.remux_container,
        Some(rdlp_types::ContainerFormat::Mp4)
    );
}

#[test]
fn test_postprocess_none_preserves_extract_audio() {
    let mut config = Config::default();
    config.postprocess.extract_audio = true;
    config.postprocess.audio_format = Some(rdlp_types::AudioFormat::Mp3);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.extract_audio);
    assert_eq!(
        config.postprocess.audio_format,
        Some(rdlp_types::AudioFormat::Mp3)
    );
}

#[test]
fn test_postprocess_some_overrides_extract_audio() {
    let mut config = Config::default();
    config.postprocess.extract_audio = false;
    config.postprocess.audio_format = None;
    let opts = PostProcessOptions {
        extract_audio: Some(rdlp_types::AudioFormat::Aac),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.extract_audio);
    assert_eq!(
        config.postprocess.audio_format,
        Some(rdlp_types::AudioFormat::Aac)
    );
}

#[test]
fn test_postprocess_none_preserves_embed_metadata() {
    let mut config = Config::default();
    config.postprocess.embed_metadata = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.embed_metadata);
}

#[test]
fn test_postprocess_some_overrides_embed_metadata() {
    let mut config = Config::default();
    config.postprocess.embed_metadata = false;
    let opts = PostProcessOptions {
        embed_metadata: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.embed_metadata);
}

#[test]
fn test_postprocess_none_preserves_embed_thumbnail() {
    let mut config = Config::default();
    config.postprocess.embed_thumbnail = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.embed_thumbnail);
}

#[test]
fn test_postprocess_some_overrides_embed_thumbnail() {
    let mut config = Config::default();
    config.postprocess.embed_thumbnail = false;
    let opts = PostProcessOptions {
        embed_thumbnail: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.embed_thumbnail);
}

#[test]
fn test_postprocess_none_preserves_no_thumbnail() {
    let mut config = Config::default();
    config.postprocess.embed_thumbnail = true;
    config.postprocess.write_thumbnail = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.embed_thumbnail);
    assert!(config.postprocess.write_thumbnail);
}

#[test]
fn test_postprocess_some_overrides_no_thumbnail() {
    let mut config = Config::default();
    config.postprocess.embed_thumbnail = true;
    config.postprocess.write_thumbnail = true;
    let opts = PostProcessOptions {
        no_thumbnail: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(!config.postprocess.embed_thumbnail);
    assert!(!config.postprocess.write_thumbnail);
}

#[test]
fn test_postprocess_none_preserves_write_thumbnail() {
    let mut config = Config::default();
    config.postprocess.write_thumbnail = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.write_thumbnail);
}

#[test]
fn test_postprocess_some_overrides_write_thumbnail() {
    let mut config = Config::default();
    config.postprocess.write_thumbnail = false;
    let opts = PostProcessOptions {
        write_thumbnail: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.write_thumbnail);
}

#[test]
fn test_postprocess_none_preserves_normalize_audio() {
    let mut config = Config::default();
    config.postprocess.normalize_audio = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.normalize_audio);
}

#[test]
fn test_postprocess_some_overrides_normalize_audio() {
    let mut config = Config::default();
    config.postprocess.normalize_audio = false;
    let opts = PostProcessOptions {
        normalize_audio: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.normalize_audio);
}

#[test]
fn test_postprocess_none_preserves_loudnorm() {
    let mut config = Config::default();
    config.postprocess.loudnorm = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.loudnorm);
}

#[test]
fn test_postprocess_some_overrides_loudnorm() {
    let mut config = Config::default();
    config.postprocess.loudnorm = false;
    let opts = PostProcessOptions {
        loudnorm: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.loudnorm);
}

#[test]
fn test_postprocess_none_preserves_loudnorm_preset() {
    let mut config = Config::default();
    config.postprocess.loudnorm_preset = Some("broadcast".into());
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(
        config.postprocess.loudnorm_preset.as_deref(),
        Some("broadcast")
    );
}

#[test]
fn test_postprocess_some_overrides_loudnorm_preset() {
    let mut config = Config::default();
    config.postprocess.loudnorm_preset = Some("broadcast".into());
    let opts = PostProcessOptions {
        loudnorm_preset: Some("streaming".into()),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(
        config.postprocess.loudnorm_preset.as_deref(),
        Some("streaming")
    );
}

#[test]
fn test_postprocess_none_preserves_loudnorm_target_i() {
    let mut config = Config::default();
    config.postprocess.loudnorm_target_i = Some(-16.0);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.loudnorm_target_i, Some(-16.0));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_target_i() {
    let mut config = Config::default();
    config.postprocess.loudnorm_target_i = None;
    let opts = PostProcessOptions {
        loudnorm_target_i: Some(-23.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.loudnorm_target_i, Some(-23.0));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_target_tp() {
    let mut config = Config::default();
    config.postprocess.loudnorm_target_tp = Some(-1.0);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.loudnorm_target_tp, Some(-1.0));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_target_tp() {
    let mut config = Config::default();
    config.postprocess.loudnorm_target_tp = None;
    let opts = PostProcessOptions {
        loudnorm_target_tp: Some(-2.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.loudnorm_target_tp, Some(-2.0));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_target_lra() {
    let mut config = Config::default();
    config.postprocess.loudnorm_target_lra = Some(7.0);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.loudnorm_target_lra, Some(7.0));
}

#[test]
fn test_postprocess_some_overrides_loudnorm_target_lra() {
    let mut config = Config::default();
    config.postprocess.loudnorm_target_lra = None;
    let opts = PostProcessOptions {
        loudnorm_target_lra: Some(11.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.loudnorm_target_lra, Some(11.0));
}

#[test]
fn test_postprocess_none_preserves_loudnorm_dynamic() {
    let mut config = Config::default();
    config.postprocess.loudnorm_dynamic = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.loudnorm_dynamic);
}

#[test]
fn test_postprocess_some_overrides_loudnorm_dynamic() {
    let mut config = Config::default();
    config.postprocess.loudnorm_dynamic = false;
    let opts = PostProcessOptions {
        loudnorm_dynamic: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.loudnorm_dynamic);
}

#[test]
fn test_postprocess_none_preserves_loudnorm_precompress() {
    let mut config = Config::default();
    config.postprocess.loudnorm_precompress = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.loudnorm_precompress);
}

#[test]
fn test_postprocess_some_overrides_loudnorm_precompress() {
    let mut config = Config::default();
    config.postprocess.loudnorm_precompress = false;
    let opts = PostProcessOptions {
        loudnorm_precompress: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.loudnorm_precompress);
}

#[test]
fn test_postprocess_none_preserves_normalize_boost() {
    let mut config = Config::default();
    config.postprocess.normalize_boost = true;
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert!(config.postprocess.normalize_boost);
}

#[test]
fn test_postprocess_some_overrides_normalize_boost() {
    let mut config = Config::default();
    config.postprocess.normalize_boost = false;
    let opts = PostProcessOptions {
        normalize_boost: Some(true),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert!(config.postprocess.normalize_boost);
}

#[test]
fn test_postprocess_none_preserves_normalize_boost_db() {
    let mut config = Config::default();
    config.postprocess.normalize_boost_db = Some(8.0);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.normalize_boost_db, Some(8.0));
}

#[test]
fn test_postprocess_some_overrides_normalize_boost_db() {
    let mut config = Config::default();
    config.postprocess.normalize_boost_db = None;
    let opts = PostProcessOptions {
        normalize_boost_db: Some(12.0),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.normalize_boost_db, Some(12.0));
}

#[test]
fn test_postprocess_none_preserves_recode_video() {
    let mut config = Config::default();
    config.postprocess.recode_video = Some(rdlp_types::ContainerFormat::Mkv);
    let opts = PostProcessOptions::default();
    opts.merge_into(&mut config);
    assert_eq!(
        config.postprocess.recode_video,
        Some(rdlp_types::ContainerFormat::Mkv)
    );
}

#[test]
fn test_postprocess_some_overrides_recode_video() {
    let mut config = Config::default();
    config.postprocess.recode_video = None;
    let opts = PostProcessOptions {
        recode_video: Some(rdlp_types::ContainerFormat::Mp4),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(
        config.postprocess.recode_video,
        Some(rdlp_types::ContainerFormat::Mp4)
    );
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

#[test]
fn network_options_merges_read_timeout() {
    let mut config = Config::default();
    let opts = NetworkOptions {
        read_timeout_secs: Some(45),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.read_timeout, Some(45));
}

#[test]
fn network_options_merges_pool_idle_timeout_positive() {
    let mut config = Config::default();
    let opts = NetworkOptions {
        pool_idle_timeout_secs: Some(120),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.pool_idle_timeout, Some(120));
}

#[test]
fn network_options_merges_pool_idle_timeout_zero_sentinel() {
    let mut config = Config::default();
    let opts = NetworkOptions {
        pool_idle_timeout_secs: Some(0),
        ..NetworkOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.pool_idle_timeout, Some(0));
}

#[test]
fn network_options_none_preserves_base_timeouts() {
    let mut config = Config {
        read_timeout: Some(99),
        pool_idle_timeout: Some(99),
        ..Config::default()
    };
    NetworkOptions::default().merge_into(&mut config);
    assert_eq!(config.read_timeout, Some(99));
    assert_eq!(config.pool_idle_timeout, Some(99));
}

#[test]
fn test_postprocess_recode_threads_and_preset_propagate() {
    let mut config = Config::default();
    let opts = PostProcessOptions {
        recode_threads: Some(6),
        recode_preset: Some("faster".to_string()),
        ..PostProcessOptions::default()
    };
    opts.merge_into(&mut config);
    assert_eq!(config.postprocess.recode_threads, Some(6));
    assert_eq!(config.postprocess.recode_preset, Some("faster".to_string()));
}

#[test]
fn test_postprocess_recode_threads_none_preserves_base() {
    let mut config = Config::default();
    config.postprocess.recode_threads = Some(4);
    config.postprocess.recode_preset = Some("slow".to_string());
    PostProcessOptions::default().merge_into(&mut config);
    assert_eq!(config.postprocess.recode_threads, Some(4));
    assert_eq!(config.postprocess.recode_preset, Some("slow".to_string()));
}
