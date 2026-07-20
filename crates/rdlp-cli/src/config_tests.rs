//! Tests for configuration building and merging.

use super::*;
use crate::args::Args;
use rdlp_api::{AudioFormat, Config, FixupPolicy, SubtitleFormat};

/// Helper: create default Args for testing (all fields at defaults).
pub fn default_args() -> Args {
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
        recode_audio: None,
        recode_threads: None,
        recode_preset: None,
        recode_deadline: None,
        recode_cpu_used: None,
        recode_speed_level: None,
        fixup: None,
        match_filter: vec![],
        browser: None,
        trust_publisher: vec![],
        plugin: None,
        download_timeout: None,
        merge_timeout: None,
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
fn cli_download_timeout_flag_sets_field() {
    use clap::Parser;
    let args = Args::try_parse_from([
        "rdlp",
        "--download-timeout",
        "7200",
        "https://example.com/x",
    ])
    .expect("parse should succeed");
    assert_eq!(args.download_timeout, Some(7200));
}

#[test]
fn cli_merge_timeout_flag_sets_field() {
    use clap::Parser;
    let args = Args::try_parse_from(["rdlp", "--merge-timeout", "600", "https://example.com/x"])
        .expect("parse should succeed");
    assert_eq!(args.merge_timeout, Some(600));
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
    // Seed all three with NON-default values so a regression that overwrites
    // a field with `Config::default()` would no longer pass by coincidence.
    // `Config::default().socket_timeout` is `Some(30)`, hence 99 here.
    let file_config = Config {
        socket_timeout: Some(99),
        read_timeout: Some(77),
        pool_idle_timeout: Some(88),
        ..Config::default()
    };
    let cfg = merge_config(&args, file_config, no_interactive()).expect("merge");
    assert_eq!(cfg.socket_timeout, Some(99));
    assert_eq!(cfg.read_timeout, Some(77));
    assert_eq!(cfg.pool_idle_timeout, Some(88));
}

#[test]
fn cli_socket_timeout_zero_is_rejected_by_validate() {
    let mut args = default_args();
    args.socket_timeout = Some(0);
    let err = merge_config(&args, Config::default(), no_interactive())
        .expect_err("Config::validate must reject socket_timeout=0");
    let msg = format!("{err:#}");
    // ConfigValidationError::OutOfRange { field, .. } renders as "{field}: {reason}";
    // pin the field name so an unrelated validation triggering the same Err
    // would no longer make this test pass silently.
    assert!(
        msg.contains("socket_timeout"),
        "rejection should cite socket_timeout, got: {msg}"
    );
}

#[test]
fn cli_pool_idle_timeout_above_max_is_rejected_by_validate() {
    let mut args = default_args();
    args.pool_idle_timeout = Some(9999);
    let err = merge_config(&args, Config::default(), no_interactive())
        .expect_err("Config::validate must reject pool_idle_timeout > 3600");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("pool_idle_timeout"),
        "rejection should cite pool_idle_timeout, got: {msg}"
    );
}

#[test]
fn recode_threads_and_preset_flags_map_to_config() {
    use clap::Parser;
    let args = Args::try_parse_from([
        "rdlp",
        "--recode-video=mkv",
        "--recode-threads",
        "6",
        "--recode-preset",
        "faster",
        "https://example.com/v",
    ])
    .expect("args parse");
    let config = build_config(&args).expect("config builds");
    assert_eq!(config.postprocess.recode_threads, Some(6));
    assert_eq!(config.postprocess.recode_preset.as_deref(), Some("faster"));
}

#[test]
fn speed_control_flags_map_to_config() {
    use clap::Parser;
    // VP9 supports deadline + cpu_used; speed_level is for libxavs2 only.
    // Test that all three fields round-trip through config correctly by
    // using a valid per-encoder combination: VP9 with deadline + cpu_used,
    // then verify speed_level maps when set independently without an encoder.
    let args = Args::try_parse_from([
        "rdlp",
        "--recode-video=webm",
        "--video-encoder=libvpx-vp9",
        "--recode-deadline=good",
        "--recode-cpu-used=-4",
        "https://example.com/v",
    ])
    .expect("args parse");
    let config = build_config(&args).expect("config builds");
    assert_eq!(
        config.postprocess.recode_deadline,
        Some(rdlp_types::VpxDeadline::Good)
    );
    assert_eq!(config.postprocess.recode_cpu_used, Some(-4));

    // speed_level field mapping: verify it reaches config when no encoder
    // is resolved (validator is a no-op when encoder is None).
    let mut args2 = default_args();
    args2.recode_speed_level = Some(3);
    let config2 = merge_config(&args2, rdlp_api::Config::default(), no_interactive())
        .expect("config2 builds");
    assert_eq!(config2.postprocess.recode_speed_level, Some(3));
}

