//! Small test to verify ffmpeg-the-third bindings work for media probing.
//!
//! Generates a synthetic test video with ffmpeg CLI, then probes it
//! using the ffmpeg Rust bindings to extract stream info.

use std::path::Path;
use std::process::Command;

/// Generate a 1-second test video using ffmpeg CLI (lavfi source).
fn generate_test_video(path: &Path) {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=1:size=320x240:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(path)
        .output()
        .expect("ffmpeg CLI must be available");

    assert!(
        status.status.success(),
        "ffmpeg failed to generate test video"
    );
}

#[test]
fn test_ffmpeg_probe() {
    ffmpeg_the_third::init().expect("ffmpeg init failed");

    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let video_path = dir.path().join("test.mp4");

    generate_test_video(&video_path);

    let ctx = ffmpeg_the_third::format::input(&video_path).expect("failed to open test video");

    // Check duration (~1 second)
    let duration_secs = ctx.duration() as f64 / f64::from(ffmpeg_the_third::ffi::AV_TIME_BASE);
    assert!(
        duration_secs > 0.5 && duration_secs < 2.0,
        "unexpected duration: {duration_secs}"
    );

    // Should have at least 2 streams (video + audio)
    let stream_count = ctx.streams().count();
    assert!(
        stream_count >= 2,
        "expected >= 2 streams, got {stream_count}"
    );

    // Find video stream
    let video_stream = ctx
        .streams()
        .best(ffmpeg_the_third::media::Type::Video)
        .expect("no video stream found");

    let video_codec =
        ffmpeg_the_third::codec::context::Context::from_parameters(video_stream.parameters())
            .expect("failed to get video codec context");

    assert_eq!(video_codec.medium(), ffmpeg_the_third::media::Type::Video);

    if let Ok(video) = video_codec.decoder().video() {
        assert_eq!(video.width(), 320);
        assert_eq!(video.height(), 240);
        println!(
            "Video: {}x{}, format: {:?}",
            video.width(),
            video.height(),
            video.format()
        );
    }

    // Find audio stream
    let audio_stream = ctx
        .streams()
        .best(ffmpeg_the_third::media::Type::Audio)
        .expect("no audio stream found");

    let audio_codec =
        ffmpeg_the_third::codec::context::Context::from_parameters(audio_stream.parameters())
            .expect("failed to get audio codec context");

    assert_eq!(audio_codec.medium(), ffmpeg_the_third::media::Type::Audio);

    if let Ok(audio) = audio_codec.decoder().audio() {
        assert!(audio.rate() > 0, "audio sample rate should be > 0");
        println!(
            "Audio: {} Hz, channel_layout: {:?}, format: {:?}",
            audio.rate(),
            audio.ch_layout(),
            audio.format()
        );
    }

    println!("ffmpeg probe test passed");
}
