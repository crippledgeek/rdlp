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

/// Maximum width for an option's value placeholder. A wider one inflates clap's
/// shared `longest` column and flips the whole option block to next-line
/// rendering (verified: `clap_builder` `help_template.rs` `will_args_wrap`) — the
/// measured cause of the pre-PR-3 239-line `--help`.
const MAX_VALUE_NAME_LEN: usize = 12;

#[test]
fn every_value_flag_has_a_tight_explicit_placeholder() {
    let cmd = Args::command();
    let mut checked = 0usize;

    for arg in cmd.get_arguments() {
        let id = arg.get_id().as_str();
        // Skip positionals (e.g. `url` — its `<URL>` echo reads fine), clap's
        // auto help/version, and flags that take no value (bools/counters
        // render no placeholder).
        if arg.is_positional()
            || id == "help"
            || id == "version"
            || !arg.get_action().takes_values()
        {
            continue;
        }

        let names: Vec<&str> = arg
            .get_value_names()
            .map(|ns| ns.iter().map(std::convert::AsRef::as_ref).collect())
            .unwrap_or_default();

        assert!(
            !names.is_empty(),
            "value flag `{id}` has no value_name (clap would echo its \
             field name). Add `value_name = \"…\"` (#585 PR 3).",
        );

        for name in names {
            assert_ne!(
                name,
                id.to_uppercase(),
                "value flag `{id}` still renders clap's inferred `<{}>` \
                 placeholder — add an explicit tight `value_name` (#585 PR 3).",
                id.to_uppercase(),
            );
            assert!(
                name.len() <= MAX_VALUE_NAME_LEN,
                "value_name `<{name}>` for `{id}` is {} chars (> {MAX_VALUE_NAME_LEN}); \
                 a wide placeholder flips the whole help block to next-line rendering.",
                name.len(),
            );
        }
        checked += 1;
    }

    // Sanity: 29 annotated in this PR + 14 pre-existing tight value_names = 43.
    assert_eq!(
        checked, 43,
        "expected 43 value-taking flags; got {checked} — a flag was added or removed",
    );
}

#[test]
fn examples_lead_the_help_before_options() {
    let short = Args::command().render_help().to_string();
    let long = Args::command().render_long_help().to_string();

    for (label, help) in [("-h", &short), ("--help", &long)] {
        // The examples block renders…
        let ex = help
            .find("Examples:")
            .unwrap_or_else(|| panic!("`{label}` help is missing the Examples block"));
        // …after the about line…
        let about = help
            .find("Rust Download Program")
            .unwrap_or_else(|| panic!("`{label}` help is missing the about line"));
        // …before the first option heading (General) and before the Usage line.
        let general = help
            .find("General")
            .unwrap_or_else(|| panic!("`{label}` help is missing the General heading"));
        let usage = help
            .find("Usage:")
            .unwrap_or_else(|| panic!("`{label}` help is missing the Usage line"));

        assert!(about < ex, "`{label}`: about should precede Examples");
        assert!(ex < usage, "`{label}`: Examples should precede Usage");
        assert!(
            ex < general,
            "`{label}`: Examples should precede the option list"
        );
        // A representative example command is present.
        assert!(
            help.contains("--cookies-from-browser chrome"),
            "`{label}` help is missing a representative example line",
        );
    }
}

#[test]
fn help_examples_lines_fit_80_columns() {
    // 80 cols is the conventional terminal default; a longer example line
    // hard-wraps and breaks the aligned Examples table. clap does not
    // re-indent wrapped before_help continuation lines.
    for line in super::HELP_EXAMPLES.lines() {
        assert!(
            line.chars().count() <= 78,
            "example line is {} chars (>78, wraps at 80): {line:?}",
            line.chars().count(),
        );
    }
}