#[test]
fn negative_cpu_used_parses_both_forms() {
    use clap::Parser;
    assert_eq!(
        Args::parse_from(["rdlp", "--recode-cpu-used=-8", "URL"]).recode_cpu_used,
        Some(-8)
    );
    assert_eq!(
        Args::parse_from(["rdlp", "--recode-cpu-used", "-8", "URL"]).recode_cpu_used,
        Some(-8)
    );
}

/// `--recode-audio` must accept `copy`/`auto` in any case, like every
/// neighbouring format flag (#540).
///
/// Before the fix the match was case-sensitive, so `--recode-audio=COPY`
/// silently became `RecodeAudioMode::Encoder { name: "COPY" }` and failed much
/// later inside `FFmpeg` with a confusing "unknown encoder" error, rather than
/// being recognised as the stream-copy mode the user asked for.
#[test]
fn test_merge_config_recode_audio_mode_is_case_insensitive() {
    for spelling in ["copy", "COPY", "Copy", "cOpY"] {
        let mut args = default_args();
        args.recode_audio = Some(spelling.to_string());
        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");
        assert_eq!(
            config.postprocess.recode_audio,
            RecodeAudioMode::Copy,
            "--recode-audio={spelling} must mean Copy"
        );
    }

    for spelling in ["auto", "AUTO", "Auto"] {
        let mut args = default_args();
        args.recode_audio = Some(spelling.to_string());
        let config =
            merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");
        assert_eq!(
            config.postprocess.recode_audio,
            RecodeAudioMode::Auto,
            "--recode-audio={spelling} must mean Auto"
        );
    }
}

/// An explicit encoder name must survive verbatim — case-folding the mode
/// keywords must not case-fold the encoder name, which `FFmpeg` matches exactly.
#[test]
fn test_merge_config_recode_audio_encoder_name_preserved() {
    let mut args = default_args();
    args.recode_audio = Some("libOpus".to_string());
    let config =
        merge_config(&args, Config::default(), no_interactive()).expect("merge should succeed");
    assert_eq!(
        config.postprocess.recode_audio,
        RecodeAudioMode::Encoder {
            name: "libOpus".to_string()
        },
        "encoder names are passed to FFmpeg verbatim and must not be lowercased"
    );
}

/// A `recode_audio` set in `config.toml` must survive when the flag is omitted.
///
/// The arg carried `default_value = "copy"` and was assigned unconditionally,
/// so the clap default silently overwrote the config file's value on every run
/// — the user's setting was discarded with no error. The adjacent `fixup` arg
/// guarded on its default, which avoided *this* direction but broke the other
/// one (#583); both now use the same `Option` treatment.
#[test]
fn test_merge_config_file_recode_audio_survives_when_flag_omitted() {
    let args = default_args(); // no --recode-audio passed
    let mut file_config = Config::default();
    file_config.postprocess.recode_audio = RecodeAudioMode::Encoder {
        name: "libopus".to_string(),
    };

    let merged = merge_config(&args, file_config, no_interactive()).expect("merge should succeed");

    assert_eq!(
        merged.postprocess.recode_audio,
        RecodeAudioMode::Encoder {
            name: "libopus".to_string()
        },
        "config.toml's recode_audio must not be clobbered by the CLI default"
    );
}

