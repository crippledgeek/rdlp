//! Video and audio codec availability commands.

use rdlp_ffmpeg::{
    AudioCodecInfo, VideoCodecInfo, ffmpeg::audio_encoder_registry::list_available_audio_codecs,
    ffmpeg::video_codecs::list_available_codecs,
};
use rdlp_types::ContainerFormat;

/// List available video codecs and their encoders.
///
/// Returns only codecs with at least one available encoder in the
/// linked FFmpeg build.
#[tauri::command]
#[must_use]
pub fn get_available_codecs() -> Vec<VideoCodecInfo> {
    rdlp_ffmpeg::ffmpeg::ensure_init().ok();
    list_available_codecs()
}

/// List available audio codecs (optionally filtered by target container).
///
/// When `container` is supplied, only codecs compatible with that container
/// are returned. When `container` is `None`, all available audio codecs
/// are returned.
#[tauri::command]
#[must_use]
pub fn get_available_audio_codecs(container: Option<ContainerFormat>) -> Vec<AudioCodecInfo> {
    rdlp_ffmpeg::ffmpeg::ensure_init().ok();
    let all = list_available_audio_codecs();
    if let Some(c) = container {
        all.into_iter()
            .filter(|codec| {
                rdlp_ffmpeg::ffmpeg::audio_encoder_registry::container_supports_audio_codec(
                    c,
                    &codec.codec,
                )
            })
            .collect()
    } else {
        all
    }
}
