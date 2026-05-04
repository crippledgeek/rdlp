//! Tests for configuration building and merging.

use super::*;
use crate::args::Args;
use rdlp_api::{AudioFormat, Config, SubtitleFormat};

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
        list_formats: false,
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
        socket_timeout: None,
        read_timeout: None,
        pool_idle_timeout: None,
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
        fixup: "detect_or_warn".to_string(),
        match_filter: vec![],
        browser: None,
        trust_publisher: vec![],
        plugin: None,
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

    assert!(config.postprocess.write_subtitles);
    assert!(!config.write_auto_subtitles);
    assert!(!config.postprocess.embed_subtitles);
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

    assert!(config.postprocess.embed_subtitles);
    assert!(
        config.postprocess.write_subtitles,
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

    assert!(config.postprocess.embed_subtitles);
    assert!(config.postprocess.write_subtitles);
}

#[test]
fn test_merge_config_subtitle_defaults_are_off() {
    let args = default_args();

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(!config.postprocess.write_subtitles);
    assert!(!config.write_auto_subtitles);
    assert!(!config.postprocess.embed_subtitles);
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

    assert!(config.postprocess.write_subtitles);
    assert!(config.write_auto_subtitles);
    assert!(config.postprocess.embed_subtitles);
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
        config.postprocess.write_subtitles,
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
        config.postprocess.write_subtitles,
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

    assert_eq!(config.output_template, "%(title|Unknown)s [%(id)s].%(ext)s");
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

    assert!(config.postprocess.extract_audio);
    // extract_audio without format defaults to mp3
    assert_eq!(config.postprocess.audio_format, Some(AudioFormat::Mp3));
}

