//! Conditional merge of request options into Config.
//!
//! Each request sub-struct implements [`MergeOverrides`] to apply only
//! explicitly-set fields, leaving unset (`None`) values untouched in
//! the base config.

// TODO: remove once build_config() calls merge_into()
#![allow(dead_code)]

use crate::request::{FormatOptions, OutputOptions, SubtitleOptions};
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

impl MergeOverrides for SubtitleOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(v) = self.write_subs {
            config.write_subtitles = v;
        }
        if let Some(v) = self.write_auto_subs {
            config.write_auto_subtitles = v;
        }
        if !self.sub_langs.is_empty() {
            config.subtitle_langs = self.sub_langs.clone();
        }
        if let Some(v) = self.sub_format {
            config.subtitle_format = Some(v);
        }
        if let Some(v) = self.embed_subs {
            config.embed_subtitles = v;
        }
        if let Some(v) = self.strict_subs {
            config.strict_subs = v;
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

    // ─── SubtitleOptions ─────────────────────────────────────────────

    #[test]
    fn test_subtitle_none_preserves_write_subs() {
        let mut config = Config::default();
        config.write_subtitles = true;
        let opts = SubtitleOptions::default();
        opts.merge_into(&mut config);
        assert!(config.write_subtitles);
    }

    #[test]
    fn test_subtitle_some_overrides_write_subs() {
        let mut config = Config::default();
        config.write_subtitles = false;
        let opts = SubtitleOptions {
            write_subs: Some(true),
            ..SubtitleOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.write_subtitles);
    }

    #[test]
    fn test_subtitle_none_preserves_write_auto_subs() {
        let mut config = Config::default();
        config.write_auto_subtitles = true;
        let opts = SubtitleOptions::default();
        opts.merge_into(&mut config);
        assert!(config.write_auto_subtitles);
    }

    #[test]
    fn test_subtitle_some_overrides_write_auto_subs() {
        let mut config = Config::default();
        config.write_auto_subtitles = false;
        let opts = SubtitleOptions {
            write_auto_subs: Some(true),
            ..SubtitleOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.write_auto_subtitles);
    }

    #[test]
    fn test_subtitle_empty_preserves_sub_langs() {
        let mut config = Config::default();
        config.subtitle_langs = vec!["en".into(), "ja".into()];
        let opts = SubtitleOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.subtitle_langs, vec!["en", "ja"]);
    }

    #[test]
    fn test_subtitle_nonempty_overrides_sub_langs() {
        let mut config = Config::default();
        config.subtitle_langs = vec!["en".into()];
        let opts = SubtitleOptions {
            sub_langs: vec!["de".into(), "fr".into()],
            ..SubtitleOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.subtitle_langs, vec!["de", "fr"]);
    }

    #[test]
    fn test_subtitle_none_preserves_sub_format() {
        let mut config = Config::default();
        config.subtitle_format = Some(rdlp_core::SubtitleFormat::Vtt);
        let opts = SubtitleOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.subtitle_format, Some(rdlp_core::SubtitleFormat::Vtt));
    }

    #[test]
    fn test_subtitle_some_overrides_sub_format() {
        let mut config = Config::default();
        config.subtitle_format = Some(rdlp_core::SubtitleFormat::Vtt);
        let opts = SubtitleOptions {
            sub_format: Some(rdlp_core::SubtitleFormat::Srt),
            ..SubtitleOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.subtitle_format, Some(rdlp_core::SubtitleFormat::Srt));
    }

    #[test]
    fn test_subtitle_none_preserves_embed_subs() {
        let mut config = Config::default();
        config.embed_subtitles = true;
        let opts = SubtitleOptions::default();
        opts.merge_into(&mut config);
        assert!(config.embed_subtitles);
    }

    #[test]
    fn test_subtitle_some_overrides_embed_subs() {
        let mut config = Config::default();
        config.embed_subtitles = false;
        let opts = SubtitleOptions {
            embed_subs: Some(true),
            ..SubtitleOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.embed_subtitles);
    }

    #[test]
    fn test_subtitle_none_preserves_strict_subs() {
        let mut config = Config::default();
        config.strict_subs = true;
        let opts = SubtitleOptions::default();
        opts.merge_into(&mut config);
        assert!(config.strict_subs);
    }

    #[test]
    fn test_subtitle_some_overrides_strict_subs() {
        let mut config = Config::default();
        config.strict_subs = false;
        let opts = SubtitleOptions {
            strict_subs: Some(true),
            ..SubtitleOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.strict_subs);
    }
}
