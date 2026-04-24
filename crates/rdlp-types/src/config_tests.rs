//! Tests for configuration types and validation.
#![allow(clippy::field_reassign_with_default)]

use super::*;
use crate::container::ContainerFormat;

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.output_template, "%(title)s.%(ext)s");
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
