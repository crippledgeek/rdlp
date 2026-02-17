//! Conditional merge of request options into Config.
//!
//! Each request sub-struct implements [`MergeOverrides`] to apply only
//! explicitly-set fields, leaving unset (`None`) values untouched in
//! the base config.

// TODO: remove once build_config() calls merge_into()
#![allow(dead_code)]

use crate::request::{FormatOptions, OutputOptions};
use rdlp_core::Config;

/// Merge explicitly-set request fields into a base [`Config`].
///
/// Implementors MUST follow the "only override when Some" rule:
/// `None` fields are skipped, preserving whatever the config already has.
pub(crate) trait MergeOverrides {
    /// Apply overrides from `self` to `config`.
    fn merge_into(&self, config: &mut Config);
}

impl MergeOverrides for OutputOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(ref v) = self.output_dir {
            config.output_directory = v.clone();
        }
        if let Some(ref v) = self.template {
            config.output_template = v.clone();
        }
    }
}

impl MergeOverrides for FormatOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(ref v) = self.selector {
            config.format = v.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ─── OutputOptions ───────────────────────────────────────────────

    #[test]
    fn test_output_none_preserves_output_dir() {
        let mut config = Config::default();
        config.output_directory = PathBuf::from("/kept");
        let opts = OutputOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.output_directory, PathBuf::from("/kept"));
    }

    #[test]
    fn test_output_some_overrides_output_dir() {
        let mut config = Config::default();
        config.output_directory = PathBuf::from("/old");
        let opts = OutputOptions {
            output_dir: Some(PathBuf::from("/new")),
            ..OutputOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.output_directory, PathBuf::from("/new"));
    }

    #[test]
    fn test_output_none_preserves_template() {
        let mut config = Config::default();
        config.output_template = "kept.%(ext)s".to_string();
        let opts = OutputOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.output_template, "kept.%(ext)s");
    }

    #[test]
    fn test_output_some_overrides_template() {
        let mut config = Config::default();
        config.output_template = "old.%(ext)s".to_string();
        let opts = OutputOptions {
            template: Some("new.%(ext)s".into()),
            ..OutputOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.output_template, "new.%(ext)s");
    }

    // ─── FormatOptions ───────────────────────────────────────────────

    #[test]
    fn test_format_none_preserves_selector() {
        let mut config = Config::default();
        config.format = "kept".to_string();
        let opts = FormatOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.format, "kept");
    }

    #[test]
    fn test_format_some_overrides_selector() {
        let mut config = Config::default();
        config.format = "old".to_string();
        let opts = FormatOptions {
            selector: Some("bestaudio".into()),
            ..FormatOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.format, "bestaudio");
    }
}