/// An explicit `--fixup=detect_or_warn` must beat a config-file `fixup`.
///
/// `fixup` carried `default_value = "detect_or_warn"` and was merged behind
/// `if args.fixup != "detect_or_warn"`, so the one value a user could not
/// express on the CLI was the default itself: passing it explicitly was
/// byte-identical to not passing the flag, and `fixup = "never"` in
/// `config.toml` silently won (#583). This is the mirror image of the
/// `recode_audio` clobber above — same class, opposite direction.
#[test]
fn test_merge_config_explicit_fixup_overrides_file() {
    let mut args = default_args();
    args.fixup = Some("detect_or_warn".to_string());
    let mut file_config = Config::default();
    file_config.postprocess.fixup = FixupPolicy::Never;

    let merged = merge_config(&args, file_config, no_interactive()).expect("merge should succeed");

    assert_eq!(
        merged.postprocess.fixup,
        FixupPolicy::DetectOrWarn,
        "an explicitly passed --fixup must win over config.toml, even when the \
         value happens to equal the documented default"
    );
}

/// A `fixup` set in `config.toml` must survive when the flag is omitted.
///
/// The direction the old sentinel guard got right — pinned so the #583 fix
/// cannot regress it. Dropping `default_value` without guarding on `Option`
/// would break exactly this.
#[test]
fn test_merge_config_file_fixup_survives_when_flag_omitted() {
    let args = default_args(); // no --fixup passed
    let mut file_config = Config::default();
    file_config.postprocess.fixup = FixupPolicy::Never;

    let merged = merge_config(&args, file_config, no_interactive()).expect("merge should succeed");

    assert_eq!(
        merged.postprocess.fixup,
        FixupPolicy::Never,
        "config.toml's fixup must not be clobbered by the CLI default"
    );
}

/// The flag must still win when it IS passed — including when the user
/// explicitly asks for the same value as the old default.
///
/// A naive `if args.recode_audio != "copy"` guard would fix the clobber but
/// break this: an explicit `--recode-audio=copy` would be indistinguishable
/// from the default and the config file would wrongly win.
#[test]
fn test_merge_config_explicit_recode_audio_overrides_file() {
    let mut file_config = Config::default();
    file_config.postprocess.recode_audio = RecodeAudioMode::Encoder {
        name: "libopus".to_string(),
    };

    let mut args = default_args();
    args.recode_audio = Some("copy".to_string());
    let merged =
        merge_config(&args, file_config.clone(), no_interactive()).expect("merge should succeed");
    assert_eq!(
        merged.postprocess.recode_audio,
        RecodeAudioMode::Copy,
        "an explicit --recode-audio=copy must override the config file"
    );

    let mut args = default_args();
    args.recode_audio = Some("libfdk_aac".to_string());
    let merged = merge_config(&args, file_config, no_interactive()).expect("merge should succeed");
    assert_eq!(
        merged.postprocess.recode_audio,
        RecodeAudioMode::Encoder {
            name: "libfdk_aac".to_string()
        },
        "an explicit encoder must override the config file"
    );
}

/// Optional CLI args must not overwrite config-file values when omitted.
///
/// `recode_cpu_used` and `recode_speed_level` were assigned unconditionally, so
/// running with no flags wrote `None` over whatever the config file had set —
/// silently discarding the user's tuning. Their immediate neighbours
/// (`recode_threads`, `recode_preset`) guard correctly, which is what made the
/// omission easy to miss (#540).
#[test]
fn test_merge_config_optional_args_do_not_clobber_config_file() {
    let args = default_args(); // no flags passed
    let mut file_config = Config::default();
    file_config.postprocess.recode_cpu_used = Some(4);
    file_config.postprocess.recode_speed_level = Some(3);
    file_config.postprocess.recode_threads = Some(8);
    file_config.postprocess.recode_preset = Some("slow".to_string());

    let merged = merge_config(&args, file_config, no_interactive()).expect("merge should succeed");

    assert_eq!(
        merged.postprocess.recode_cpu_used,
        Some(4),
        "config.toml's recode_cpu_used must survive an invocation without the flag"
    );
    assert_eq!(
        merged.postprocess.recode_speed_level,
        Some(3),
        "config.toml's recode_speed_level must survive an invocation without the flag"
    );
    assert_eq!(merged.postprocess.recode_threads, Some(8));
    assert_eq!(merged.postprocess.recode_preset, Some("slow".to_string()));
}

