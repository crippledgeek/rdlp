//! Configuration file I/O operations
//!
//! This module provides functions to load and save Config from/to files.
//! The Config type itself is defined in rdlp-types.

use crate::{Config, Result, RdlpError};
use std::path::Path;

/// Load configuration from a TOML file
pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Config> {
    let content = std::fs::read_to_string(path.as_ref())?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| RdlpError::Config(format!("Failed to parse TOML: {e}")))?;
    Ok(config)
}

/// Load configuration from a YAML file
pub fn from_yaml_file(path: impl AsRef<Path>) -> Result<Config> {
    let content = std::fs::read_to_string(path.as_ref())?;
    let config: Config = serde_yaml::from_str(&content)
        .map_err(|e| RdlpError::Config(format!("Failed to parse YAML: {e}")))?;
    Ok(config)
}

/// Save configuration to a TOML file
pub fn to_toml_file(config: &Config, path: impl AsRef<Path>) -> Result<()> {
    let content = toml::to_string_pretty(config)
        .map_err(|e| RdlpError::Config(format!("Failed to serialize TOML: {e}")))?;
    std::fs::write(path.as_ref(), content)?;
    Ok(())
}

/// Save configuration to a YAML file
pub fn to_yaml_file(config: &Config, path: impl AsRef<Path>) -> Result<()> {
    let content = serde_yaml::to_string(config)
        .map_err(|e| RdlpError::Config(format!("Failed to serialize YAML: {e}")))?;
    std::fs::write(path.as_ref(), content)?;
    Ok(())
}

/// Validate configuration, returning RdlpError on failure
pub fn validate(config: &Config) -> Result<()> {
    config.validate().map_err(RdlpError::Config)
}
