//! Conditional merge of request options into Config.
//!
//! Each request sub-struct implements [`MergeOverrides`] to apply only
//! explicitly-set fields, leaving unset (`None`) values untouched in
//! the base config.

use crate::request::{
    FormatOptions, NetworkOptions, OutputOptions, PostProcessOptions, SubtitleOptions,
};
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
            config.format = Some(v.clone());
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

impl MergeOverrides for PostProcessOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(v) = self.remux {
            config.remux_container = Some(v);
        }
        if let Some(v) = self.extract_audio {
            config.extract_audio = true;
            config.audio_format = Some(v);
        }
        if let Some(v) = self.embed_metadata {
            config.embed_metadata = v;
        }
        if let Some(v) = self.embed_thumbnail {
            config.embed_thumbnail = v;
        }
        if let Some(true) = self.no_thumbnail {
            config.embed_thumbnail = false;
            config.write_thumbnail = false;
        }
        if let Some(v) = self.write_thumbnail {
            config.write_thumbnail = v;
        }
        if let Some(v) = self.normalize_audio {
            config.normalize_audio = v;
        }
        if let Some(v) = self.loudnorm {
            config.loudnorm = v;
        }
        if let Some(ref v) = self.loudnorm_preset {
            config.loudnorm_preset = Some(v.clone());
        }
    }
}