/// The flag must still win when passed — the guard must not make config-file
/// values sticky.
#[test]
fn test_merge_config_optional_args_still_override_when_passed() {
    let mut file_config = Config::default();
    file_config.postprocess.recode_cpu_used = Some(4);
    file_config.postprocess.recode_speed_level = Some(3);

    let mut args = default_args();
    args.recode_cpu_used = Some(-8);
    args.recode_speed_level = Some(7);

    let merged = merge_config(&args, file_config, no_interactive()).expect("merge should succeed");

    assert_eq!(merged.postprocess.recode_cpu_used, Some(-8));
    assert_eq!(merged.postprocess.recode_speed_level, Some(7));
}

/// Empty and whitespace-only values are rejected by clap, for every string- and
/// path-valued arg — not just the one where the problem was first noticed.
///
/// A blank value otherwise reaches the domain layer as a real-looking value:
/// `--recode-audio=` became `Encoder { name: "" }` and failed inside `FFmpeg`
/// after the download completed. Validating at the parse boundary means clap
/// reports it with the flag name, the offending value and usage text (#540).
#[test]
fn test_blank_values_are_rejected_at_the_parse_boundary() {
    use clap::Parser;

    let blank_forms = ["", "   ", "\t"];
    let string_flags = [
        "--recode-audio",
        "--format",
        "--output",
        "--audio-quality",
        "--sub-langs",
        "--video-encoder",
        "--recode-preset",
        "--proxy",
        "--limit-rate",
        "--search",
    ];
    for flag in string_flags {
        for blank in blank_forms {
            let result = Args::try_parse_from(["rdlp", &format!("{flag}={blank}"), "URL"]);
            assert!(
                result.is_err(),
                "{flag}={blank:?} must be rejected at parse time"
            );
        }
    }

    // Path-valued args need the same rule — a blank path is equally meaningless.
    for flag in ["--cookies", "--download-archive", "--ffmpeg-location", "-P"] {
        for blank in blank_forms {
            let result = Args::try_parse_from(["rdlp", &format!("{flag}={blank}"), "URL"]);
            assert!(
                result.is_err(),
                "{flag}={blank:?} must be rejected at parse time"
            );
        }
    }
}

/// The rejection must name the flag and the offending value, which is what
/// clap's `ValueValidation` error shape provides.
#[test]
fn test_blank_rejection_message_names_flag_and_value() {
    use clap::Parser;

    let Err(err) = Args::try_parse_from(["rdlp", "--recode-audio=   ", "URL"]) else {
        panic!("blank must be rejected");
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("--recode-audio"),
        "must name the flag; got: {rendered}"
    );
    assert!(
        rendered.contains("empty or whitespace-only"),
        "must explain what was wrong; got: {rendered}"
    );
}

/// Ordinary values must still parse — the guard must not reject real input.
#[test]
fn test_non_blank_values_still_parse() {
    use clap::Parser;

    let Ok(args) = Args::try_parse_from([
        "rdlp",
        "--recode-audio=libOpus",
        "--format=bv+ba",
        "--cookies=/tmp/c.txt",
        "URL",
    ]) else {
        panic!("ordinary values must parse");
    };
    assert_eq!(args.recode_audio.as_deref(), Some("libOpus"));
    assert_eq!(args.format.as_deref(), Some("bv+ba"));
}

/// The bare-flag forms must survive the blank-value parser.
///
/// `--audio-format`, `--recode-video` and `--remux` use
/// `default_missing_value = "interactive"`, so the value clap substitutes also
/// flows through `value_parser`. That value is non-blank, but the interaction
/// is worth pinning: breaking it would silently disable interactive selection.
#[test]
fn test_bare_interactive_flags_still_parse() {
    use clap::Parser;

    for (flag, get) in [
        ("--audio-format", 0usize),
        ("--recode-video", 1),
        ("--remux", 2),
    ] {
        let Ok(args) = Args::try_parse_from(["rdlp", flag, "URL"]) else {
            panic!("{flag} with no value must still parse");
        };
        let actual = match get {
            0 => args.audio_format.clone(),
            1 => args.recode_video.clone(),
            _ => args.remux.clone(),
        };
        assert_eq!(
            actual.as_deref(),
            Some("interactive"),
            "{flag} with no value must yield the interactive sentinel"
        );
    }
}

