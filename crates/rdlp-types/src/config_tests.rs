//! Tests for configuration types and validation.
#![allow(clippy::field_reassign_with_default)]

use super::*;
use crate::container::ContainerFormat;
use std::ops::RangeInclusive;

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.output_template, "%(title|Unknown)s [%(id)s].%(ext)s");
    assert!(config.format.is_none());
    assert!(config.continue_downloads);
    assert_eq!(
        config.concurrent_fragments,
        crate::EffectiveNetwork::DEFAULT.concurrent_fragments
    );
}

#[test]
fn test_default_format_is_none() {
    let config = Config::default();
    assert!(
        config.format.is_none(),
        "Default format should be None (dynamic default)"
    );
}

#[test]
fn test_default_audio_multistreams_is_false() {
    let config = Config::default();
    assert!(!config.audio_multistreams);
}

#[test]
fn test_validate_config() {
    let mut config = Config::default();

    // Valid config
    assert!(config.validate().is_ok());

    // Invalid concurrent_fragments
    config.concurrent_fragments = 0;
    assert!(config.validate().is_err());

    // Fix and test buffer_size
    config.concurrent_fragments = 8;
    config.buffer_size = 0;
    assert!(config.validate().is_err());

    // Fix and test playlist
    config.buffer_size = 1024;
    config.playlist_start = 10;
    config.playlist_end = Some(5);
    assert!(config.validate().is_err());
}

#[test]
fn test_default_output_to_stdout_is_false() {
    let config = Config::default();
    assert!(!config.output_to_stdout);
}

#[test]
fn test_stdout_valid_when_no_postprocessing() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    config.postprocess.embed_thumbnail = false;
    assert!(config.validate().is_ok());
}

#[test]
fn test_stdout_rejects_extract_audio() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    config.postprocess.embed_thumbnail = false;
    config.postprocess.extract_audio = true;
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::StdoutIncompatible {
            option: "extract-audio".to_string()
        }
    );
}

#[test]
fn test_stdout_rejects_remux() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    config.postprocess.embed_thumbnail = false;
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::StdoutIncompatible {
            option: "remux".to_string()
        }
    );
}

#[test]
fn test_stdout_rejects_embed_thumbnail() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    // embed_thumbnail defaults to true, which should fail
    assert!(config.postprocess.embed_thumbnail);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::StdoutIncompatible {
            option: "embed-thumbnail".to_string()
        }
    );
}

#[test]
fn test_stdout_rejects_embed_metadata() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    config.postprocess.embed_thumbnail = false;
    config.postprocess.embed_metadata = true;
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::StdoutIncompatible {
            option: "embed-metadata".to_string()
        }
    );
}

#[test]
fn test_stdout_rejects_normalize_audio() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    config.postprocess.embed_thumbnail = false;
    config.postprocess.normalize_audio = true;
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::StdoutIncompatible {
            option: "normalize-audio".to_string()
        }
    );
}

#[test]
fn test_stdout_rejects_normalize_boost() {
    let mut config = Config::default();
    config.output_to_stdout = true;
    config.postprocess.embed_thumbnail = false;
    config.postprocess.normalize_boost = true;
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::StdoutIncompatible {
            option: "normalize-boost".to_string()
        }
    );
}

#[test]
fn test_stdout_incompatible_error_display() {
    let err = ConfigValidationError::StdoutIncompatible {
        option: "remux".to_string(),
    };
    assert_eq!(
        err.to_string(),
        "--remux is not compatible with -o - (stdout output)"
    );
}

#[test]
fn test_postprocess_embedded_in_config() {
    let config = Config::default();
    assert!(config.postprocess.embed_thumbnail);
    assert!(!config.postprocess.extract_audio);
    assert_eq!(
        config.postprocess.fixup,
        crate::fixup_policy::FixupPolicy::DetectOrWarn
    );
}

// === Plugin timeout / resource limit validation ===

#[test]
fn test_plugin_defaults_pass_validation() {
    let config = Config::default();
    assert!(config.plugin_timeout_metadata_ms.is_none());
    assert!(config.plugin_timeout_extract_s.is_none());
    assert!(config.plugin_timeout_search_s.is_none());
    assert!(config.plugin_memory_limit_mb.is_none());
    assert!(config.plugin_stack_limit_mb.is_none());
    assert!(config.plugin_trusted_publishers.is_empty());
    assert!(config.validate().is_ok());
}

