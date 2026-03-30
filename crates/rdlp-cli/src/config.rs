//! Configuration building and merging for CLI.
//!
//! Resolves interactive prompts, loads config files, and merges
//! CLI arguments with file-based and default configuration using
//! a three-layer precedence: CLI > config file > defaults.

use anyhow::{Context, Result};
use rdlp_api::{
    AudioFormat, BrowserType, Config, ContainerFormat, FixupPolicy, RecodeAudioMode, SubtitleFormat,
    config_io,
};

use crate::args::Args;
use crate::selection::{select_audio_format, select_recode_video, select_remux_container};

/// Resolved values from interactive CLI prompts.
///
/// These are resolved BEFORE the pure config merge so that `merge_config()`
/// remains free of side effects and is testable.
pub(crate) struct ResolvedInteractiveValues {
    pub audio_format: Option<AudioFormat>,
    pub recode_video: Option<ContainerFormat>,
    pub remux_container: Option<ContainerFormat>,
}

/// Resolve interactive CLI values (inquire prompts) before config merge.
///
/// Returns `None` values for fields that weren't set to "interactive".
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

/// Merge a boolean CLI flag into config (sets to true if flag is set).
macro_rules! merge_bool {
    ($config:expr, $args:expr, $field:ident) => {
        if $args.$field {
            $config.$field = true;
        }
    };
    ($config:expr, $args:expr, postprocess.$field:ident) => {
        if $args.$field {
            $config.postprocess.$field = true;
        }
    };
}

