//! Utility commands and helpers for CLI output.
//!
//! Contains codec listing, field printing, exit code mapping,
//! and the error exit helper.

use anyhow::Result;
use rdlp_api::RdlpApiError;
use rdlp_core::InfoDict;
use tracing::error;

/// Print all supported codecs
pub(crate) fn print_codecs() {
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

/// Print specific fields from an InfoDict
pub(crate) fn print_fields(info: &InfoDict, fields: &str) -> Result<()> {
    let value = serde_json::to_value(info)?;
    let map = value.as_object().expect("InfoDict serializes to object");

    for field in fields.split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        match map.get(field) {
            Some(serde_json::Value::String(s)) => println!("{field}: {s}"),
            Some(serde_json::Value::Null) => println!("{field}:"),
            Some(v) => println!("{field}: {v}"),
            None => {
                tracing::warn!("Unknown field: {field}");
                eprintln!("Warning: unknown field '{field}'");
            }
        }
    }
    Ok(())
}

/// Map RdlpApiError to a structured process exit code.
///
/// Exit codes:
///   0 -- success (handled by Ok paths)
///   1 -- general/unknown error (I/O, processing, platform)
///   2 -- user cancelled (Ctrl+C, ESC)
///   3 -- extraction failed (unsupported URL, extraction error)
///   4 -- download/network failed (network error)
///   5 -- configuration/format error (invalid input, builder error)
pub(crate) fn exit_code_for(e: &RdlpApiError) -> i32 {
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

/// Log an RdlpApiError and exit with the appropriate structured code.
pub(crate) fn fail_with(e: RdlpApiError, verbose: bool) -> ! {
    error!("Error: {e}");
    if verbose {
        error!("Debug info: {e:?}");
    }
    std::process::exit(exit_code_for(&e))
}