#[test]
fn test_merge_config_audio_format_direct() {
    let mut args = default_args();
    args.audio_format = Some("flac".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert_eq!(config.postprocess.audio_format, Some(AudioFormat::Flac));
}

#[test]
fn test_merge_config_normalize_boost_implies_loudnorm() {
    let mut args = default_args();
    args.normalize_boost = true;

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(config.postprocess.normalize_boost);
    assert!(
        config.postprocess.loudnorm,
        "normalize_boost should imply loudnorm"
    );
    assert!(
        config.postprocess.normalize_audio,
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
    assert!(!config.postprocess.embed_thumbnail);
}

#[test]
fn test_merge_config_stdout_does_not_set_output_template() {
    let mut args = default_args();
    args.output = Some("-".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    // output_template should remain the default, not "-"
    assert_eq!(config.output_template, "%(title|Unknown)s [%(id)s].%(ext)s");
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

// === Browser emulation tests ===
//
// `merge_config` reads `RDLP_BROWSER_EMULATION` when `args.browser` is None.
// temp-env's singleton mutex serialises all tests that read or write this var;
// no per-module guard is needed.

#[test]
fn test_merge_config_browser_default_is_chrome_latest() {
    // temp-env's singleton mutex serialises against any other test that
    // reads or writes RDLP_BROWSER_EMULATION; the var is unset for the
    // duration of the closure.
    temp_env::with_var_unset("RDLP_BROWSER_EMULATION", || {
        let args = default_args();
        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");
        assert!(matches!(
            config.browser_emulation,
            rdlp_api::BrowserEmulation::ChromeLatest
        ));
    });
}

#[test]
fn test_merge_config_browser_cli_flag_firefox() {
    let mut args = default_args();
    args.browser = Some("firefox-latest".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    assert!(matches!(
        config.browser_emulation,
        rdlp_api::BrowserEmulation::FirefoxLatest
    ));
}

#[test]
fn test_merge_config_browser_cli_flag_pinned() {
    let mut args = default_args();
    args.browser = Some("chrome-137".to_string());

    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");

    match &config.browser_emulation {
        rdlp_api::BrowserEmulation::Pinned(s) => assert_eq!(s, "chrome-137"),
        other => panic!("expected Pinned, got {other:?}"),
    }
}

#[test]
fn test_merge_config_browser_env_fallback() {
    // CLI flag absent; env var should drive the value. temp-env's
    // singleton mutex serialises against the default-test reader.
    temp_env::with_var("RDLP_BROWSER_EMULATION", Some("safari-latest"), || {
        let args = default_args();
        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");
        assert!(matches!(
            config.browser_emulation,
            rdlp_api::BrowserEmulation::SafariLatest
        ));
    });
}

// === Proxy hard-fail negatives ===
//
// `--proxy` must be validated at config-build time. A user passing
// `--proxy http://192.168.1.1:8080` expecting traffic routing must NOT
// silently get a direct connection: the merge must error out so they
// know their proxy was rejected.

#[test]
fn rejects_proxy_pointing_at_loopback() {
    let mut args = default_args();
    args.proxy = Some("http://127.0.0.1:3128".into());
    let result = merge_config(&args, Config::default(), no_interactive());
    let err = result.expect_err("loopback proxy MUST be rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("--proxy validation failed"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn rejects_proxy_pointing_at_rfc1918() {
    let mut args = default_args();
    args.proxy = Some("http://192.168.1.1:8080".into());
    assert!(merge_config(&args, Config::default(), no_interactive()).is_err());
}

#[test]
fn rejects_proxy_with_unsupported_scheme() {
    let mut args = default_args();
    args.proxy = Some("ftp://proxy.example.com:21".into());
    assert!(merge_config(&args, Config::default(), no_interactive()).is_err());
}

#[test]
fn accepts_proxy_at_public_https_endpoint() {
    let mut args = default_args();
    args.proxy = Some("https://corp-proxy.example.com:443".into());
    let cfg = merge_config(&args, Config::default(), no_interactive())
        .expect("public proxy must be accepted");
    assert_eq!(
        cfg.proxy.as_deref(),
        Some("https://corp-proxy.example.com:443")
    );
}

// === Network timeout CLI tests (#278) ===

#[test]
fn cli_socket_timeout_flag_sets_field() {
    use clap::Parser;
    let args = Args::try_parse_from(["rdlp", "--socket-timeout", "45", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(args.socket_timeout, Some(45));
}

#[test]
fn cli_read_timeout_flag_sets_field() {
    use clap::Parser;
    let args = Args::try_parse_from(["rdlp", "--read-timeout", "120", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(args.read_timeout, Some(120));
}

#[test]
fn cli_pool_idle_timeout_flag_accepts_zero_sentinel() {
    use clap::Parser;
    let args = Args::try_parse_from(["rdlp", "--pool-idle-timeout", "0", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(args.pool_idle_timeout, Some(0));
}

#[test]
fn cli_pool_idle_timeout_flag_accepts_positive() {
    use clap::Parser;
    let args = Args::try_parse_from([
        "rdlp",
        "--pool-idle-timeout",
        "300",
        "https://example.com/x",
    ])
    .expect("parse should succeed");
    assert_eq!(args.pool_idle_timeout, Some(300));
}

#[test]
fn cli_timeout_flags_unset_default_to_none() {
    use clap::Parser;
    let args =
        Args::try_parse_from(["rdlp", "https://example.com/x"]).expect("parse should succeed");
    assert!(args.socket_timeout.is_none());
    assert!(args.read_timeout.is_none());
    assert!(args.pool_idle_timeout.is_none());
}

#[test]
fn cli_timeout_flags_merge_into_config() {
    let mut args = default_args();
    args.socket_timeout = Some(45);
    args.read_timeout = Some(200);
    args.pool_idle_timeout = Some(0);

    let cfg = merge_config(&args, Config::default(), no_interactive()).expect("merge");
    assert_eq!(cfg.socket_timeout, Some(45));
    assert_eq!(cfg.read_timeout, Some(200));
    assert_eq!(cfg.pool_idle_timeout, Some(0));
}

#[test]
fn cli_timeout_flags_unset_preserve_config() {
    let args = default_args();
    let file_config = Config {
        read_timeout: Some(77),
        pool_idle_timeout: Some(88),
        ..Config::default()
    };
    let cfg = merge_config(&args, file_config, no_interactive()).expect("merge");
    assert_eq!(cfg.read_timeout, Some(77));
    assert_eq!(cfg.pool_idle_timeout, Some(88));
}

#[test]
fn cli_socket_timeout_zero_is_rejected_by_validate() {
    let mut args = default_args();
    args.socket_timeout = Some(0);
    let res = merge_config(&args, Config::default(), no_interactive());
    // merge_config calls Config::validate() at the end, so this should be Err.
    assert!(
        res.is_err(),
        "Config::validate must reject socket_timeout=0"
    );
}

#[test]
fn cli_pool_idle_timeout_above_max_is_rejected_by_validate() {
    let mut args = default_args();
    args.pool_idle_timeout = Some(9999);
    let res = merge_config(&args, Config::default(), no_interactive());
    assert!(
        res.is_err(),
        "Config::validate must reject pool_idle_timeout > 3600"
    );
}
