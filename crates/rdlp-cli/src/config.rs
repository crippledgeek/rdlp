//! Configuration building and merging for CLI.
//!
//! Resolves interactive prompts, loads config files, and merges
//! CLI arguments with file-based and default configuration using
//! a three-layer precedence: CLI > config file > defaults.

use anyhow::{Context, Result};
use rdlp_api::{
    AudioFormat, BrowserEmulation, BrowserType, Config, ContainerFormat, FixupPolicy,
    RecodeAudioMode, SubtitleFormat, config_io,
};

use crate::args::Args;
use crate::selection::{select_audio_format, select_recode_video, select_remux_container};

/// Resolved values from interactive CLI prompts.
///
/// These are resolved BEFORE the pure config merge so that `merge_config()`
/// remains free of side effects and is testable.
#[derive(Copy, Clone)]
pub struct ResolvedInteractiveValues {
    pub audio_format: Option<AudioFormat>,
    pub recode_video: Option<ContainerFormat>,
    pub remux_container: Option<ContainerFormat>,
}

/// Resolve interactive CLI values (inquire prompts) before config merge.
///
/// Returns `None` values for fields that weren't set to "interactive".
/// Parse a `--browser` / `RDLP_BROWSER_EMULATION` value into a
/// `BrowserEmulation`. Recognised shorthands: `chrome-latest`,
/// `firefox-latest`, `safari-latest`. Anything else is treated as a
/// pinned profile identifier; invalid pins fall back to Chrome-latest
/// at resolve time.
fn parse_browser_emulation(s: &str) -> BrowserEmulation {
    match s {
        "chrome-latest" => BrowserEmulation::ChromeLatest,
        "firefox-latest" => BrowserEmulation::FirefoxLatest,
        "safari-latest" => BrowserEmulation::SafariLatest,
        other => BrowserEmulation::Pinned(other.to_string()),
    }
}

fn resolve_interactive_values(args: &Args) -> Result<ResolvedInteractiveValues> {
    let audio_format = match args.audio_format.as_deref() {
        Some("interactive") => {
            select_audio_format().context("interactive audio format selection failed")?
        }
        _ => None,
    };

    let recode_video = match args.recode_video.as_deref() {
        Some("interactive") => {
            select_recode_video().context("interactive recode video format selection failed")?
        }
        _ => None,
    };

    let remux_container = match args.remux.as_deref() {
        Some("interactive") => {
            select_remux_container().context("interactive remux container selection failed")?
        }
        _ => None,
    };

    Ok(ResolvedInteractiveValues {
        audio_format,
        recode_video,
        remux_container,
    })
}

/// Merge a declaration list of uniform CLI-arg-to-config-field assignments.
///
/// Each entry names ONE field with an explicit arm keyword, so a same-typed
/// transposition (assigning the wrong field, or reading the wrong arg) is a
/// compile error or an obviously-wrong line, not a copy-paste footgun hiding
/// in two near-identical macro invocations. Replaces the former `merge_opt!`
/// / `merge_bool!` macros (see #540, #585).
///
/// An omitted flag is `None`/`false`, so a bare assignment would silently
/// discard whatever the config file set — the `opt`/`opt_pp` arms guard with
/// `if let Some(..)`, `bool`/`bool_pp` only set `true` (never reset to
/// `false`), and `set` guards a plain (non-`Option`) field the same way.
///
/// Arms:
/// - `bool: FIELD` — `if args.FIELD { config.FIELD = true; }`
/// - `bool_pp: FIELD` — same, under `config.postprocess`
/// - `opt: FIELD` — `if let Some(v) = args.FIELD.clone() { config.FIELD = Some(v); }`
/// - `opt_pp: FIELD` — same, under `config.postprocess`
/// - `set: FIELD <- ARG` — `if let Some(v) = args.ARG.clone() { config.FIELD = v; }`
///   (config field is NOT `Option`-typed; equivalent to `.clone_from(&v)`)
/// - `tribool_pp: FIELD <- NEG` — tri-state negatable bool under
///   `config.postprocess`: `if let Some(v) = flag(args.FIELD, args.NEG) {
///   config.postprocess.FIELD = v; }`. `FIELD` is the positive flag+config
///   field; `NEG` is the `--no-*` arg. Absent pair leaves the config value.
///
/// Only `opt:` and `set:` support an optional/required `<- arg_name` arrow for
/// when the CLI arg name differs from the config field name (e.g.
/// `opt: cookies_file <- cookies`). It is optional on `opt:` (defaults to the
/// field name) but required on `set:` (no bare `set: FIELD` arm exists).
/// `bool:`, `bool_pp:`, and `opt_pp:` have no arrow variant — they always read
/// `args.FIELD` under the same name as the config field.
macro_rules! merge_fields {
    ($config:expr, $args:expr, { $($arm:ident : $field:ident $(<- $arg:ident)?),+ $(,)? }) => {
        $(
            merge_fields!(@arm $config, $args, $arm, $field $(<- $arg)?);
        )+
    };
    (@arm $config:expr, $args:expr, bool, $field:ident) => {
        if $args.$field {
            $config.$field = true;
        }
    };
    (@arm $config:expr, $args:expr, bool_pp, $field:ident) => {
        if $args.$field {
            $config.postprocess.$field = true;
        }
    };
    (@arm $config:expr, $args:expr, opt, $field:ident) => {
        if let Some(value) = $args.$field.clone() {
            $config.$field = Some(value);
        }
    };
    (@arm $config:expr, $args:expr, opt, $field:ident <- $arg:ident) => {
        if let Some(value) = $args.$arg.clone() {
            $config.$field = Some(value);
        }
    };
    (@arm $config:expr, $args:expr, opt_pp, $field:ident) => {
        if let Some(value) = $args.$field.clone() {
            $config.postprocess.$field = Some(value);
        }
    };
    (@arm $config:expr, $args:expr, set, $field:ident <- $arg:ident) => {
        if let Some(value) = $args.$arg.clone() {
            $config.$field = value;
        }
    };
    (@arm $config:expr, $args:expr, tribool_pp, $field:ident <- $neg:ident) => {
        if let Some(value) = flag($args.$field, $args.$neg) {
            $config.postprocess.$field = value;
        }
    };
}

