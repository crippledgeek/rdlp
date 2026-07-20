//! Interactive format and container selection menus.
//!
//! Provides inquire-based selection prompts for remux containers,
//! audio formats, and video recode formats.

use anyhow::{Context, Result};
use rdlp_api::{AudioFormat, ContainerFormat};

/// Interactive remux container selection
pub fn select_remux_container() -> Result<Option<ContainerFormat>> {
    let containers = [
        // Video containers
        (
            ContainerFormat::Mp4,
            "Best compatibility, faststart for streaming",
        ),
        (
            ContainerFormat::Mkv,
            "Supports all codecs, efficient cues index",
        ),
        (
            ContainerFormat::WebM,
            "Web-optimized, VP8/VP9/AV1 + Opus/Vorbis",
        ),
        (ContainerFormat::Mov, "Apple QuickTime, good for editing"),
        (ContainerFormat::Avi, "Legacy format, wide support"),
        (ContainerFormat::Ts, "MPEG-TS, broadcast/streaming"),
        (ContainerFormat::Flv, "Flash Video, legacy"),
        (ContainerFormat::ThreeGp, "3GPP mobile video"),
        (ContainerFormat::Mpg, "MPEG-1/2 program stream"),
        (ContainerFormat::F4v, "Flash Video (MP4 variant)"),
        (ContainerFormat::Wmv, "Windows Media Video"),
        (
            ContainerFormat::Asf,
            "Advanced Systems Format (third-party codecs)",
        ),
        (
            ContainerFormat::Mxf,
            "Material eXchange, broadcast/professional",
        ),
        (ContainerFormat::Vob, "DVD Video Object"),
        (ContainerFormat::Dv, "Digital Video"),
        (ContainerFormat::Nut, "NUT (FFmpeg native container)"),
        (ContainerFormat::Ivf, "On2 IVF (VP8/VP9/AV1 raw)"),
        // Audio containers
        (ContainerFormat::Mp3, "Audio only, MPEG Layer 3"),
        (ContainerFormat::Flac, "Audio only, lossless"),
        (ContainerFormat::Wav, "Audio only, PCM waveform"),
        (ContainerFormat::Ogg, "Audio only, Ogg container"),
        (ContainerFormat::M4a, "Audio only, MPEG-4 Audio"),
        (ContainerFormat::Opus, "Audio only, Ogg Opus"),
        (ContainerFormat::Aac, "Audio only, raw ADTS AAC"),
        (ContainerFormat::Aiff, "Audio only, Apple AIFF"),
        (ContainerFormat::Mka, "Audio only, Matroska Audio"),
        (ContainerFormat::Wv, "Audio only, WavPack lossless"),
        (ContainerFormat::Caf, "Audio only, Core Audio Format"),
        (ContainerFormat::Ac3, "Audio only, Dolby AC-3"),
        (ContainerFormat::Wma, "Audio only, Windows Media Audio"),
    ];

    let items: Vec<String> = containers
        .iter()
        .map(|(fmt, desc)| format!("{:<6} {desc}", fmt.as_ext()))
        .collect();

    let selection = inquire::Select::new("Select remux container:", items)
        .raw_prompt_skippable()
        .context("remux container selection prompt failed")?;

    #[allow(clippy::indexing_slicing)] // opt.index is guaranteed in-bounds by inquire
    Ok(selection.map(|opt| containers[opt.index].0))
}

/// Interactive audio format selection
pub fn select_audio_format() -> Result<Option<AudioFormat>> {
    let formats = [
        (AudioFormat::Mp3, "MPEG Layer 3, most compatible"),
        (AudioFormat::Aac, "Advanced Audio Coding"),
        (AudioFormat::M4a, "AAC in M4A container"),
        (AudioFormat::Opus, "Opus codec, excellent quality/size"),
        (AudioFormat::Vorbis, "Ogg Vorbis"),
        (AudioFormat::Flac, "Free Lossless Audio Codec"),
        (AudioFormat::Alac, "Apple Lossless"),
        (AudioFormat::Wav, "PCM waveform, uncompressed"),
        (AudioFormat::Ac3, "Dolby Digital"),
        (AudioFormat::Eac3, "Dolby Digital Plus"),
        (AudioFormat::Dts, "DTS Coherent Acoustics"),
        (AudioFormat::Mp2, "MPEG Layer 2"),
        (AudioFormat::WavPack, "WavPack lossless"),
        (AudioFormat::Tta, "True Audio lossless"),
    ];

    let items: Vec<String> = formats
        .iter()
        .map(|(fmt, desc)| format!("{:<8} {desc}", fmt.as_ext()))
        .collect();

    let selection = inquire::Select::new("Select audio format:", items)
        .raw_prompt_skippable()
        .context("audio format selection prompt failed")?;

    #[allow(clippy::indexing_slicing)] // opt.index is guaranteed in-bounds by inquire
    Ok(selection.map(|opt| formats[opt.index].0))
}

/// Interactive video recode format selection
pub fn select_recode_video() -> Result<Option<ContainerFormat>> {
    let formats = [
        (ContainerFormat::Mp4, "h264", "Best compatibility, H.264"),
        (ContainerFormat::Mkv, "h264", "Matroska, H.264"),
        (ContainerFormat::WebM, "vp9", "Web-optimized, VP9"),
        (ContainerFormat::Mov, "h264", "Apple QuickTime, H.264"),
        (ContainerFormat::Avi, "h264", "Legacy AVI, H.264"),
        (ContainerFormat::Mpg, "mpeg2", "MPEG program stream, MPEG-2"),
        (ContainerFormat::Ts, "h264", "MPEG-TS, H.264"),
        (ContainerFormat::ThreeGp, "h264", "3GPP mobile, H.264"),
        (ContainerFormat::Flv, "h264", "Flash Video, H.264"),
        (ContainerFormat::Wmv, "wmv2", "Windows Media Video, WMV2"),
    ];

    let items: Vec<String> = formats
        .iter()
        .map(|(fmt, codec, desc)| format!("{:<6} [{codec}] {desc}", fmt.as_ext()))
        .collect();

    let selection = inquire::Select::new("Select video format:", items)
        .raw_prompt_skippable()
        .context("video recode format selection prompt failed")?;

    #[allow(clippy::indexing_slicing)] // opt.index is guaranteed in-bounds by inquire
    Ok(selection.map(|opt| formats[opt.index].0))
}