#[test]
fn test_plugin_timeout_extract_too_large() {
    let mut config = Config::default();
    config.plugin_timeout_extract_s = Some(700);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_timeout_extract_s",
            reason: "must be <= 600 (10 min ceiling)",
        }
    );
}

#[test]
fn test_plugin_timeout_extract_at_ceiling_passes() {
    let mut config = Config::default();
    config.plugin_timeout_extract_s = Some(600);
    assert!(config.validate().is_ok());
}

#[test]
fn test_plugin_timeout_search_too_large() {
    let mut config = Config::default();
    config.plugin_timeout_search_s = Some(601);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_timeout_search_s",
            reason: "must be <= 600",
        }
    );
}

#[test]
fn test_plugin_timeout_metadata_too_large() {
    let mut config = Config::default();
    config.plugin_timeout_metadata_ms = Some(60_001);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_timeout_metadata_ms",
            reason: "must be <= 60000ms (60 sec ceiling)",
        }
    );
}

#[test]
fn test_plugin_memory_limit_zero_fails() {
    let mut config = Config::default();
    config.plugin_memory_limit_mb = Some(0);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_memory_limit_mb",
            reason: "must be 1..=1024 MB",
        }
    );
}

#[test]
fn test_plugin_memory_limit_too_large() {
    let mut config = Config::default();
    config.plugin_memory_limit_mb = Some(2000);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_memory_limit_mb",
            reason: "must be 1..=1024 MB",
        }
    );
}

#[test]
fn test_plugin_memory_limit_at_bounds_passes() {
    let mut config = Config::default();
    config.plugin_memory_limit_mb = Some(1);
    assert!(config.validate().is_ok());
    config.plugin_memory_limit_mb = Some(1024);
    assert!(config.validate().is_ok());
}

#[test]
fn test_plugin_stack_limit_zero_fails() {
    let mut config = Config::default();
    config.plugin_stack_limit_mb = Some(0);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_stack_limit_mb",
            reason: "must be 1..=64 MB",
        }
    );
}

#[test]
fn test_plugin_stack_limit_too_large() {
    let mut config = Config::default();
    config.plugin_stack_limit_mb = Some(65);
    let err = config.validate().unwrap_err();
    assert_eq!(
        err,
        ConfigValidationError::OutOfRange {
            field: "plugin_stack_limit_mb",
            reason: "must be 1..=64 MB",
        }
    );
}

#[test]
fn test_plugin_stack_limit_at_bounds_passes() {
    let mut config = Config::default();
    config.plugin_stack_limit_mb = Some(1);
    assert!(config.validate().is_ok());
    config.plugin_stack_limit_mb = Some(64);
    assert!(config.validate().is_ok());
}

#[test]
fn test_plugin_trusted_publishers_toml_roundtrip() {
    let mut config = Config::default();
    config.plugin_trusted_publishers = vec![
        "sigstore:github:user/repo".to_string(),
        "ed25519:deadbeef".to_string(),
    ];
    let toml_str = toml::to_string(&config).expect("serialization failed");
    let roundtripped: Config = toml::from_str(&toml_str).expect("deserialization failed");
    assert_eq!(
        roundtripped.plugin_trusted_publishers,
        config.plugin_trusted_publishers
    );
}

#[test]
fn test_out_of_range_error_display() {
    let err = ConfigValidationError::OutOfRange {
        field: "plugin_memory_limit_mb",
        reason: "must be 1..=1024 MB",
    };
    assert_eq!(
        err.to_string(),
        "plugin_memory_limit_mb: must be 1..=1024 MB"
    );
}

#[test]
fn read_timeout_accepts_valid_range() {
    for v in [1u64, 60, 600] {
        let cfg = Config {
            read_timeout: Some(v),
            ..Config::default()
        };
        assert!(cfg.validate().is_ok(), "read_timeout={v} should be valid");
    }
}

