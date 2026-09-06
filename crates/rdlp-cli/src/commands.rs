//! Utility commands and helpers for CLI output.
//!
//! Contains codec listing, field printing, exit code mapping,
//! and the error exit helper.

use anyhow::{Context, Result};
use log::{debug, error, warn};
use rdlp_api::InfoDict;
use rdlp_api::RdlpApiError;

/// Print all supported codecs
pub fn print_codecs() {
    println!("Audio codecs (14):");
    let audio_codecs = [
        ("mp3", "libmp3lame", "MPEG Layer 3"),
        ("aac", "aac", "Advanced Audio Coding"),
        ("m4a", "aac", "AAC in M4A container"),
        ("opus", "libopus", "Opus codec"),
        ("vorbis", "libvorbis", "Ogg Vorbis"),
        ("flac", "flac", "Free Lossless Audio Codec"),
        ("alac", "alac", "Apple Lossless"),
        ("wav", "pcm_s16le", "PCM waveform"),
        ("ac3", "ac3", "Dolby Digital"),
        ("eac3", "eac3", "Dolby Digital Plus"),
        ("dts", "dca", "DTS Coherent Acoustics"),
        ("mp2", "mp2", "MPEG Layer 2"),
        ("wavpack", "wavpack", "WavPack lossless"),
        ("tta", "tta", "True Audio lossless"),
    ];
    for (name, encoder, desc) in audio_codecs {
        println!("  {name:<10} [{encoder}]  {desc}");
    }

    println!();
    println!("Video codecs (16):");
    let video_codecs = [
        ("h264", "libx264", "H.264 / AVC"),
        ("h265", "libx265", "H.265 / HEVC"),
        ("vp9", "libvpx-vp9", "VP9"),
        ("vp8", "libvpx", "VP8"),
        ("av1", "libaom-av1", "AV1"),
        ("vvc", "libvvenc", "H.266 / VVC"),
        ("mpeg1", "mpeg1video", "MPEG-1 Video"),
        ("mpeg2", "mpeg2video", "MPEG-2 Video"),
        ("mpeg4", "mpeg4", "MPEG-4 Part 2"),
        ("theora", "libtheora", "Theora"),
        ("prores", "prores_ks", "Apple ProRes"),
        ("dnxhd", "dnxhd", "Avid DNxHD"),
        ("wmv2", "wmv2", "Windows Media Video 8"),
        ("ffv1", "ffv1", "FFV1 lossless archival"),
        ("xvid", "libxvid", "Xvid (MPEG-4 ASP)"),
    ];
    for (name, encoder, desc) in video_codecs {
        println!("  {name:<10} [{encoder}]  {desc}");
    }
}

/// Print specific fields from an `InfoDict`
pub fn print_fields(info: &InfoDict, fields: &str) -> Result<()> {
    let value = serde_json::to_value(info).context("failed to serialize InfoDict to JSON value")?;
    let map = value
        .as_object()
        .context("InfoDict did not serialize to a JSON object")?;

    for field in fields.split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        match map.get(field) {
            Some(serde_json::Value::String(s)) => {
                println!("{field}: {}", rdlp_cli::sanitize::sanitize_for_terminal(s));
            }
            Some(serde_json::Value::Null) => println!("{field}:"),
            Some(v) => println!("{field}: {v}"),
            None => {
                warn!("Unknown field: {field}");
                eprintln!("Warning: unknown field '{field}'");
            }
        }
    }
    Ok(())
}

/// Map `RdlpApiError` to a structured process exit code.
///
/// Exit codes:
///   0 -- success (handled by Ok paths)
///   1 -- general/unknown error (I/O, processing, platform)
///   2 -- user cancelled (Ctrl+C, ESC)
///   3 -- extraction failed (unsupported URL, extraction error)
///   4 -- download/network failed (network error)
///   5 -- configuration/format error (invalid input, builder error)
pub const fn exit_code_for(e: &RdlpApiError) -> i32 {
    match e {
        RdlpApiError::UserCancelled => 2,
        RdlpApiError::UnsupportedUrl { .. } | RdlpApiError::ExtractError { .. } => 3,
        RdlpApiError::NetworkError { .. } => 4,
        RdlpApiError::InvalidInput { .. } | RdlpApiError::BuilderError { .. } => 5,
        RdlpApiError::IoError { .. }
        | RdlpApiError::FfmpegError { .. }
        | RdlpApiError::UnsupportedPlatform { .. }
        | RdlpApiError::Soft { .. } => 1,
    }
}

/// Emit the CLI's terminal record for a failed action.
///
/// Split from [`fail_with`] so it is testable — `fail_with` is `-> !` and
/// exits the process. `e`'s Display is redacted per variant, so the reason
/// carries no credential.
///
/// `verbose`'s DEBUG line is a second record of the SAME causal event the
/// ERROR line above it already carries — the convention forbids two records
/// at ERROR for one outcome, so it is demoted to DEBUG rather than dropped.
pub fn record_failure(action: &str, e: &RdlpApiError, verbose: bool) {
    error!("action={action} outcome=failed reason={e}");
    if verbose {
        debug!("action={action} outcome=failed detail={e:?}");
    }
}

/// Log an `RdlpApiError` and exit with the appropriate structured code.
pub fn fail_with(action: &str, e: &RdlpApiError, verbose: bool) -> ! {
    record_failure(action, e, verbose);
    std::process::exit(exit_code_for(e))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    /// The CLI's terminal record carries the same vocabulary as the desktop's.
    #[test]
    fn the_cli_terminal_record_names_action_and_outcome() {
        testing_logger::setup();
        let err = rdlp_api::RdlpApiError::InvalidInput {
            message: "bad url".to_owned(),
        };
        super::record_failure("analyze", &err, false);

        testing_logger::validate(|captured| {
            let errs: Vec<_> = captured
                .iter()
                .filter(|l| l.level == log::Level::Error)
                .collect();
            assert_eq!(errs.len(), 1, "exactly one terminal record");
            let body = errs.first().map_or("", |l| l.body.as_str());
            assert!(body.contains("action=analyze"), "got: {body}");
            assert!(body.contains("outcome=failed"), "got: {body}");
            assert!(body.contains("bad url"), "got: {body}");
        });
    }

    /// The verbose detail line is a SECOND record of the same causal event,
    /// so it must never land at ERROR alongside the terminal record — that
    /// would be two ERROR-level records for one outcome, which is the
    /// duplicate the ERROR-to-DEBUG demotion exists to prevent.
    #[test]
    fn verbose_detail_is_recorded_at_debug_not_a_second_error() {
        testing_logger::setup();
        let err = rdlp_api::RdlpApiError::InvalidInput {
            message: "bad url".to_owned(),
        };
        super::record_failure("analyze", &err, true);

        testing_logger::validate(|captured| {
            let errs = captured
                .iter()
                .filter(|l| l.level == log::Level::Error)
                .count();
            let debugs = captured
                .iter()
                .filter(|l| l.level == log::Level::Debug)
                .count();
            assert_eq!(errs, 1, "exactly one terminal record at ERROR");
            assert_eq!(debugs, 1, "exactly one detail record at DEBUG");
        });
    }
}