/// Pure config merge: defaults < config file < CLI args.
///
/// This function has no side effects (no interactive prompts, no I/O).
/// Interactive values must be pre-resolved via `resolve_interactive_values()`.
pub(crate) fn merge_config(
    args: &Args,
    file_config: Config,
    interactive_values: ResolvedInteractiveValues,
) -> Result<Config> {
    let mut config = file_config;

    // Output: -o - means stdout streaming
    if args.output.as_deref() == Some("-") {
        config.output_to_stdout = true;
        // Force quiet mode — progress/log output would corrupt the byte stream.
        config.quiet = true;
        // Explicitly suppress progress as well: config.progress is derived from
        // config.quiet further below, but setting it here guards against future
        // refactors that might add an early return between here and the derivation.
        config.progress = false;
        // Disable embed_thumbnail before validation: the default is `true`, and
        // Config::validate() would reject stdout + embed-thumbnail.
        if config.postprocess.embed_thumbnail {
            // Use eprintln, not warn!(): the tracing subscriber is not
            // initialised yet (it happens after build_config returns).
            eprintln!("Warning: disabling --embed-thumbnail (incompatible with -o -)");
            config.postprocess.embed_thumbnail = false;
        }
    } else if let Some(ref output) = args.output {
        config.output_template = output.clone();
    }
    if let Some(ref dir) = args.output_dir {
        config.output_directory = dir.clone();
    }
    if let Some(ref format) = args.format {
        config.format = Some(format.clone());
    }
    merge_bool!(config, args, audio_multistreams);
    merge_bool!(config, args, quiet);
    merge_bool!(config, args, verbose);
    merge_bool!(config, args, simulate);
    merge_bool!(config, args, postprocess.extract_audio);

    // Audio format: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.audio_format {
        config.postprocess.audio_format = Some(fmt);
    } else if let Some(audio_format) = args.audio_format.as_deref()
        && audio_format != "interactive"
    {
        config.postprocess.audio_format = Some(
            audio_format
                .parse::<AudioFormat>()
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("invalid audio format '{audio_format}'"))?,
        );
    }

    if let Some(ref audio_quality) = args.audio_quality {
        config.postprocess.audio_quality = Some(audio_quality.clone());
    }
    merge_bool!(config, args, postprocess.embed_metadata);
    if args.no_thumbnail {
        config.postprocess.embed_thumbnail = false;
    }
    merge_bool!(config, args, postprocess.write_thumbnail);

    // Subtitles
    merge_bool!(config, args, postprocess.write_subtitles);
    merge_bool!(config, args, write_auto_subtitles);
    if let Some(ref langs) = args.sub_langs {
        config.subtitle_langs = langs.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Some(ref format) = args.sub_format {
        config.subtitle_format = Some(
            format
                .parse::<SubtitleFormat>()
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("invalid subtitle format '{format}'"))?,
        );
    }
    merge_bool!(config, args, postprocess.embed_subtitles);
    // --list-subs implies --write-subtitles
    if args.list_subs || args.list_subs_only {
        config.postprocess.write_subtitles = true;
        config.list_subs = true;
    }
    // --embed-subtitles implies --write-subtitles for the download step
    if config.postprocess.embed_subtitles && !config.postprocess.write_subtitles {
        config.postprocess.write_subtitles = true;
    }
    merge_bool!(config, args, strict_subs);
    merge_bool!(config, args, verify_sub_urls);
    merge_bool!(config, args, retry_subs);

    // Recode video: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.recode_video {
        config.postprocess.recode_video = Some(fmt);
    } else if let Some(recode_video) = args.recode_video.as_deref()
        && recode_video != "interactive"
    {
        config.postprocess.recode_video = Some(
            recode_video
                .parse::<ContainerFormat>()
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("invalid recode video container '{recode_video}'"))?,
        );
    }

    if let Some(ref encoder) = args.video_encoder {
        config.postprocess.video_encoder = Some(encoder.clone());
    }

    // recode_container: explicit container for recode (overrides recode_video container)
    if let Some(ref fmt) = args.recode_container {
        config.postprocess.recode_container = Some(
            fmt.parse::<ContainerFormat>()
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("invalid recode container '{fmt}'"))?,
        );
    }

    // recode_audio: "copy", "auto", or an encoder name
    match args.recode_audio.as_str() {
        "copy" => config.postprocess.recode_audio = RecodeAudioMode::Copy,
        "auto" => config.postprocess.recode_audio = RecodeAudioMode::Auto,
        name => {
            config.postprocess.recode_audio = RecodeAudioMode::Encoder {
                name: name.to_string(),
            };
        }
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
    if let Some(ref preset) = args.loudnorm_preset {
        config.postprocess.loudnorm_preset = Some(preset.clone());
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
    merge_bool!(config, args, postprocess.loudnorm_dynamic);
    merge_bool!(config, args, postprocess.loudnorm_precompress);

    // Fixup policy
    if args.fixup != "detect_or_warn" {
        config.postprocess.fixup = args
            .fixup
            .parse::<FixupPolicy>()
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("invalid fixup policy '{}'", args.fixup))?;
    }

    merge_bool!(config, args, postprocess.keep_video);
    if let Some(ref ffmpeg_location) = args.ffmpeg_location {
        config.postprocess.ffmpeg_location = Some(ffmpeg_location.clone());
    }
    if let Some(ref proxy) = args.proxy {
        config.proxy = Some(proxy.clone());
    }
    if let Some(ref rate_str) = args.limit_rate {
        let bps = rdlp_ratelimit::parse_rate_limit(rate_str)
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("invalid rate limit '{rate_str}'"))?;
        config.rate_limit = Some(bps);
    }
    if let Some(ref browser) = args.cookies_from_browser {
        config.cookies_from_browser = Some(
            browser
                .parse::<BrowserType>()
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("invalid browser type '{browser}'"))?,
        );
    }
    if let Some(ref cookies) = args.cookies {
        config.cookies_file = Some(cookies.clone());
    }
    if let Some(ref archive) = args.download_archive {
        config.download_archive = Some(archive.clone());
    }

    // Remux: interactive (pre-resolved) or direct parse
    if let Some(fmt) = interactive_values.remux_container {
        config.postprocess.remux_container = Some(fmt);
    } else if let Some(container) = args.remux.as_deref()
        && container != "interactive"
    {
        config.postprocess.remux_container = Some(
            container
                .parse::<ContainerFormat>()
                .map_err(|e| anyhow::anyhow!(e))
                .with_context(|| format!("invalid remux container '{container}'"))?,
        );
    }

    // Derived settings
    config.progress = !config.quiet;

    if config.postprocess.extract_audio && config.postprocess.audio_format.is_none() {
        config.postprocess.audio_format = Some(AudioFormat::Mp3);
    }

    // Validate final config
    config
        .validate()
        .map_err(|e| anyhow::anyhow!("Invalid configuration: {e}"))?;

    Ok(config)
}

/// Build Config by: resolve interactive prompts -> load file -> merge.
pub(crate) fn build_config(args: &Args) -> Result<Config> {
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
mod tests;