#[test]
fn read_timeout_rejects_zero() {
    let cfg = Config {
        read_timeout: Some(0),
        ..Config::default()
    };
    assert!(matches!(
        cfg.validate(),
        Err(ConfigValidationError::OutOfRange {
            field: "read_timeout",
            ..
        })
    ));
}

#[test]
fn read_timeout_rejects_above_600() {
    let cfg = Config {
        read_timeout: Some(601),
        ..Config::default()
    };
    assert!(matches!(
        cfg.validate(),
        Err(ConfigValidationError::OutOfRange {
            field: "read_timeout",
            ..
        })
    ));
}

#[test]
fn pool_idle_timeout_accepts_zero_and_max() {
    for v in [0u64, 90, 3600] {
        let cfg = Config {
            pool_idle_timeout: Some(v),
            ..Config::default()
        };
        assert!(
            cfg.validate().is_ok(),
            "pool_idle_timeout={v} should be valid"
        );
    }
}

#[test]
fn pool_idle_timeout_rejects_above_3600() {
    let cfg = Config {
        pool_idle_timeout: Some(3601),
        ..Config::default()
    };
    assert!(matches!(
        cfg.validate(),
        Err(ConfigValidationError::OutOfRange {
            field: "pool_idle_timeout",
            ..
        })
    ));
}

#[test]
fn download_timeout_accepts_valid_range_and_rejects_out_of_range() {
    for v in [1u64, 3600, 86400] {
        let cfg = Config {
            download_timeout: Some(v),
            ..Config::default()
        };
        assert!(
            cfg.validate().is_ok(),
            "download_timeout={v} should be valid"
        );
    }
    for v in [0u64, 86401] {
        let cfg = Config {
            download_timeout: Some(v),
            ..Config::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigValidationError::OutOfRange {
                field: "download_timeout",
                ..
            })
        ));
    }
}

#[test]
fn merge_timeout_accepts_valid_range_and_rejects_out_of_range() {
    for v in [1u64, 1800, 86400] {
        let cfg = Config {
            merge_timeout: Some(v),
            ..Config::default()
        };
        assert!(cfg.validate().is_ok(), "merge_timeout={v} should be valid");
    }
    for v in [0u64, 86401] {
        let cfg = Config {
            merge_timeout: Some(v),
            ..Config::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigValidationError::OutOfRange {
                field: "merge_timeout",
                ..
            })
        ));
    }
}

#[test]
fn socket_timeout_accepts_valid_range() {
    for v in [1u64, 30, 300] {
        let cfg = Config {
            socket_timeout: Some(v),
            ..Config::default()
        };
        assert!(cfg.validate().is_ok(), "socket_timeout={v} should be valid");
    }
}

#[test]
fn socket_timeout_rejects_zero() {
    let cfg = Config {
        socket_timeout: Some(0),
        ..Config::default()
    };
    assert!(matches!(
        cfg.validate(),
        Err(ConfigValidationError::OutOfRange {
            field: "socket_timeout",
            ..
        })
    ));
}

#[test]
fn socket_timeout_rejects_above_300() {
    let cfg = Config {
        socket_timeout: Some(301),
        ..Config::default()
    };
    assert!(matches!(
        cfg.validate(),
        Err(ConfigValidationError::OutOfRange {
            field: "socket_timeout",
            ..
        })
    ));
}

#[test]
fn hls_head_probe_timeout_default_is_some_5() {
    let c = Config::default();
    assert_eq!(c.hls_head_probe_timeout, Some(5));
}

#[test]
fn hls_head_probe_timeout_zero_rejected() {
    let c = Config {
        hls_head_probe_timeout: Some(0),
        ..Config::default()
    };
    let err = c.validate().expect_err("must reject");
    assert!(format!("{err:#}").contains("hls_head_probe_timeout"));
}

#[test]
fn hls_head_probe_timeout_above_max_rejected() {
    let c = Config {
        hls_head_probe_timeout: Some(301),
        ..Config::default()
    };
    let err = c.validate().expect_err("must reject");
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "hls_head_probe_timeout",
            ..
        }
    ));
}