/// Resolves a `--foo` / `--no-foo` boolean pair to a tri-state.
///
/// clap's `overrides_with` wiring makes the pair mutually exclusive with POSIX
/// last-wins semantics, so at most one of `yes`/`no` is `true` after parsing.
/// Returns `None` when neither flag was passed, so an absent pair leaves the
/// config-file / default value untouched at the merge boundary — the same
/// absent-vs-`false` distinction the `opt:` arms preserve (#540/#583). A plain
/// `bool` on the merge-facing path would collapse absent and `false` and
/// silently clobber a config-file value, the anti-pattern this replaces.
const fn flag(yes: bool, no: bool) -> Option<bool> {
    if yes {
        Some(true)
    } else if no {
        Some(false)
    } else {
        None
    }
}

/// Parses a CLI string into a typed value, attaching the context every call
/// site used to repeat by hand.
///
/// Replaces seven near-identical `parse::<T>() + map_err + with_context`
/// blocks whose only differences were the target type and the noun.
///
/// Uses `anyhow::Error::from` rather than a `Display`-formatted `anyhow!`, so
/// this is byte-identical to the `anyhow::anyhow!(e)` each call site used to
/// write: it preserves the source error's `source()` chain instead of
/// flattening it to a string.
fn parse_arg<T>(raw: &str, what: &str) -> Result<T>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::error::Error + Send + Sync + 'static,
{
    raw.parse::<T>()
        .map_err(anyhow::Error::from)
        .with_context(|| format!("invalid {what} '{raw}'"))
}

