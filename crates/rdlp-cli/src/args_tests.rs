//! Structural test for #585 PR 2: every CLI option must be classified into
//! exactly one `--help` group. A new flag with no `help_heading` (or a wrong
//! one) fails here — the CLI-side analog of the merge exhaustiveness canary.

use std::collections::HashMap;

use clap::CommandFactory;

use crate::args::{
    Args, HELP_HEADING_AUDIO_NORM, HELP_HEADING_CONFIG, HELP_HEADING_DOWNLOAD,
    HELP_HEADING_GENERAL, HELP_HEADING_INFO, HELP_HEADING_NETWORK, HELP_HEADING_POSTPROCESS,
    HELP_HEADING_RECODE, HELP_HEADING_SEARCH, HELP_HEADING_SUBTITLES,
};

/// Arg id (== Rust field name for clap-derive) → expected heading. This IS the
/// group design; keep it in sync with the field annotations. `url` (positional)
/// and `plugin` (subcommand) are intentionally absent.
const EXPECTED: &[(&str, &str)] = &[
    // General
    ("output", HELP_HEADING_GENERAL),
    ("output_dir", HELP_HEADING_GENERAL),
    ("format", HELP_HEADING_GENERAL),
    ("audio_multistreams", HELP_HEADING_GENERAL),
    ("quiet", HELP_HEADING_GENERAL),
    ("verbose", HELP_HEADING_GENERAL),
    ("interactive", HELP_HEADING_GENERAL),
    // Simulation & Info
    ("list_extractors", HELP_HEADING_INFO),
    ("list_downloaders", HELP_HEADING_INFO),
    ("list_codecs", HELP_HEADING_INFO),
    ("list_encoders", HELP_HEADING_INFO),
    ("simulate", HELP_HEADING_INFO),
    ("dump_json", HELP_HEADING_INFO),
    ("list_formats", HELP_HEADING_INFO),
    ("print", HELP_HEADING_INFO),
    // Post-Processing
    ("extract_audio", HELP_HEADING_POSTPROCESS),
    ("audio_format", HELP_HEADING_POSTPROCESS),
    ("audio_quality", HELP_HEADING_POSTPROCESS),
    ("embed_metadata", HELP_HEADING_POSTPROCESS),
    ("no_thumbnail", HELP_HEADING_POSTPROCESS),
    ("write_thumbnail", HELP_HEADING_POSTPROCESS),
    ("remux", HELP_HEADING_POSTPROCESS),
    ("fixup", HELP_HEADING_POSTPROCESS),
    ("keep_video", HELP_HEADING_POSTPROCESS),
    ("ffmpeg_location", HELP_HEADING_POSTPROCESS),
    // Subtitles
    ("write_subtitles", HELP_HEADING_SUBTITLES),
    ("write_auto_subtitles", HELP_HEADING_SUBTITLES),
    ("sub_langs", HELP_HEADING_SUBTITLES),
    ("sub_format", HELP_HEADING_SUBTITLES),
    ("embed_subtitles", HELP_HEADING_SUBTITLES),
    ("list_subs", HELP_HEADING_SUBTITLES),
    ("list_subs_only", HELP_HEADING_SUBTITLES),
    ("strict_subs", HELP_HEADING_SUBTITLES),
    ("verify_sub_urls", HELP_HEADING_SUBTITLES),
    ("retry_subs", HELP_HEADING_SUBTITLES),
    // Recode & Encoding
    ("video_encoder", HELP_HEADING_RECODE),
    ("recode_video", HELP_HEADING_RECODE),
    ("recode_container", HELP_HEADING_RECODE),
    ("recode_audio", HELP_HEADING_RECODE),
    ("recode_threads", HELP_HEADING_RECODE),
    ("recode_preset", HELP_HEADING_RECODE),
    ("recode_deadline", HELP_HEADING_RECODE),
    ("recode_cpu_used", HELP_HEADING_RECODE),
    ("recode_speed_level", HELP_HEADING_RECODE),
    // Audio Normalization
    ("normalize_audio", HELP_HEADING_AUDIO_NORM),
    ("loudnorm", HELP_HEADING_AUDIO_NORM),
    ("audio_gain_target", HELP_HEADING_AUDIO_NORM),
    ("loudnorm_preset", HELP_HEADING_AUDIO_NORM),
    ("loudnorm_i", HELP_HEADING_AUDIO_NORM),
    ("loudnorm_tp", HELP_HEADING_AUDIO_NORM),
    ("loudnorm_lra", HELP_HEADING_AUDIO_NORM),
    ("loudnorm_dynamic", HELP_HEADING_AUDIO_NORM),
    ("loudnorm_precompress", HELP_HEADING_AUDIO_NORM),
    ("normalize_boost", HELP_HEADING_AUDIO_NORM),
    ("normalize_boost_db", HELP_HEADING_AUDIO_NORM),
    // Network & Cookies
    ("proxy", HELP_HEADING_NETWORK),
    ("socket_timeout", HELP_HEADING_NETWORK),
    ("read_timeout", HELP_HEADING_NETWORK),
    ("pool_idle_timeout", HELP_HEADING_NETWORK),
    ("download_timeout", HELP_HEADING_NETWORK),
    ("merge_timeout", HELP_HEADING_NETWORK),
    ("browser", HELP_HEADING_NETWORK),
    ("cookies_from_browser", HELP_HEADING_NETWORK),
    ("cookies", HELP_HEADING_NETWORK),
    // Download Behaviour
    ("limit_rate", HELP_HEADING_DOWNLOAD),
    ("download_archive", HELP_HEADING_DOWNLOAD),
    ("match_filter", HELP_HEADING_DOWNLOAD),
    // Search
    ("search", HELP_HEADING_SEARCH),
    ("search_site", HELP_HEADING_SEARCH),
    ("search_filter", HELP_HEADING_SEARCH),
    // Configuration & Plugins
    ("ignore_config", HELP_HEADING_CONFIG),
    ("config_location", HELP_HEADING_CONFIG),
    ("trust_publisher", HELP_HEADING_CONFIG),
];