#[test]
fn hls_expansion_timeout_default_is_none() {
    // Unset keeps rdlp-api's `DEFAULT_HLS_EXPANSION_TIMEOUT_SECS`, the same
    // "None = consumer default" shape `download_timeout`/`merge_timeout` use.
    assert_eq!(Config::default().hls_expansion_timeout, None);
}

#[test]
fn hls_expansion_timeout_boundaries() {
    // Inclusive at both ends: 1 and 600 pass, 0 and 601 are rejected.
    for accepted in [1, 600] {
        Config {
            hls_expansion_timeout: Some(accepted),
            ..Config::default()
        }
        .validate()
        .unwrap_or_else(|e| panic!("{accepted} must be accepted: {e}"));
    }
    for rejected in [0, 601] {
        let err = Config {
            hls_expansion_timeout: Some(rejected),
            ..Config::default()
        }
        .validate()
        .expect_err("out of range must reject");
        assert!(
            matches!(
                err,
                ConfigValidationError::OutOfRange {
                    field: "hls_expansion_timeout",
                    ..
                }
            ),
            "{rejected}: got {err:?}"
        );
    }
}

/// Asserts a `usize`/`u64` `Config` field's inclusive validation boundary
/// `range`: both ends accepted, one below and one above rejected with
/// `ConfigValidationError::OutOfRange { field, reason }` whose `reason`
/// cites the upper bound's value (the named constant, not a restated
/// literal). Mirrors `hls_expansion_timeout_boundaries` above, generalized
/// so the five playlist/metadata fields don't each hand-roll the same
/// assertions.
fn assert_usize_field_boundaries(
    field: &'static str,
    range: RangeInclusive<usize>,
    with_value: impl Fn(Option<usize>) -> Config,
) {
    let (min, max) = (*range.start(), *range.end());
    for accepted in [min, max] {
        with_value(Some(accepted))
            .validate()
            .unwrap_or_else(|e| panic!("{field}={accepted} must be accepted: {e}"));
    }
    for rejected in [min - 1, max + 1] {
        let err = with_value(Some(rejected))
            .validate()
            .expect_err("out of range must reject");
        match err {
            ConfigValidationError::OutOfRange { field: f, reason } if f == field => assert!(
                reason.contains(&format!("1..={max}")),
                "{field}: the reason must cite the bound: {reason:?}"
            ),
            other => panic!("{field}={rejected}: got {other:?}"),
        }
    }
    with_value(None)
        .validate()
        .expect("None must be accepted (falls back to the consuming module's default)");
}

#[test]
fn playlist_concurrency_boundaries() {
    assert_usize_field_boundaries("playlist_concurrency", 1..=MAX_PLAYLIST_CONCURRENCY, |v| {
        Config {
            playlist_concurrency: v,
            ..Config::default()
        }
    });
}

#[test]
fn playlist_item_timeout_boundaries() {
    assert_usize_field_boundaries(
        "playlist_item_timeout",
        1..=usize::try_from(MAX_PLAYLIST_ITEM_TIMEOUT_SECS).expect("600 fits"),
        |v| Config {
            playlist_item_timeout: v.map(|n| n as u64),
            ..Config::default()
        },
    );
}

#[test]
fn max_metadata_extras_boundaries() {
    assert_usize_field_boundaries(
        "max_metadata_extras",
        1..=MAX_METADATA_EXTRAS_UPPER_BOUND,
        |v| Config {
            max_metadata_extras: v,
            ..Config::default()
        },
    );
}

#[test]
fn max_metadata_value_bytes_boundaries() {
    assert_usize_field_boundaries(
        "max_metadata_value_bytes",
        1..=MAX_METADATA_VALUE_BYTES_UPPER_BOUND,
        |v| Config {
            max_metadata_value_bytes: v,
            ..Config::default()
        },
    );
}

#[test]
fn max_metadata_extras_bytes_boundaries() {
    assert_usize_field_boundaries(
        "max_metadata_extras_bytes",
        1..=MAX_METADATA_EXTRAS_BYTES_UPPER_BOUND,
        |v| Config {
            max_metadata_extras_bytes: v,
            ..Config::default()
        },
    );
}