/// The same three flags must still REJECT an explicitly blank value.
///
/// `require_equals` + `default_missing_value` means bare `--remux` and
/// `--remux=` take different paths through clap: the first substitutes the
/// sentinel, the second supplies an empty value that must reach the parser.
/// Pinning only the bare form would leave the rejection untested.
#[test]
fn test_interactive_flags_still_reject_an_explicit_blank() {
    use clap::Parser;

    for flag in ["--audio-format", "--recode-video", "--remux"] {
        for blank in ["", "   "] {
            let result = Args::try_parse_from(["rdlp", &format!("{flag}={blank}"), "URL"]);
            assert!(
                result.is_err(),
                "{flag}={blank:?} must be rejected even though the bare flag is valid"
            );
        }
    }
}

/// Compile-time canary: adding a field to `Config` or `PostProcess` breaks
/// this until someone decides whether `merge_config` should merge it.
///
/// A struct pattern without `..` must name every field (Rust Reference,
/// Patterns), so a new field is `E0027: pattern does not mention field`.
///
/// DO NOT ADD `..` TO THESE PATTERNS. rustc's own E0027 help suggests it
/// ("or always ignore missing fields here") and taking that suggestion
/// silently disables this guarantee forever. The correct fix for E0027 here
/// is to classify the new field: add it to `merge_fields!` if the CLI should
/// merge it, or to the exceptions block if it needs bespoke handling, then
/// name it below. `scripts/check-merge-exhaustive.sh` enforces the no-`..`
/// rule.
#[test]
fn every_config_field_is_classified() {
    let rdlp_api::Config {
        output_to_stdout: _,
        output_template: _,
        output_directory: _,
        restrict_filenames: _,
        overwrite: _,
        continue_downloads: _,
        no_part: _,
        format: _,
        audio_multistreams: _,
        concurrent_fragments: _,
        rate_limit: _,
        retries: _,
        fragment_retries: _,
        retry_initial_delay_ms: _,
        retry_max_delay_ms: _,
        retry_backoff_multiplier: _,
        buffer_size: _,
        proxy: _,
        socket_timeout: _,
        read_timeout: _,
        pool_idle_timeout: _,
        download_timeout: _,
        merge_timeout: _,
        hls_head_probe_timeout: _,
        parallel_threshold: _,
        source_address: _,
        user_agent: _,
        browser_emulation: _,
        http_headers: _,
        write_auto_subtitles: _,
        subtitle_langs: _,
        subtitle_format: _,
        list_subs: _,
        strict_subs: _,
        verify_sub_urls: _,
        retry_subs: _,
        quiet: _,
        verbose: _,
        simulate: _,
        skip_download: _,
        extract_playlist: _,
        playlist_start: _,
        playlist_end: _,
        playlist_items: _,
        username: _,
        password: _,
        two_factor: _,
        netrc: _,
        cookies_from_browser: _,
        cookies_file: _,
        download_archive: _,
        match_filters: _,
        plugin_directories: _,
        enabled_plugins: _,
        load_plugins: _,
        plugin_timeout_metadata_ms: _,
        plugin_timeout_extract_s: _,
        plugin_timeout_search_s: _,
        plugin_memory_limit_mb: _,
        plugin_stack_limit_mb: _,
        plugin_trusted_publishers: _,
        adaptive_downloads: _,
        postprocess,
    } = Config::default();

    let rdlp_api::PostProcess {
        extract_audio: _,
        audio_format: _,
        audio_quality: _,
        recode_video: _,
        remux_container: _,
        merge_output_format: _,
        embed_thumbnail: _,
        write_thumbnail: _,
        embed_metadata: _,
        embed_subtitles: _,
        write_subtitles: _,
        keep_video: _,
        ffmpeg_location: _,
        ffmpeg_args: _,
        normalize_audio: _,
        loudnorm: _,
        audio_gain_target: _,
        loudnorm_preset: _,
        loudnorm_target_i: _,
        loudnorm_target_tp: _,
        loudnorm_target_lra: _,
        loudnorm_dynamic: _,
        loudnorm_precompress: _,
        normalize_boost: _,
        normalize_boost_db: _,
        video_encoder: _,
        recode_audio: _,
        recode_container: _,
        recode_threads: _,
        recode_preset: _,
        recode_deadline: _,
        recode_cpu_used: _,
        recode_speed_level: _,
        fixup: _,
    } = postprocess;
}