/// Pure config merge: defaults < config file < CLI args.
///
/// This function has no side effects (no interactive prompts, no I/O).
/// Interactive values must be pre-resolved via `resolve_interactive_values()`.
///
/// # Errors
///
/// Returns an error if any CLI argument fails to parse (invalid format string,
/// container name, subtitle format, etc.) or if the resulting config fails
/// validation.
#[allow(clippy::too_many_lines)] // flat merge of 50+ CLI flags; extracting sub-functions would add indirection without clarity
pub fn merge_config(
    args: &Args,
    file_config: Config,
    interactive_values: ResolvedInteractiveValues,
) -> Result<Config> {
    let mut config = file_config;

    merge_fields!(config, args, {
        set: output_directory <- output_dir,
        opt: format,
        bool: audio_multistreams,
        bool: quiet,
        bool: verbose,
        bool: simulate,
        bool_pp: extract_audio,
        opt_pp: audio_quality,
        tribool_pp: embed_metadata <- no_embed_metadata,
        tribool_pp: embed_thumbnail <- no_embed_thumbnail,
        tribool_pp: write_thumbnail <- no_write_thumbnail,
        bool_pp: write_subtitles,
        bool: write_auto_subtitles,
        tribool_pp: embed_subtitles <- no_embed_subtitles,
        bool: strict_subs,
        bool: verify_sub_urls,
        bool: retry_subs,
        opt_pp: video_encoder,
        opt_pp: recode_threads,
        opt_pp: recode_preset,
        opt_pp: recode_cpu_used,
        opt_pp: recode_speed_level,
        bool_pp: loudnorm_dynamic,
        bool_pp: loudnorm_precompress,
        opt_pp: loudnorm_preset,
        bool_pp: keep_video,
        opt_pp: ffmpeg_location,
        opt: socket_timeout,
        opt: read_timeout,
        opt: pool_idle_timeout,
        opt: download_timeout,
        opt: merge_timeout,
        opt: cookies_file <- cookies,
        opt: download_archive,
    });

    // === Exceptions: post-merge invariant restoration ===
    //
    // These are deliberately NOT in `merge_fields!`. Each depends on the
    // merged value of another field, or combines layers rather than
    // overwriting, so it cannot be expressed as an independent per-field
    // rule. They run AFTER the mechanical pass so they see final values.
    //
    // Every field touched here must also appear in the canary
    // (`every_config_field_is_classified`) — the canary is what proves the
    // declared set and this set together cover all 97 fields.

    // Output: -o - means stdout streaming
    if args.output.as_deref() == Some("-") {
        config.output_to_stdout = true;
        // Force quiet mode — progress/log output would corrupt the byte stream.
        // `quiet` is what actually suppresses the progress bar: the CLI event
        // handler is constructed from it directly (see `main.rs`).
        config.quiet = true;
        // Disable embed_thumbnail before validation: the default is `true`, and
        // Config::validate() would reject stdout + embed-thumbnail.
        if config.postprocess.embed_thumbnail {
            // Use eprintln, not warn!(): the tracing subscriber is not
            // initialised yet (it happens after build_config returns).
            eprintln!("Warning: disabling --embed-thumbnail (incompatible with -o -)");
            config.postprocess.embed_thumbnail = false;
        }
    } else if let Some(ref output) = args.output {
        config.output_template.clone_from(output);
    }

    // Audio format: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.audio_format {
        config.postprocess.audio_format = Some(fmt);
    } else if let Some(audio_format) = args.audio_format.as_deref()
        && audio_format != "interactive"
    {
        config.postprocess.audio_format =
            Some(parse_arg::<AudioFormat>(audio_format, "audio format")?);
    }

    // Subtitles
    if let Some(ref langs) = args.sub_langs {
        config.subtitle_langs = langs.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Some(ref format) = args.sub_format {
        config.subtitle_format = Some(parse_arg::<SubtitleFormat>(format, "subtitle format")?);
    }
    // --list-subs implies --write-subtitles
    if args.list_subs || args.list_subs_only {
        config.postprocess.write_subtitles = true;
        config.list_subs = true;
    }
    // --embed-subtitles implies --write-subtitles for the download step
    if config.postprocess.embed_subtitles && !config.postprocess.write_subtitles {
        config.postprocess.write_subtitles = true;
    }
    // Recode video: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.recode_video {
        config.postprocess.recode_video = Some(fmt);
    } else if let Some(recode_video) = args.recode_video.as_deref()
        && recode_video != "interactive"
    {
        config.postprocess.recode_video = Some(parse_arg::<ContainerFormat>(
            recode_video,
            "recode video container",
        )?);
    }

    // recode_container: explicit container for recode (overrides recode_video container)
    if let Some(ref fmt) = args.recode_container {
        config.postprocess.recode_container =
            Some(parse_arg::<ContainerFormat>(fmt, "recode container")?);
    }

    if let Some(ref deadline) = args.recode_deadline {
        config.postprocess.recode_deadline = Some(
            deadline
                .parse::<rdlp_types::VpxDeadline>()
                .map_err(|e| anyhow::anyhow!("invalid --recode-deadline: {e}"))?,
        );
    }
    // Assigned only when the flag is present, so a `recode_audio` set in the
    // config file survives an invocation that does not mention it (#540). The
    // vocabulary itself is `RecodeAudioMode`'s to define, not the CLI's.
    if let Some(recode_audio) = &args.recode_audio {
        config.postprocess.recode_audio = RecodeAudioMode::from(recode_audio.as_str());
    }

    // Audio normalization: --normalize-boost / --normalize-boost-db implies --loudnorm
    // implies --normalize-audio
    let boost_enabled = args.normalize_boost || args.normalize_boost_db.is_some();
    if args.normalize_audio || args.loudnorm || boost_enabled {
        config.postprocess.normalize_audio = true;
    }
    if args.loudnorm || boost_enabled {
        config.postprocess.loudnorm = true;
    }
    if boost_enabled {
        config.postprocess.normalize_boost = true;
    }
    if let Some(db) = args.normalize_boost_db {
        config.postprocess.normalize_boost_db = Some(db);
    }
    if let Some(target) = args.audio_gain_target {
        config.postprocess.audio_gain_target = Some(target);
    }
    if let Some(i) = args.loudnorm_i {
        config.postprocess.loudnorm_target_i = Some(i);
    }
    if let Some(tp) = args.loudnorm_tp {
        config.postprocess.loudnorm_target_tp = Some(tp);
    }
    if let Some(lra) = args.loudnorm_lra {
        config.postprocess.loudnorm_target_lra = Some(lra);
    }
    // Fixup policy — assign only when the flag was actually passed, so an
    // explicit `--fixup=detect_or_warn` still beats a config-file value (#583).
    if let Some(ref fixup) = args.fixup {
        config.postprocess.fixup = parse_arg::<FixupPolicy>(fixup, "fixup policy")?;
    }

    if let Some(ref proxy) = args.proxy {
        // Hard-fail at config-build time when --proxy is set explicitly.
        // The HttpClientFactory previously logged a warn-level message and
        // silently dropped the proxy on validation failure — a privacy
        // regression for users who pass `--proxy` expecting traffic
        // routing.
        rdlp_security::validate_proxy_url(proxy)
            .map_err(|e| anyhow::anyhow!("--proxy validation failed: {e}"))?;
        config.proxy = Some(proxy.clone());
    }
    // Browser emulation: CLI flag > env var > default (ChromeLatest).
    if let Some(ref cli_browser) = args.browser {
        config.browser_emulation = parse_browser_emulation(cli_browser);
    } else if let Ok(env_browser) = std::env::var("RDLP_BROWSER_EMULATION")
        && !env_browser.is_empty()
    {
        config.browser_emulation = parse_browser_emulation(&env_browser);
    }
    if let Some(ref rate_str) = args.limit_rate {
        let bps = rdlp_ratelimit::parse_rate_limit(rate_str)
            .with_context(|| format!("invalid rate limit '{rate_str}'"))?;
        config.rate_limit = Some(bps);
    }
    if let Some(ref browser) = args.cookies_from_browser {
        config.cookies_from_browser = Some(parse_arg::<BrowserType>(browser, "browser type")?);
    }
    // Remux: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.remux_container {
        config.postprocess.remux_container = Some(fmt);
    } else if let Some(container) = args.remux.as_deref()
        && container != "interactive"
    {
        config.postprocess.remux_container =
            Some(parse_arg::<ContainerFormat>(container, "remux container")?);
    }

    // Match filters (CLI appends to config file values)
    if !args.match_filter.is_empty() {
        config
            .match_filters
            .extend(args.match_filter.iter().cloned());
    }

    // Validate match filter syntax early
    for filter_str in &config.match_filters {
        rdlp_api::MatchFilter::parse(filter_str)
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("invalid match filter '{filter_str}'"))?;
    }

    if config.postprocess.extract_audio && config.postprocess.audio_format.is_none() {
        config.postprocess.audio_format = Some(AudioFormat::Mp3);
    }

    // Validate recode speed controls against the resolved encoder
    rdlp_ffmpeg::validate_speed_controls(
        rdlp_ffmpeg::resolve_recode_encoder(
            config.postprocess.video_encoder.as_deref(),
            config.postprocess.recode_video.map(|c| c.as_ext()),
            config.postprocess.recode_container.map(|c| c.as_ext()),
        ),
        config.postprocess.recode_preset.as_deref(),
        config
            .postprocess
            .recode_deadline
            .map(rdlp_types::VpxDeadline::as_str),
        config.postprocess.recode_cpu_used,
        config.postprocess.recode_speed_level,
    )
    .map_err(|e| anyhow::anyhow!("invalid recode speed control: {e}"))?;

    // Validate final config
    config
        .validate()
        .map_err(|e| anyhow::anyhow!("Invalid configuration: {e}"))?;

    Ok(config)
}

/// Build Config by: resolve interactive prompts -> load file -> merge.
pub fn build_config(args: &Args) -> Result<Config> {
    // Step 1: Resolve interactive values (side effects isolated here)
    let interactive_values = resolve_interactive_values(args)
        .context("failed to resolve interactive configuration values")?;

    // Step 2: Load config file (or use defaults)
    let file_config = if args.ignore_config {
        Config::default()
    } else {
        match config_io::load_config(args.config_location.as_deref()) {
            Ok(Some((file_config, path))) => {
                eprintln!("Loaded config from {}", path.display());
                file_config
            }
            Ok(None) => Config::default(),
            Err(e) => {
                if args.config_location.is_some() {
                    return Err(e.into());
                }
                eprintln!("Warning: Failed to load config file: {e}");
                Config::default()
            }
        }
    };

    // Step 3: Pure merge (no side effects, testable)
    merge_config(args, file_config, interactive_values)
}

#[cfg(test)]
#[path = "config_tests.rs"]
pub mod tests;