/// The named bounds are the documented ones — a constant edited without
/// its rationale would move every reason string with it, so the values
/// are pinned here where a change is deliberate.
#[test]
fn validate_bounds_are_the_documented_values() {
    assert_eq!(MAX_PLAYLIST_CONCURRENCY, 16);
    assert_eq!(MAX_PLAYLIST_ITEM_TIMEOUT_SECS, 600);
    assert_eq!(MAX_METADATA_EXTRAS_UPPER_BOUND, 1024);
    assert_eq!(MAX_METADATA_VALUE_BYTES_UPPER_BOUND, 1_048_576);
    assert_eq!(MAX_METADATA_EXTRAS_BYTES_UPPER_BOUND, 16_777_216);
}

#[test]
fn playlist_ignore_errors_default_is_none() {
    // Unset keeps the consuming module's `true` default, matching the
    // `Option`-as-inherit shape every other field in this cluster uses.
    assert_eq!(Config::default().playlist_ignore_errors, None);
}

#[test]
fn playlist_items_rejects_reversed_range() {
    let err = Config {
        playlist_items: Some("5-3".to_string()),
        ..Config::default()
    }
    .validate()
    .expect_err("reversed range must reject");
    assert!(matches!(
        err,
        ConfigValidationError::InvalidPlaylistItems(_)
    ));
}

#[test]
fn playlist_items_accepts_valid_spec() {
    Config {
        playlist_items: Some("1,3-5".to_string()),
        ..Config::default()
    }
    .validate()
    .expect("valid spec must be accepted");
}

#[test]
fn playlist_items_none_is_accepted() {
    Config {
        playlist_items: None,
        ..Config::default()
    }
    .validate()
    .expect("None must be accepted");
}

#[test]
fn hls_timeouts_partial_json_inherits_struct_default() {
    // Struct-level `#[serde(default)]` means an empty/partial JSON deserializes
    // to `Config::default()`-overlaid values. Pin the documented default
    // (Some(5)) so a future refactor that strips struct-level
    // serde(default) and forgets to add per-field annotations gets caught.
    let c: Config = serde_json::from_str("{}").expect("partial config must deserialize");
    assert_eq!(c.hls_head_probe_timeout, Some(5));
}

#[test]
fn hls_timeouts_round_trip_serde() {
    let c = Config {
        hls_head_probe_timeout: Some(7),
        ..Config::default()
    };
    let json = serde_json::to_string(&c).expect("serialize");
    let back: Config = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.hls_head_probe_timeout, Some(7));
}

#[test]
fn test_default_concurrent_fragments_is_8() {
    let config = Config::default();
    assert_eq!(
        config.concurrent_fragments, 8,
        "default concurrent_fragments was bumped 4 → 8 in F1 to match \
         AdaptiveConfig::default().max_connections and align with industry standard \
         (yt-dlp 1..16, N_m3u8DL-RE 8). See docs/superpowers/specs/2026-05-04-download-optimization-audit.md Q4."
    );
}

#[test]
fn test_validate_concurrent_fragments_rejects_above_64() {
    let mut config = Config::default();
    config.concurrent_fragments = 65;
    let err = config.validate().unwrap_err();
    assert!(
        matches!(
            err,
            ConfigValidationError::OutOfRange {
                field: "concurrent_fragments",
                ..
            }
        ),
        "expected OutOfRange for concurrent_fragments, got {err:?}"
    );
}

#[test]
fn test_validate_concurrent_fragments_accepts_64_boundary() {
    let mut config = Config::default();
    config.concurrent_fragments = 64;
    assert!(
        config.validate().is_ok(),
        "64 must be the inclusive upper bound"
    );
}

#[test]
fn test_default_parallel_threshold_is_the_effective_network_default() {
    let config = Config::default();
    assert_eq!(
        config.parallel_threshold,
        Some(crate::EffectiveNetwork::DEFAULT.parallel_threshold)
    );
}

#[test]
fn test_validate_parallel_threshold_rejects_zero() {
    let mut config = Config::default();
    config.parallel_threshold = Some(0);
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "parallel_threshold",
            ..
        }
    ));
}

#[test]
fn test_validate_parallel_threshold_rejects_above_1_gib() {
    let mut config = Config::default();
    config.parallel_threshold = Some(1024 * 1024 * 1024 + 1);
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "parallel_threshold",
            ..
        }
    ));
}

