//! Video codec availability command.

use rdlp_ffmpeg::{VideoCodecInfo, ffmpeg::video_codecs::list_available_codecs};

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