/// Ids visible in `-h` (short help). Everything else that takes part in tiering
/// is EXPERT and hidden from `-h` (still shown in `--help`). Keep in sync with
/// the `hide_short_help = true` annotations in args.rs.
const HELP_SHORT_COMMON: &[&str] = &[
    // General
    "output",
    "output_dir",
    "format",
    "quiet",
    "verbose",
    "interactive",
    // Simulation & Info
    "list_formats",
    "simulate",
    "dump_json",
    "print",
    // Post-Processing
    "extract_audio",
    "remux",
    "embed_metadata",
    "keep_video",
    // Subtitles
    "embed_subtitles",
    "sub_langs",
    "write_subtitles",
    // Recode & Encoding
    "recode_video",
    "recode_audio",
    // Audio Normalization
    "normalize_audio",
    "loudnorm",
    // Network & Cookies
    "cookies_from_browser",
    "cookies",
    "proxy",
    // Download Behaviour
    "limit_rate",
    "download_archive",
    // Search
    "search",
    "search_site",
    // Configuration & Plugins
    "config_location",
];

/// Ids hidden from `-h` (present only in `--help`).
const HELP_SHORT_EXPERT: &[&str] = &[
    // General
    "audio_multistreams",
    // Simulation & Info
    "list_extractors",
    "list_downloaders",
    "list_codecs",
    "list_encoders",
    // Post-Processing
    "audio_format",
    "audio_quality",
    "no_thumbnail",
    "write_thumbnail",
    "fixup",
    "ffmpeg_location",
    // Subtitles
    "write_auto_subtitles",
    "sub_format",
    "list_subs",
    "list_subs_only",
    "strict_subs",
    "verify_sub_urls",
    "retry_subs",
    // Recode & Encoding
    "video_encoder",
    "recode_container",
    "recode_threads",
    "recode_preset",
    "recode_deadline",
    "recode_cpu_used",
    "recode_speed_level",
    // Audio Normalization
    "audio_gain_target",
    "loudnorm_preset",
    "loudnorm_i",
    "loudnorm_tp",
    "loudnorm_lra",
    "loudnorm_dynamic",
    "loudnorm_precompress",
    "normalize_boost",
    "normalize_boost_db",
    // Network & Cookies
    "socket_timeout",
    "read_timeout",
    "pool_idle_timeout",
    "download_timeout",
    "merge_timeout",
    "browser",
    // Download Behaviour
    "match_filter",
    // Search
    "search_filter",
    // Configuration & Plugins
    "ignore_config",
    "trust_publisher",
];

#[test]
fn every_option_is_tiered_common_or_expert() {
    use std::collections::HashSet;

    assert_eq!(HELP_SHORT_COMMON.len(), 29, "common set drifted");
    assert_eq!(HELP_SHORT_EXPERT.len(), 44, "expert set drifted");
    let common: HashSet<&str> = HELP_SHORT_COMMON.iter().copied().collect();
    let expert: HashSet<&str> = HELP_SHORT_EXPERT.iter().copied().collect();
    assert!(
        common.is_disjoint(&expert),
        "an id is in both COMMON and EXPERT"
    );

    let cmd = Args::command();
    let mut seen = HashSet::new();

    for arg in cmd.get_arguments() {
        let id = arg.get_id().as_str();
        if arg.is_positional() || id == "help" || id == "version" {
            continue;
        }
        seen.insert(id.to_owned());
        let hidden = arg.is_hide_short_help_set();
        if common.contains(id) {
            assert!(
                !hidden,
                "`{id}` is COMMON but is hidden from -h (drop its hide_short_help)"
            );
        } else if expert.contains(id) {
            assert!(
                hidden,
                "`{id}` is EXPERT but visible in -h (add hide_short_help = true)"
            );
        } else {
            panic!(
                "flag `{id}` is not tiered. Add it to HELP_SHORT_COMMON (visible in -h) or \
                 HELP_SHORT_EXPERT + `hide_short_help = true` (—help only). #585 PR 3b.",
            );
        }
    }

    for id in HELP_SHORT_COMMON.iter().chain(HELP_SHORT_EXPERT) {
        assert!(
            seen.contains(*id),
            "tier map lists `{id}`, not a real Args option"
        );
    }
}

