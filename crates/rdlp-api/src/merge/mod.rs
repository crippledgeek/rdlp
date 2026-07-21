//! Conditional merge of request options into Config.
//!
//! Each request sub-struct implements [`MergeOverrides`] to apply only
//! explicitly-set fields, leaving unset (`None`) values untouched in
//! the base config.

use crate::request::{
    FormatOptions, NetworkOptions, OutputOptions, PostProcessOptions, SubtitleOptions,
};
use rdlp_types::Config;

#[cfg(test)]
mod tests_output_format_subtitle;
#[cfg(test)]
mod tests_postprocess_network;

/// Merge explicitly-set request fields into a base [`Config`].
///
/// Implementors MUST follow the "only override when Some" rule:
/// `None` fields are skipped, preserving whatever the config already has.
pub trait MergeOverrides {
    /// Apply overrides from `self` to `config`.
    fn merge_into(&self, config: &mut Config);
}

impl MergeOverrides for OutputOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(ref v) = self.output_dir {
            config.output_directory.clone_from(v);
        }
        if let Some(ref v) = self.template {
            config.output_template.clone_from(v);
        }
        if let Some(v) = self.stdout {
            config.output_to_stdout = v;
            if v {
                // Mirror CLI behaviour: silence output and disable incompatible
                // defaults so Config::validate() won't reject the combination.
                config.quiet = true;
                config.postprocess.embed_thumbnail = false;
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
            config.postprocess.write_subtitles = v;
        }
        if let Some(v) = self.write_auto_subs {
            config.write_auto_subtitles = v;
        }
        if !self.sub_langs.is_empty() {
            config.subtitle_langs.clone_from(&self.sub_langs);
        }
        if let Some(v) = self.sub_format {
            config.subtitle_format = Some(v);
        }
        if let Some(v) = self.embed_subs {
            config.postprocess.embed_subtitles = v;
        }
        if let Some(v) = self.strict_subs {
            config.strict_subs = v;
        }
        if let Some(v) = self.verify_sub_urls {
            config.verify_sub_urls = v;
        }
        if let Some(v) = self.retry_subs {
            config.retry_subs = v;
        }
    }
}

impl MergeOverrides for PostProcessOptions {
    fn merge_into(&self, config: &mut Config) {
        if let Some(v) = self.remux {
            config.postprocess.remux_container = Some(v);
        }
        if let Some(v) = self.extract_audio {
            config.postprocess.extract_audio = true;
            config.postprocess.audio_format = Some(v);
        }
        if let Some(v) = self.embed_metadata {
            config.postprocess.embed_metadata = v;
        }
        if let Some(v) = self.embed_thumbnail {
            config.postprocess.embed_thumbnail = v;
        }
        if self.no_thumbnail == Some(true) {
            config.postprocess.embed_thumbnail = false;
            config.postprocess.write_thumbnail = false;
        }
        if let Some(v) = self.write_thumbnail {
            config.postprocess.write_thumbnail = v;
        }
        if let Some(v) = self.normalize_audio {
            config.postprocess.normalize_audio = v;
        }
        if let Some(v) = self.loudnorm {
            config.postprocess.loudnorm = v;
        }
        if let Some(ref v) = self.loudnorm_preset {
            config.postprocess.loudnorm_preset = Some(v.clone());
        }
        if let Some(v) = self.loudnorm_target_i {
            config.postprocess.loudnorm_target_i = Some(v);
        }
        if let Some(v) = self.loudnorm_target_tp {
            config.postprocess.loudnorm_target_tp = Some(v);
        }
        if let Some(v) = self.loudnorm_target_lra {
            config.postprocess.loudnorm_target_lra = Some(v);
        }
        if let Some(v) = self.loudnorm_dynamic {
            config.postprocess.loudnorm_dynamic = v;
        }
        if let Some(v) = self.loudnorm_precompress {
            config.postprocess.loudnorm_precompress = v;
        }
        if let Some(v) = self.normalize_boost {
            config.postprocess.normalize_boost = v;
        }
        if let Some(v) = self.normalize_boost_db {
            config.postprocess.normalize_boost_db = Some(v);
        }
        if let Some(v) = self.recode_video {
            config.postprocess.recode_video = Some(v);
        }
        if let Some(ref v) = self.video_encoder {
            config.postprocess.video_encoder = Some(v.clone());
        }
        if let Some(v) = self.recode_container {
            config.postprocess.recode_container = Some(v);
        }
        if let Some(ref v) = self.recode_audio {
            config.postprocess.recode_audio = v.clone();
        }
        if let Some(v) = self.recode_threads {
            config.postprocess.recode_threads = Some(v);
        }
        if let Some(ref v) = self.recode_preset {
            config.postprocess.recode_preset = Some(v.clone());
        }
        if let Some(v) = self.recode_deadline {
            config.postprocess.recode_deadline = Some(v);
        }
        if let Some(v) = self.recode_cpu_used {
            config.postprocess.recode_cpu_used = Some(v);
        }
        if let Some(v) = self.recode_speed_level {
            config.postprocess.recode_speed_level = Some(v);
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
        if let Some(v) = self.read_timeout_secs {
            config.read_timeout = Some(v);
        }
        if let Some(v) = self.pool_idle_timeout_secs {
            config.pool_idle_timeout = Some(v);
        }
        if let Some(v) = self.download_timeout_secs {
            config.download_timeout = Some(v);
        }
        if let Some(v) = self.merge_timeout_secs {
            config.merge_timeout = Some(v);
        }
        if let Some(v) = self.concurrent_fragments {
            config.concurrent_fragments = v as usize;
        }
        if let Some(v) = self.buffer_size {
            // Genuinely narrowing on a 32-bit target. Values are capped at 1 GiB by
            // AppSettings::validate_security / Config::validate upstream, so the
            // saturating fallback is unreachable on every target rdlp supports.
            config.buffer_size = usize::try_from(v).unwrap_or(usize::MAX);
        }
        if let Some(v) = self.parallel_threshold {
            config.parallel_threshold = Some(v);
        }
        if let Some(v) = self.hls_head_probe_timeout {
            config.hls_head_probe_timeout = Some(v);
        }
        if let Some(v) = self.rate_limit {
            config.rate_limit = Some(v);
        }
        if let Some(ref v) = self.proxy {
            config.proxy = Some(v.clone());
        }
        if let Some(v) = self.cookies_from_browser {
            config.cookies_from_browser = Some(v);
        }
        if let Some(ref v) = self.cookies_file {
            config.cookies_file = Some(v.clone());
        }
    }
}