/// Ids present as clap args but intentionally NOT grouped.
const EXEMPT: &[&str] = &["help", "version", "url", "plugin"];

#[test]
fn every_option_is_classified_into_a_help_group() {
    let expected: HashMap<&str, &str> = EXPECTED.iter().copied().collect();
    assert_eq!(expected.len(), EXPECTED.len(), "duplicate id in EXPECTED");
    assert_eq!(
        EXPECTED.len(),
        73,
        "group map should cover all 73 option fields"
    );

    let cmd = Args::command();
    let mut seen = std::collections::HashSet::new();

    for arg in cmd.get_arguments() {
        let id = arg.get_id().as_str();
        if EXEMPT.contains(&id) || arg.is_positional() {
            continue;
        }
        seen.insert(id.to_owned());
        match expected.get(id) {
            Some(&heading) => assert_eq!(
                arg.get_help_heading(),
                Some(heading),
                "flag `{id}` is in the wrong help group",
            ),
            None => panic!(
                "flag `{id}` is not classified into a help group. Add it to \
                 EXPECTED in args_tests.rs and annotate the field with \
                 `help_heading = HELP_HEADING_<GROUP>` (residue rule, #585 PR 2).",
            ),
        }
    }

    // Every mapped id must correspond to a real arg (catch stale map entries).
    for (id, _) in EXPECTED {
        assert!(
            seen.contains(*id),
            "EXPECTED lists `{id}`, which is not an Args option"
        );
    }
}

/// All 10 group headings, for asserting they actually render in emitted help
/// text (the structural test above only inspects the `Command` model).
const ALL_HEADINGS: &[&str] = &[
    HELP_HEADING_GENERAL,
    HELP_HEADING_INFO,
    HELP_HEADING_POSTPROCESS,
    HELP_HEADING_SUBTITLES,
    HELP_HEADING_RECODE,
    HELP_HEADING_AUDIO_NORM,
    HELP_HEADING_NETWORK,
    HELP_HEADING_DOWNLOAD,
    HELP_HEADING_SEARCH,
    HELP_HEADING_CONFIG,
];

#[test]
fn all_headings_render_in_short_and_long_help() {
    let short = Args::command().render_help().to_string();
    let long = Args::command().render_long_help().to_string();
    for heading in ALL_HEADINGS {
        assert!(
            short.contains(heading),
            "`-h` output is missing heading `{heading}`"
        );
        assert!(
            long.contains(heading),
            "`--help` output is missing heading `{heading}`"
        );
    }
}
