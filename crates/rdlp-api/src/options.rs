//! Pure-data option registry (config↔GUI axis). NO clap, NO I/O.
//!
//! [`OPTION_REGISTRY`] maps every `rdlp_types::Config`/`PostProcess` leaf
//! field to its desktop-GUI exposure. It is metadata only — the CLI↔config
//! axis is already gated elsewhere (the `rdlp-cli` merge-exhaustiveness
//! canary and `Args` tiering canaries), so this registry deliberately omits
//! `cli`/`group`/`tier` to avoid duplicating data those own.

/// How a `Config`/`PostProcess` option is exposed on the desktop GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gui {
    /// Bound to this `AppSettings` field (gate: the field must exist).
    Control(&'static str),
    /// Deliberately no GUI control; the string is the rationale.
    NotApplicable(&'static str),
    /// A control that should exist but does not yet; the string is the tracking issue, e.g. "#602".
    Missing(&'static str),
}

/// One registry entry: a config field path and its GUI exposure.
#[derive(Debug, Clone, Copy)]
pub struct OptionEntry {
    /// Canonical config path: `"<field>"` or `"postprocess.<field>"`. Primary key.
    pub field: &'static str,
    /// The field's desktop-GUI exposure classification.
    pub gui: Gui,
}

/// The registry. One entry per `Config`/`PostProcess` leaf field (96 total).
pub const OPTION_REGISTRY: &[OptionEntry] = &[
    OptionEntry {
        field: "output_to_stdout",
        gui: Gui::NotApplicable("-o - stdout streaming; a per-invocation CLI action"),
    },
    OptionEntry {
        field: "output_template",
        gui: Gui::Control("output_template"),
    },
    OptionEntry {
        field: "output_directory",
        gui: Gui::Control("output_dir"),
    },
    OptionEntry {
        field: "restrict_filenames",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "overwrite",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "continue_downloads",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "no_part",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "format",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "audio_multistreams",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "concurrent_fragments",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "rate_limit",
        gui: Gui::Control("rate_limit"),
    },
    OptionEntry {
        field: "retries",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "fragment_retries",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "retry_initial_delay_ms",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "retry_max_delay_ms",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "retry_backoff_multiplier",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "buffer_size",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "proxy",
        gui: Gui::Control("proxy"),
    },
    OptionEntry {
        field: "socket_timeout",
        gui: Gui::Control("socket_timeout"),
    },
    OptionEntry {
        field: "read_timeout",
        gui: Gui::Control("read_timeout"),
    },
    OptionEntry {
        field: "pool_idle_timeout",
        gui: Gui::Control("pool_idle_timeout"),
    },
    OptionEntry {
        field: "download_timeout",
        gui: Gui::Control("download_timeout"),
    },
    OptionEntry {
        field: "merge_timeout",
        gui: Gui::Control("merge_timeout"),
    },
    OptionEntry {
        field: "hls_head_probe_timeout",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "parallel_threshold",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "source_address",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "user_agent",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "browser_emulation",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "http_headers",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "write_auto_subtitles",
        gui: Gui::Control("write_auto_subtitles"),
    },
    OptionEntry {
        field: "subtitle_langs",
        gui: Gui::Control("default_subtitle_langs"),
    },
    OptionEntry {
        field: "subtitle_format",
        gui: Gui::Control("default_subtitle_format"),
    },
    OptionEntry {
        field: "list_subs",
        gui: Gui::NotApplicable("per-invocation subtitle-listing action"),
    },
    OptionEntry {
        field: "strict_subs",
        gui: Gui::Control("strict_subs"),
    },
    OptionEntry {
        field: "verify_sub_urls",
        gui: Gui::Control("verify_sub_urls"),
    },
    OptionEntry {
        field: "retry_subs",
        gui: Gui::Control("retry_subs"),
    },
    OptionEntry {
        field: "quiet",
        gui: Gui::NotApplicable("per-invocation log-suppression; GUI manages its own logging"),
    },
    OptionEntry {
        field: "verbose",
        gui: Gui::Control("verbose"),
    },
    OptionEntry {
        field: "simulate",
        gui: Gui::NotApplicable("per-invocation dry-run action"),
    },
    OptionEntry {
        field: "skip_download",
        gui: Gui::NotApplicable("per-invocation action"),
    },
    OptionEntry {
        field: "extract_playlist",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "playlist_start",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "playlist_end",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "playlist_items",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "username",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "password",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "two_factor",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "netrc",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "cookies_from_browser",
        gui: Gui::Control("cookies_from_browser"),
    },
    OptionEntry {
        field: "cookies_file",
        gui: Gui::Control("cookies_file"),
    },
    OptionEntry {
        field: "download_archive",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "match_filters",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_directories",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "enabled_plugins",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "load_plugins",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_timeout_metadata_ms",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_timeout_extract_s",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_timeout_search_s",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_memory_limit_mb",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_stack_limit_mb",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "plugin_trusted_publishers",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "adaptive_downloads",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.extract_audio",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.audio_format",
        gui: Gui::Control("default_extract_audio"),
    },
    OptionEntry {
        field: "postprocess.audio_quality",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_video",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.remux_container",
        gui: Gui::Control("default_remux"),
    },
    OptionEntry {
        field: "postprocess.merge_output_format",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.embed_thumbnail",
        gui: Gui::Control("embed_thumbnail"),
    },
    OptionEntry {
        field: "postprocess.write_thumbnail",
        gui: Gui::Control("write_thumbnail"),
    },
    OptionEntry {
        field: "postprocess.embed_metadata",
        gui: Gui::Control("embed_metadata"),
    },
    OptionEntry {
        field: "postprocess.embed_subtitles",
        gui: Gui::Control("embed_subtitles"),
    },
    OptionEntry {
        field: "postprocess.write_subtitles",
        gui: Gui::Control("write_subtitles"),
    },
    OptionEntry {
        field: "postprocess.keep_video",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.ffmpeg_location",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.ffmpeg_args",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.normalize_audio",
        gui: Gui::Control("normalize_audio"),
    },
    OptionEntry {
        field: "postprocess.loudnorm",
        gui: Gui::Control("loudnorm"),
    },
    OptionEntry {
        field: "postprocess.audio_gain_target",
        gui: Gui::Control("audio_gain_target"),
    },
    OptionEntry {
        field: "postprocess.loudnorm_preset",
        gui: Gui::Control("loudnorm_preset"),
    },
    OptionEntry {
        field: "postprocess.loudnorm_target_i",
        gui: Gui::Control("loudnorm_target_i"),
    },
    OptionEntry {
        field: "postprocess.loudnorm_target_tp",
        gui: Gui::Control("loudnorm_target_tp"),
    },
    OptionEntry {
        field: "postprocess.loudnorm_target_lra",
        gui: Gui::Control("loudnorm_target_lra"),
    },
    OptionEntry {
        field: "postprocess.loudnorm_dynamic",
        gui: Gui::Control("loudnorm_dynamic"),
    },
    OptionEntry {
        field: "postprocess.loudnorm_precompress",
        gui: Gui::Control("loudnorm_precompress"),
    },
    OptionEntry {
        field: "postprocess.normalize_boost",
        gui: Gui::Control("normalize_boost"),
    },
    OptionEntry {
        field: "postprocess.normalize_boost_db",
        gui: Gui::Control("normalize_boost_db"),
    },
    OptionEntry {
        field: "postprocess.video_encoder",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_audio",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_container",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_threads",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_preset",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_deadline",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_cpu_used",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.recode_speed_level",
        gui: Gui::Missing("#602"),
    },
    OptionEntry {
        field: "postprocess.fixup",
        gui: Gui::Missing("#602"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_types::{Config, PostProcess};
    use std::collections::HashSet;

    /// Mirrors `every_config_field_is_classified` (rdlp-cli): a no-`..` destructure
    /// so a new Config/PostProcess field is E0027 here until it gets a registry entry.
    /// DO NOT ADD `..` — the correct fix for E0027 is a new `OPTION_REGISTRY` entry.
    #[test]
    // Exhaustively destructuring `Config` + `PostProcess` (96 fields total) plus the
    // 1:1 `EXPECTED` mirror is the forcing function this test exists for — splitting it
    // would break the single-canary property. `items_after_statements`: `EXPECTED` sits
    // next to the destructures it mirrors, which is the readable order here.
    #[allow(clippy::too_many_lines, clippy::items_after_statements)]
    fn registry_classifies_every_config_field() {
        let Config {
            output_to_stdout: _,
            output_template: _,
            output_directory: _,
            restrict_filenames: _,
            overwrite: _,
            continue_downloads: _,
            no_part: _,
            format: _,
            audio_multistreams: _,
            concurrent_fragments: _,
            rate_limit: _,
            retries: _,
            fragment_retries: _,
            retry_initial_delay_ms: _,
            retry_max_delay_ms: _,
            retry_backoff_multiplier: _,
            buffer_size: _,
            proxy: _,
            socket_timeout: _,
            read_timeout: _,
            pool_idle_timeout: _,
            download_timeout: _,
            merge_timeout: _,
            hls_head_probe_timeout: _,
            parallel_threshold: _,
            source_address: _,
            user_agent: _,
            browser_emulation: _,
            http_headers: _,
            write_auto_subtitles: _,
            subtitle_langs: _,
            subtitle_format: _,
            list_subs: _,
            strict_subs: _,
            verify_sub_urls: _,
            retry_subs: _,
            quiet: _,
            verbose: _,
            simulate: _,
            skip_download: _,
            extract_playlist: _,
            playlist_start: _,
            playlist_end: _,
            playlist_items: _,
            username: _,
            password: _,
            two_factor: _,
            netrc: _,
            cookies_from_browser: _,
            cookies_file: _,
            download_archive: _,
            match_filters: _,
            plugin_directories: _,
            enabled_plugins: _,
            load_plugins: _,
            plugin_timeout_metadata_ms: _,
            plugin_timeout_extract_s: _,
            plugin_timeout_search_s: _,
            plugin_memory_limit_mb: _,
            plugin_stack_limit_mb: _,
            plugin_trusted_publishers: _,
            adaptive_downloads: _,
            postprocess,
        } = Config::default();
        let PostProcess {
            extract_audio: _,
            audio_format: _,
            audio_quality: _,
            recode_video: _,
            remux_container: _,
            merge_output_format: _,
            embed_thumbnail: _,
            write_thumbnail: _,
            embed_metadata: _,
            embed_subtitles: _,
            write_subtitles: _,
            keep_video: _,
            ffmpeg_location: _,
            ffmpeg_args: _,
            normalize_audio: _,
            loudnorm: _,
            audio_gain_target: _,
            loudnorm_preset: _,
            loudnorm_target_i: _,
            loudnorm_target_tp: _,
            loudnorm_target_lra: _,
            loudnorm_dynamic: _,
            loudnorm_precompress: _,
            normalize_boost: _,
            normalize_boost_db: _,
            video_encoder: _,
            recode_audio: _,
            recode_container: _,
            recode_threads: _,
            recode_preset: _,
            recode_deadline: _,
            recode_cpu_used: _,
            recode_speed_level: _,
            fixup: _,
        } = postprocess;

        // The full set of expected field paths (must match the destructure above 1:1).
        const EXPECTED: &[&str] = &[
            // top-level Config
            "output_to_stdout",
            "output_template",
            "output_directory",
            "restrict_filenames",
            "overwrite",
            "continue_downloads",
            "no_part",
            "format",
            "audio_multistreams",
            "concurrent_fragments",
            "rate_limit",
            "retries",
            "fragment_retries",
            "retry_initial_delay_ms",
            "retry_max_delay_ms",
            "retry_backoff_multiplier",
            "buffer_size",
            "proxy",
            "socket_timeout",
            "read_timeout",
            "pool_idle_timeout",
            "download_timeout",
            "merge_timeout",
            "hls_head_probe_timeout",
            "parallel_threshold",
            "source_address",
            "user_agent",
            "browser_emulation",
            "http_headers",
            "write_auto_subtitles",
            "subtitle_langs",
            "subtitle_format",
            "list_subs",
            "strict_subs",
            "verify_sub_urls",
            "retry_subs",
            "quiet",
            "verbose",
            "simulate",
            "skip_download",
            "extract_playlist",
            "playlist_start",
            "playlist_end",
            "playlist_items",
            "username",
            "password",
            "two_factor",
            "netrc",
            "cookies_from_browser",
            "cookies_file",
            "download_archive",
            "match_filters",
            "plugin_directories",
            "enabled_plugins",
            "load_plugins",
            "plugin_timeout_metadata_ms",
            "plugin_timeout_extract_s",
            "plugin_timeout_search_s",
            "plugin_memory_limit_mb",
            "plugin_stack_limit_mb",
            "plugin_trusted_publishers",
            "adaptive_downloads",
            // PostProcess (prefixed)
            "postprocess.extract_audio",
            "postprocess.audio_format",
            "postprocess.audio_quality",
            "postprocess.recode_video",
            "postprocess.remux_container",
            "postprocess.merge_output_format",
            "postprocess.embed_thumbnail",
            "postprocess.write_thumbnail",
            "postprocess.embed_metadata",
            "postprocess.embed_subtitles",
            "postprocess.write_subtitles",
            "postprocess.keep_video",
            "postprocess.ffmpeg_location",
            "postprocess.ffmpeg_args",
            "postprocess.normalize_audio",
            "postprocess.loudnorm",
            "postprocess.audio_gain_target",
            "postprocess.loudnorm_preset",
            "postprocess.loudnorm_target_i",
            "postprocess.loudnorm_target_tp",
            "postprocess.loudnorm_target_lra",
            "postprocess.loudnorm_dynamic",
            "postprocess.loudnorm_precompress",
            "postprocess.normalize_boost",
            "postprocess.normalize_boost_db",
            "postprocess.video_encoder",
            "postprocess.recode_audio",
            "postprocess.recode_container",
            "postprocess.recode_threads",
            "postprocess.recode_preset",
            "postprocess.recode_deadline",
            "postprocess.recode_cpu_used",
            "postprocess.recode_speed_level",
            "postprocess.fixup",
        ];

        let registry: HashSet<&str> = OPTION_REGISTRY.iter().map(|e| e.field).collect();
        assert_eq!(
            registry.len(),
            OPTION_REGISTRY.len(),
            "duplicate field in OPTION_REGISTRY"
        );
        assert_eq!(
            OPTION_REGISTRY.len(),
            96,
            "registry must have exactly 96 entries"
        );
        let expected: HashSet<&str> = EXPECTED.iter().copied().collect();
        assert_eq!(expected.len(), 96, "EXPECTED drifted from 96");
        assert_eq!(
            registry, expected,
            "OPTION_REGISTRY fields must exactly match Config+PostProcess"
        );
    }

    #[test]
    fn registry_gui_bucket_counts_match_design() {
        let (mut control, mut na, mut missing) = (0, 0, 0);
        for e in OPTION_REGISTRY {
            match e.gui {
                Gui::Control(_) => control += 1,
                Gui::NotApplicable(_) => na += 1,
                Gui::Missing(_) => missing += 1,
            }
        }
        assert_eq!(control, 36, "Control count drifted");
        assert_eq!(na, 5, "NotApplicable count drifted");
        assert_eq!(missing, 55, "Missing count drifted");
    }

    #[test]
    fn every_control_names_a_distinct_appsettings_field() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for e in OPTION_REGISTRY {
            if let Gui::Control(f) = e.gui {
                assert!(
                    seen.insert(f),
                    "two registry entries bind the same AppSettings field `{f}`"
                );
            }
        }
    }
}