impl MergeOverrides for NetworkOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(v) = self.retries {
            config.retries = v as usize;
        }
        if let Some(v) = self.timeout_secs {
            config.socket_timeout = Some(v);
        }
        if let Some(v) = self.concurrent_fragments {
            config.concurrent_fragments = v as usize;
        }
        if let Some(v) = self.rate_limit {
            config.rate_limit = Some(v);
        }
        if let Some(v) = self.cookies_from_browser {
            config.cookies_from_browser = Some(v);
        }
        if let Some(ref v) = self.cookies_file {
            config.cookies_file = Some(v.clone());
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
        config.format = Some("kept".to_string());
        let opts = FormatOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.format, Some("kept".to_string()));
    }

    #[test]
    fn test_format_some_overrides_selector() {
        let mut config = Config::default();
        config.format = Some("old".to_string());
        let opts = FormatOptions {
            selector: Some("bestaudio".into()),
            ..FormatOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.format, Some("bestaudio".to_string()));
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

    // ─── PostProcessOptions ──────────────────────────────────────────

    #[test]
    fn test_postprocess_none_preserves_remux() {
        let mut config = Config::default();
        config.remux_container = Some(rdlp_core::ContainerFormat::Mkv);
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(
            config.remux_container,
            Some(rdlp_core::ContainerFormat::Mkv)
        );
    }

    #[test]
    fn test_postprocess_some_overrides_remux() {
        let mut config = Config::default();
        config.remux_container = None;
        let opts = PostProcessOptions {
            remux: Some(rdlp_core::ContainerFormat::Mp4),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(
            config.remux_container,
            Some(rdlp_core::ContainerFormat::Mp4)
        );
    }

    #[test]
    fn test_postprocess_none_preserves_extract_audio() {
        let mut config = Config::default();
        config.extract_audio = true;
        config.audio_format = Some(rdlp_core::AudioFormat::Mp3);
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.extract_audio);
        assert_eq!(config.audio_format, Some(rdlp_core::AudioFormat::Mp3));
    }

    #[test]
    fn test_postprocess_some_overrides_extract_audio() {
        let mut config = Config::default();
        config.extract_audio = false;
        config.audio_format = None;
        let opts = PostProcessOptions {
            extract_audio: Some(rdlp_core::AudioFormat::Aac),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.extract_audio);
        assert_eq!(config.audio_format, Some(rdlp_core::AudioFormat::Aac));
    }

    #[test]
    fn test_postprocess_none_preserves_embed_metadata() {
        let mut config = Config::default();
        config.embed_metadata = true;
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.embed_metadata);
    }

    #[test]
    fn test_postprocess_some_overrides_embed_metadata() {
        let mut config = Config::default();
        config.embed_metadata = false;
        let opts = PostProcessOptions {
            embed_metadata: Some(true),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.embed_metadata);
    }

    #[test]
    fn test_postprocess_none_preserves_embed_thumbnail() {
        let mut config = Config::default();
        config.embed_thumbnail = true;
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.embed_thumbnail);
    }

    #[test]
    fn test_postprocess_some_overrides_embed_thumbnail() {
        let mut config = Config::default();
        config.embed_thumbnail = false;
        let opts = PostProcessOptions {
            embed_thumbnail: Some(true),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.embed_thumbnail);
    }

    #[test]
    fn test_postprocess_none_preserves_no_thumbnail() {
        let mut config = Config::default();
        config.embed_thumbnail = true;
        config.write_thumbnail = true;
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.embed_thumbnail);
        assert!(config.write_thumbnail);
    }

    #[test]
    fn test_postprocess_some_overrides_no_thumbnail() {
        let mut config = Config::default();
        config.embed_thumbnail = true;
        config.write_thumbnail = true;
        let opts = PostProcessOptions {
            no_thumbnail: Some(true),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(!config.embed_thumbnail);
        assert!(!config.write_thumbnail);
    }

    #[test]
    fn test_postprocess_none_preserves_write_thumbnail() {
        let mut config = Config::default();
        config.write_thumbnail = true;
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.write_thumbnail);
    }

    #[test]
    fn test_postprocess_some_overrides_write_thumbnail() {
        let mut config = Config::default();
        config.write_thumbnail = false;
        let opts = PostProcessOptions {
            write_thumbnail: Some(true),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.write_thumbnail);
    }

    #[test]
    fn test_postprocess_none_preserves_normalize_audio() {
        let mut config = Config::default();
        config.normalize_audio = true;
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.normalize_audio);
    }

    #[test]
    fn test_postprocess_some_overrides_normalize_audio() {
        let mut config = Config::default();
        config.normalize_audio = false;
        let opts = PostProcessOptions {
            normalize_audio: Some(true),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.normalize_audio);
    }

    #[test]
    fn test_postprocess_none_preserves_loudnorm() {
        let mut config = Config::default();
        config.loudnorm = true;
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert!(config.loudnorm);
    }

    #[test]
    fn test_postprocess_some_overrides_loudnorm() {
        let mut config = Config::default();
        config.loudnorm = false;
        let opts = PostProcessOptions {
            loudnorm: Some(true),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert!(config.loudnorm);
    }

    #[test]
    fn test_postprocess_none_preserves_loudnorm_preset() {
        let mut config = Config::default();
        config.loudnorm_preset = Some("broadcast".into());
        let opts = PostProcessOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.loudnorm_preset.as_deref(), Some("broadcast"));
    }

    #[test]
    fn test_postprocess_some_overrides_loudnorm_preset() {
        let mut config = Config::default();
        config.loudnorm_preset = Some("broadcast".into());
        let opts = PostProcessOptions {
            loudnorm_preset: Some("streaming".into()),
            ..PostProcessOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.loudnorm_preset.as_deref(), Some("streaming"));
    }

    // ─── NetworkOptions ──────────────────────────────────────────────

    #[test]
    fn test_network_none_preserves_retries() {
        let mut config = Config::default();
        config.retries = 42;
        let opts = NetworkOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.retries, 42);
    }

    #[test]
    fn test_network_some_overrides_retries() {
        let mut config = Config::default();
        config.retries = 10;
        let opts = NetworkOptions {
            retries: Some(3),
            ..NetworkOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.retries, 3);
    }

    #[test]
    fn test_network_none_preserves_timeout_secs() {
        let mut config = Config::default();
        config.socket_timeout = Some(99);
        let opts = NetworkOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.socket_timeout, Some(99));
    }

    #[test]
    fn test_network_some_overrides_timeout_secs() {
        let mut config = Config::default();
        config.socket_timeout = Some(30);
        let opts = NetworkOptions {
            timeout_secs: Some(120),
            ..NetworkOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.socket_timeout, Some(120));
    }

    #[test]
    fn test_network_none_preserves_concurrent_fragments() {
        let mut config = Config::default();
        config.concurrent_fragments = 16;
        let opts = NetworkOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.concurrent_fragments, 16);
    }

    #[test]
    fn test_network_some_overrides_concurrent_fragments() {
        let mut config = Config::default();
        config.concurrent_fragments = 4;
        let opts = NetworkOptions {
            concurrent_fragments: Some(8),
            ..NetworkOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.concurrent_fragments, 8);
    }

    #[test]
    fn test_network_none_preserves_rate_limit() {
        let mut config = Config::default();
        config.rate_limit = Some(1_000_000);
        let opts = NetworkOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(config.rate_limit, Some(1_000_000));
    }

    #[test]
    fn test_network_some_overrides_rate_limit() {
        let mut config = Config::default();
        config.rate_limit = None;
        let opts = NetworkOptions {
            rate_limit: Some(500_000),
            ..NetworkOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.rate_limit, Some(500_000));
    }

    #[test]
    fn test_network_none_preserves_cookies_from_browser() {
        let mut config = Config::default();
        config.cookies_from_browser = Some(rdlp_core::BrowserType::Firefox);
        let opts = NetworkOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(
            config.cookies_from_browser,
            Some(rdlp_core::BrowserType::Firefox)
        );
    }

    #[test]
    fn test_network_some_overrides_cookies_from_browser() {
        let mut config = Config::default();
        config.cookies_from_browser = None;
        let opts = NetworkOptions {
            cookies_from_browser: Some(rdlp_core::BrowserType::Chrome),
            ..NetworkOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(
            config.cookies_from_browser,
            Some(rdlp_core::BrowserType::Chrome)
        );
    }

    #[test]
    fn test_network_none_preserves_cookies_file() {
        let mut config = Config::default();
        config.cookies_file = Some(PathBuf::from("/kept/cookies.txt"));
        let opts = NetworkOptions::default();
        opts.merge_into(&mut config);
        assert_eq!(
            config.cookies_file,
            Some(PathBuf::from("/kept/cookies.txt"))
        );
    }

    #[test]
    fn test_network_some_overrides_cookies_file() {
        let mut config = Config::default();
        config.cookies_file = None;
        let opts = NetworkOptions {
            cookies_file: Some(PathBuf::from("/new/cookies.txt")),
            ..NetworkOptions::default()
        };
        opts.merge_into(&mut config);
        assert_eq!(config.cookies_file, Some(PathBuf::from("/new/cookies.txt")));
    }
}