#[test]
fn test_validate_parallel_threshold_accepts_boundaries() {
    let mut config = Config::default();
    config.parallel_threshold = Some(1);
    assert!(config.validate().is_ok(), "1 byte must be valid");

    config.parallel_threshold = Some(1024 * 1024 * 1024);
    assert!(config.validate().is_ok(), "1 GiB must be valid");
}

#[test]
fn test_validate_parallel_threshold_none_is_valid() {
    let mut config = Config::default();
    config.parallel_threshold = None;
    assert!(
        config.validate().is_ok(),
        "None must be valid (uses downloader default)"
    );
}

#[test]
fn test_default_max_fragment_bytes_is_512_mib() {
    let config = Config::default();
    assert_eq!(config.max_fragment_bytes, Some(512 * 1024 * 1024));
    assert_eq!(config.max_fragment_bytes, Some(DEFAULT_MAX_FRAGMENT_BYTES));
}

#[test]
fn test_validate_max_fragment_bytes_rejects_zero() {
    let mut config = Config::default();
    config.max_fragment_bytes = Some(0);
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "max_fragment_bytes",
            ..
        }
    ));
}

#[test]
fn test_validate_max_fragment_bytes_rejects_above_2_gib() {
    let mut config = Config::default();
    config.max_fragment_bytes = Some(MAX_FRAGMENT_BYTES_UPPER_BOUND + 1);
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "max_fragment_bytes",
            ..
        }
    ));
}

#[test]
fn test_validate_max_fragment_bytes_accepts_boundaries() {
    let mut config = Config::default();
    config.max_fragment_bytes = Some(1);
    assert!(config.validate().is_ok(), "1 byte must be valid");

    config.max_fragment_bytes = Some(MAX_FRAGMENT_BYTES_UPPER_BOUND);
    assert!(config.validate().is_ok(), "2 GiB must be valid");
}

#[test]
fn test_validate_max_fragment_bytes_none_is_valid() {
    let mut config = Config::default();
    config.max_fragment_bytes = None;
    assert!(
        config.validate().is_ok(),
        "None must be valid (uses downloader default)"
    );
}

#[test]
fn validate_rejects_zero_recode_threads() {
    let mut cfg = Config::default();
    cfg.postprocess.recode_threads = Some(0);
    assert!(matches!(
        cfg.validate().unwrap_err(),
        ConfigValidationError::OutOfRange {
            field: "recode_threads",
            ..
        }
    ));
}

#[test]
fn validate_rejects_recode_threads_above_cap() {
    let mut cfg = Config::default();
    cfg.postprocess.recode_threads = Some(MAX_RECODE_THREADS + 1);
    assert!(matches!(
        cfg.validate().unwrap_err(),
        ConfigValidationError::OutOfRange {
            field: "recode_threads",
            ..
        }
    ));
}

#[test]
fn validate_accepts_recode_threads_in_range() {
    let mut cfg = Config::default();
    for threads in [1, 8, MAX_RECODE_THREADS] {
        cfg.postprocess.recode_threads = Some(threads);
        assert!(cfg.validate().is_ok(), "threads={threads} must be valid");
    }
}

#[test]
fn test_validate_buffer_size_accepts_1_gib_boundary() {
    let mut config = Config::default();
    config.buffer_size = 1024 * 1024 * 1024;
    assert!(
        config.validate().is_ok(),
        "1 GiB must be the inclusive upper bound"
    );
}

#[test]
fn test_validate_buffer_size_rejects_above_1_gib() {
    let mut config = Config::default();
    config.buffer_size = 1024 * 1024 * 1024 + 1;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "buffer_size",
            ..
        }
    ));
}

// --- retry settings (issue #570) ---
//
// These five fields were unvalidated for as long as they were dead: nothing
// read them until #570 wired them through to the downloader. Each test below
// pins one side of a boundary the validator now enforces.

#[test]
fn test_validate_accepts_default_retry_settings() {
    assert!(
        Config::default().validate().is_ok(),
        "the shipped defaults must satisfy the bounds they are defaulted to"
    );
}