#[test]
fn short_help_footer_fits_80_columns() {
    // clap wraps after_help at terminal width; keep the -h footer on one line.
    assert!(
        super::HELP_SHORT_FOOTER.chars().count() <= 78,
        "HELP_SHORT_FOOTER is {} chars (>78, wraps at 80)",
        super::HELP_SHORT_FOOTER.chars().count(),
    );
}

#[test]
fn short_help_footer_only_in_short_help() {
    let short = Args::command().render_help().to_string();
    let long = Args::command().render_long_help().to_string();
    assert!(
        short.contains("Run 'rdlp --help' for the full list"),
        "`-h` output is missing the footer pointing to --help",
    );
    assert!(
        !long.contains("Run 'rdlp --help' for the full list"),
        "`--help` should NOT carry the short-help footer (it already lists everything)",
    );
    // The empty `after_long_help` must not leak a trailing blank block into --help.
    assert!(
        !long.ends_with("\n\n"),
        "--help gained a stray trailing blank line from after_long_help = \"\"",
    );
}

#[test]
fn short_flags_k_and_o_parse() {
    use clap::Parser;

    // -k => keep_video (yt-dlp parity)
    let a = Args::try_parse_from(["rdlp", "-k", "https://example.com/v"]).expect("-k should parse");
    assert!(a.keep_video, "-k must set keep_video");

    // -O => print (uppercase O; coexists with lowercase -o = --output, curl precedent)
    let b = Args::try_parse_from(["rdlp", "-O", "title", "https://example.com/v"])
        .expect("-O should parse");
    assert_eq!(b.print.as_deref(), Some("title"), "-O must set print");

    // -o (lowercase) still means --output, not print
    let c = Args::try_parse_from(["rdlp", "-o", "out.mp4", "https://example.com/v"])
        .expect("-o should parse");
    assert_eq!(
        c.output.as_deref(),
        Some("out.mp4"),
        "-o must still be --output"
    );
    assert_eq!(c.print, None);
}

#[test]
fn renamed_and_removed_sub_aliases_still_parse() {
    use clap::Parser;

    // Removed no-op aliases: the derived long name still works (value-taking).
    assert!(
        Args::try_parse_from(["rdlp", "--sub-langs", "en", "u"]).is_ok(),
        "--sub-langs must still parse"
    );
    assert!(
        Args::try_parse_from(["rdlp", "--sub-format", "srt", "u"]).is_ok(),
        "--sub-format must still parse"
    );
    // Removed no-op aliases: the derived long name still works (bool flags).
    assert!(
        Args::try_parse_from(["rdlp", "--list-subs", "u"]).is_ok(),
        "--list-subs must still parse"
    );
    assert!(
        Args::try_parse_from(["rdlp", "--list-subs-only", "u"]).is_ok(),
        "--list-subs-only must still parse"
    );

    // Kept real aliases (yt-dlp spellings) still parse.
    assert!(
        Args::try_parse_from(["rdlp", "--embed-subs", "u"]).is_ok(),
        "--embed-subs alias kept"
    );
    assert!(
        Args::try_parse_from(["rdlp", "--write-subs", "u"]).is_ok(),
        "--write-subs alias kept"
    );
    assert!(
        Args::try_parse_from(["rdlp", "--write-auto-subs", "u"]).is_ok(),
        "--write-auto-subs alias kept"
    );
}

#[test]
fn no_arg_aliases_its_own_long_name() {
    // A hidden alias identical to the arg's own long name is a no-op (clap matches
    // aliases against registered names). Guard against re-introducing the dead
    // aliases this PR removed. Would fail on develop pre-PR (4 no-op aliases existed).
    let cmd = Args::command();
    for arg in cmd.get_arguments() {
        let Some(long) = arg.get_long() else { continue };
        if let Some(aliases) = arg.get_all_aliases() {
            for a in aliases {
                assert_ne!(
                    a,
                    long,
                    "arg `{}` has a no-op alias equal to its long name `{long}`",
                    arg.get_id(),
                );
            }
        }
    }
}
