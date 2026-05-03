//! Tests for configuration types and validation.
#![allow(clippy::field_reassign_with_default)]

use super::*;
use crate::container::ContainerFormat;

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.output_template, "%(title|Unknown)s [%(id)s].%(ext)s");
    assert!(config.format.is_none());
    assert!(config.continue_downloads);
    assert_eq!(config.concurrent_fragments, 4);
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
    config.concurrent_fragments = 4;
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