#[test]
fn test_validate_retries_accepts_zero_and_the_ceiling() {
    // Both ends of the allowed range: 0 is "never retry", 100 is the ceiling.
    let mut config = Config::default();
    config.retries = 0;
    config.fragment_retries = 0;
    assert!(
        config.validate().is_ok(),
        "0 disables retrying and is valid"
    );

    config.retries = 100;
    config.fragment_retries = 100;
    assert!(config.validate().is_ok(), "100 is the inclusive ceiling");
}

#[test]
fn test_validate_rejects_retries_above_the_ceiling() {
    let mut config = Config::default();
    config.retries = 101;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "retries",
            ..
        }
    ));
}

#[test]
fn test_validate_rejects_fragment_retries_above_the_ceiling() {
    let mut config = Config::default();
    config.fragment_retries = 101;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "fragment_retries",
            ..
        }
    ));
}

#[test]
fn test_validate_rejects_zero_initial_retry_delay() {
    // Every backoff step is a multiple of the initial delay, so zero makes the
    // whole ladder zero — a busy loop rather than a backoff.
    let mut config = Config::default();
    config.retry_initial_delay_ms = 0;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "retry_initial_delay_ms",
            ..
        }
    ));
}

#[test]
fn test_validate_retry_delays_accept_one_millisecond_and_one_hour() {
    let mut config = Config::default();
    config.retry_initial_delay_ms = 1;
    config.retry_max_delay_ms = 1;
    assert!(config.validate().is_ok(), "1ms is the inclusive floor");

    config.retry_initial_delay_ms = 1;
    config.retry_max_delay_ms = 60 * 60 * 1000;
    assert!(
        config.validate().is_ok(),
        "one hour is the inclusive ceiling"
    );
}

#[test]
fn test_validate_rejects_retry_delay_above_one_hour() {
    let mut config = Config::default();
    config.retry_max_delay_ms = 60 * 60 * 1000 + 1;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "retry_max_delay_ms",
            ..
        }
    ));
}

#[test]
fn test_validate_rejects_max_retry_delay_below_the_initial_delay() {
    // A ceiling under the first delay silently caps every attempt at the
    // ceiling, which is not a backoff at all.
    let mut config = Config::default();
    config.retry_initial_delay_ms = 5_000;
    config.retry_max_delay_ms = 4_999;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "retry_max_delay_ms",
            ..
        }
    ));
}

#[test]
fn test_validate_retry_multiplier_accepts_both_ends_of_its_range() {
    let mut config = Config::default();
    config.retry_backoff_multiplier = 1.0;
    assert!(config.validate().is_ok(), "1.0 is a constant delay, valid");
    config.retry_backoff_multiplier = 10.0;
    assert!(config.validate().is_ok(), "10.0 is the inclusive ceiling");
}

#[test]
fn test_validate_rejects_retry_multiplier_below_one() {
    // Below 1.0 each delay is shorter than the last — the opposite of backoff.
    let mut config = Config::default();
    config.retry_backoff_multiplier = 0.5;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "retry_backoff_multiplier",
            ..
        }
    ));
}

#[test]
fn test_validate_rejects_retry_multiplier_that_saturates_to_infinity() {
    // The downloader casts this to f32, and `as` saturates an out-of-range
    // float rather than erroring: `1e40_f64 as f32` is `inf`. Bounding the
    // range is what keeps the cast finite — it does NOT make the narrowing
    // exact, since 1.1f64 and 1.1f32 differ in the mantissa.
    let mut config = Config::default();
    config.retry_backoff_multiplier = 1e40;
    let err = config.validate().unwrap_err();
    assert!(matches!(
        err,
        ConfigValidationError::OutOfRange {
            field: "retry_backoff_multiplier",
            ..
        }
    ));
}

#[test]
fn show_progress_derives_from_quiet_when_unset() {
    let mut c = Config::default();
    assert!(c.show_progress(), "default is neither quiet nor disabled");
    c.quiet = true;
    assert!(!c.show_progress(), "unset progress follows !quiet");
}

