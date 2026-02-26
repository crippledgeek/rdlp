//! Conditional merge of request options into Config.
//!
//! Each request sub-struct implements [`MergeOverrides`] to apply only
//! explicitly-set fields, leaving unset (`None`) values untouched in
//! the base config.

use crate::request::{
    FormatOptions, NetworkOptions, OutputOptions, PostProcessOptions, SubtitleOptions,
};
use rdlp_core::Config;

#[cfg(test)]
mod tests_output_format_subtitle;
#[cfg(test)]
mod tests_postprocess_network;

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
        if let Some(v) = self.stdout {
            config.output_to_stdout = v;
            if v {
                // Mirror CLI behaviour: silence output and disable incompatible
                // defaults so Config::validate() won't reject the combination.
                config.quiet = true;
                config.progress = false;
                config.embed_thumbnail = false;
            }
        }
    }
}

impl MergeOverrides for FormatOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(ref v) = self.selector {
            config.format = Some(v.clone());
        }
        if let Some(v) = self.audio_multistreams {
            config.audio_multistreams = v;
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
        if let Some(v) = self.loudnorm_target_i {
            config.loudnorm_target_i = Some(v);
        }
        if let Some(v) = self.loudnorm_target_tp {
            config.loudnorm_target_tp = Some(v);
        }
        if let Some(v) = self.loudnorm_target_lra {
            config.loudnorm_target_lra = Some(v);
        }
        if let Some(v) = self.loudnorm_dynamic {
            config.loudnorm_dynamic = v;
        }
        if let Some(v) = self.loudnorm_precompress {
            config.loudnorm_precompress = v;
        }
        if let Some(v) = self.normalize_boost {
            config.normalize_boost = v;
        }
        if let Some(v) = self.normalize_boost_db {
            config.normalize_boost_db = Some(v);
        }
        if let Some(v) = self.recode_video {
            config.recode_video = Some(v);
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
