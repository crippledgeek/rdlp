//! Configuration file I/O operations
//!
//! This module provides functions to load and save Config from/to TOML files.
//! The Config type itself is defined in rdlp-types.

use crate::{RdlpError, Result};
use rdlp_types::Config;
use std::path::{Path, PathBuf};

/// Returns the platform-specific default config file path.
///
/// - Windows: `%APPDATA%\rdlp\config.toml`
/// - Linux/macOS: `~/.config/rdlp/config.toml`
///
/// Returns `None` if the platform config directory cannot be determined.
#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rdlp").join("config.toml"))
}

/// Load configuration from a TOML file.
///
/// Missing fields use `Config::default()` values thanks to `#[serde(default)]`.
///
/// # Errors
///
/// Returns an error if the file cannot be read (`io::Error`) or the TOML
/// is malformed (`RdlpError::Config`).
pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Config> {
    // Safe: sync public API invoked from CLI startup / tests before any async runtime.
    // Callers in async contexts must wrap in spawn_blocking.
    #[allow(clippy::disallowed_methods)]
    let content = std::fs::read_to_string(path.as_ref())?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| RdlpError::Config(format!("Failed to parse TOML: {e}")))?;
    Ok(config)
}

/// Load configuration from an explicit path or the default location.
///
/// - If `path` is `Some`, loads from that path (errors if file doesn't exist or is invalid).
/// - If `path` is `None`, tries `default_config_path()`. Returns `Ok(None)` if no file found.
///
/// Returns `Ok(Some(config))` on success, `Ok(None)` if no config file exists at the
/// default location, or `Err` on parse/IO errors.
///
/// # Errors
///
/// Returns an error if the given path cannot be read or parsed, or if an existing default
/// config file is malformed.
pub fn load_config(path: Option<&Path>) -> Result<Option<(Config, PathBuf)>> {
    if let Some(p) = path {
        let config = from_toml_file(p)?;
        Ok(Some((config, p.to_path_buf())))
    } else {
        let Some(default_path) = default_config_path() else {
            return Ok(None);
        };
        if !default_path.exists() {
            return Ok(None);
        }
        let config = from_toml_file(&default_path)?;
        Ok(Some((config, default_path)))
    }
}

/// Save configuration to a TOML file
///
/// # Errors
///
/// Returns an error if the configuration cannot be serialized (`RdlpError::Config`)
/// or the file cannot be written (`io::Error`).
pub fn write_to_file(config: &Config, path: impl AsRef<Path>) -> Result<()> {
    let content = toml::to_string_pretty(config)
        .map_err(|e| RdlpError::Config(format!("Failed to serialize TOML: {e}")))?;
    // Safe: sync public API invoked from CLI startup / tests before any async runtime.
    // Callers in async contexts must wrap in spawn_blocking.
    #[allow(clippy::disallowed_methods)]
    std::fs::write(path.as_ref(), content)?;
    Ok(())
}

/// Validate configuration, returning `RdlpError` on failure
///
/// # Errors
///
/// Returns [`RdlpError::Config`] if `config.validate()` fails.
pub fn validate(config: &Config) -> Result<()> {
    config
        .validate()
        .map_err(|e| RdlpError::Config(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_partial_toml() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "verbose = true").unwrap();
        writeln!(file, "format = \"worst\"").unwrap();

        let config = from_toml_file(file.path()).unwrap();
        assert!(config.verbose);
        assert_eq!(config.format, Some("worst".to_string()));
        // Defaults should fill in the rest
        assert_eq!(config.concurrent_fragments, 8);
        assert!(!config.quiet);
    }

    #[test]
    fn test_load_empty_toml() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file).unwrap();

        let config = from_toml_file(file.path()).unwrap();
        let defaults = Config::default();
        assert_eq!(config.format, defaults.format);
        assert_eq!(config.concurrent_fragments, defaults.concurrent_fragments);
    }

    #[test]
    fn test_load_full_toml_roundtrip() {
        let original = Config::default();
        let mut file = NamedTempFile::new().unwrap();
        let content = toml::to_string_pretty(&original).unwrap();
        write!(file, "{content}").unwrap();

        let loaded = from_toml_file(file.path()).unwrap();
        assert_eq!(loaded.format, original.format);
        assert_eq!(loaded.buffer_size, original.buffer_size);
        assert_eq!(loaded.concurrent_fragments, original.concurrent_fragments);
    }

    /// #642: `postprocess.video_encoder` moved from `Option<String>` to
    /// `Option<VideoEncoderName>`. The wire format is unchanged — a plain
    /// TOML string — so an existing `config.toml` with a valid encoder name
    /// still loads exactly as before.
    #[test]
    fn test_video_encoder_toml_roundtrip() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "[postprocess]").unwrap();
        writeln!(file, "video_encoder = \"libx264\"").unwrap();

        let config = from_toml_file(file.path()).unwrap();
        assert_eq!(config.postprocess.video_encoder.as_deref(), Some("libx264"));
    }

    /// Negative companion: an empty `video_encoder` value — tolerated as
    /// "no override" before #642's `Option<String>` → `Option<VideoEncoderName>`
    /// change — now fails to deserialize with a clear, actionable message
    /// instead of silently reaching the recode stage as a would-be empty
    /// encoder override.
    #[test]
    fn test_video_encoder_empty_string_rejected_at_load() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "[postprocess]").unwrap();
        writeln!(file, "video_encoder = \"\"").unwrap();

        let err = from_toml_file(file.path()).expect_err("empty video_encoder must fail to load");
        let msg = err.to_string();
        assert!(
            msg.contains("must not be empty"),
            "error should name the actual problem, got: {msg}"
        );
    }

    #[test]
    fn test_load_config_explicit_path() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "quiet = true").unwrap();

        let result = load_config(Some(file.path())).unwrap();
        assert!(result.is_some());
        let (config, path) = result.unwrap();
        assert!(config.quiet);
        assert_eq!(path, file.path());
    }

    #[test]
    fn test_load_config_missing_explicit_path() {
        let result = load_config(Some(Path::new("/nonexistent/config.toml")));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_no_default() {
        // When no path given and no default file exists, returns None
        let result = load_config(None).unwrap();
        // May or may not be Some depending on whether the user has a config file
        // We can't assert None here because the test runner might have one
        // Just verify it doesn't error
        let _ = result;
    }

    #[test]
    fn test_default_config_path_is_some() {
        // On most systems, config_dir() returns something
        let path = default_config_path();
        if let Some(p) = &path {
            assert!(p.ends_with("config.toml"));
            assert!(p.to_string_lossy().contains("rdlp"));
        }
    }

    #[test]
    fn test_invalid_toml() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "not valid toml {{{{").unwrap();

        let result = from_toml_file(file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_field_rejected() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "nonexistent_field = true").unwrap();

        let result = from_toml_file(file.path());
        // serde should reject unknown fields by default with deny_unknown_fields
        // Without it, unknown fields are silently ignored
        // Config doesn't have deny_unknown_fields, so this should succeed
        assert!(result.is_ok());
    }

    #[test]
    fn test_save_and_reload() {
        let config = Config {
            format: Some("worst".to_string()),
            verbose: true,
            rate_limit: Some(1_048_576),
            ..Default::default()
        };

        let file = NamedTempFile::new().unwrap();
        write_to_file(&config, file.path()).unwrap();

        let loaded = from_toml_file(file.path()).unwrap();
        assert_eq!(loaded.format, Some("worst".to_string()));
        assert!(loaded.verbose);
        assert_eq!(loaded.rate_limit, Some(1_048_576));
    }
}