#[test]
fn show_progress_explicit_value_wins_over_quiet() {
    let c = Config {
        quiet: true,
        progress: Some(true),
        ..Config::default()
    };
    assert!(
        c.show_progress(),
        "explicit progress = true overrides quiet"
    );
    let c = Config {
        quiet: false,
        progress: Some(false),
        ..Config::default()
    };
    assert!(
        !c.show_progress(),
        "explicit progress = false without quiet"
    );
}

#[test]
fn progress_key_deserializes_and_is_absent_by_default() {
    let c: Config = toml::from_str("progress = false").expect("valid toml");
    assert_eq!(c.progress, Some(false));
    let c: Config = toml::from_str("").expect("valid toml");
    assert_eq!(c.progress, None);
}

// =============================================================================
// EffectiveNetwork resolution (#611)
// =============================================================================

/// The default `Config` materialises to exactly the single owner of the
/// nine network defaults.
#[test]
fn default_config_resolves_to_effective_network_default() {
    assert_eq!(
        Config::default().effective_network(),
        crate::EffectiveNetwork::DEFAULT
    );
}

/// Every `None` (inherit) resolves to its `DEFAULT` field. Destructured
/// without `..` so adding a field to `EffectiveNetwork` fails this test until
/// the resolver and the assertion cover it.
#[test]
fn effective_network_none_fields_resolve_to_default() {
    let config = Config {
        socket_timeout: None,
        read_timeout: None,
        pool_idle_timeout: None,
        download_timeout: None,
        merge_timeout: None,
        parallel_threshold: None,
        hls_head_probe_timeout: None,
        ..Config::default()
    };
    let d = crate::EffectiveNetwork::DEFAULT;
    let crate::EffectiveNetwork {
        socket_timeout_secs,
        read_timeout_secs,
        pool_idle_timeout_secs,
        download_timeout_secs,
        merge_timeout_secs,
        concurrent_fragments,
        buffer_size,
        parallel_threshold,
        hls_head_probe_timeout_secs,
    } = config.effective_network();
    assert_eq!(socket_timeout_secs, d.socket_timeout_secs);
    assert_eq!(read_timeout_secs, d.read_timeout_secs);
    assert_eq!(pool_idle_timeout_secs, d.pool_idle_timeout_secs);
    assert_eq!(download_timeout_secs, d.download_timeout_secs);
    assert_eq!(merge_timeout_secs, d.merge_timeout_secs);
    assert_eq!(concurrent_fragments, d.concurrent_fragments);
    assert_eq!(buffer_size, d.buffer_size);
    assert_eq!(parallel_threshold, d.parallel_threshold);
    assert_eq!(hls_head_probe_timeout_secs, d.hls_head_probe_timeout_secs);
}

/// Every `Some(x)` (and every concrete field) resolves to `x`, not to the
/// default. Values are chosen to differ from every `DEFAULT` field so a
/// resolver that ignored one `Some` would be caught.
#[test]
fn effective_network_some_fields_resolve_to_their_value() {
    let config = Config {
        socket_timeout: Some(11),
        read_timeout: Some(12),
        pool_idle_timeout: Some(0),
        download_timeout: Some(14),
        merge_timeout: Some(15),
        concurrent_fragments: 3,
        buffer_size: 4096,
        parallel_threshold: Some(17),
        hls_head_probe_timeout: Some(18),
        ..Config::default()
    };
    let crate::EffectiveNetwork {
        socket_timeout_secs,
        read_timeout_secs,
        pool_idle_timeout_secs,
        download_timeout_secs,
        merge_timeout_secs,
        concurrent_fragments,
        buffer_size,
        parallel_threshold,
        hls_head_probe_timeout_secs,
    } = config.effective_network();
    assert_eq!(socket_timeout_secs, 11);
    assert_eq!(read_timeout_secs, 12);
    assert_eq!(
        pool_idle_timeout_secs, 0,
        "0 is the disable sentinel and must survive"
    );
    assert_eq!(download_timeout_secs, 14);
    assert_eq!(merge_timeout_secs, 15);
    assert_eq!(concurrent_fragments, 3);
    assert_eq!(buffer_size, 4096);
    assert_eq!(parallel_threshold, 17);
    assert_eq!(hls_head_probe_timeout_secs, 18);
}
