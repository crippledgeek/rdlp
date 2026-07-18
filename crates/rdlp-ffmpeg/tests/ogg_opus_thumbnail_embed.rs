//! Regression test for #531: thumbnail embed into Ogg/Opus.
//!
//! `ogg`/`opus` are listed in `SUPPORTED_CONTAINERS`
//! (`rdlp-postprocess/src/pipeline/stages/thumbnail.rs`), but embedding via
//! `FFmpegRunner::embed_thumbnail` failed with `Unsupported codec id in
//! stream 1` — `FFmpeg`'s Ogg muxer (`oggenc.c` `ogg_init()`) hard-rejects
//! any stream whose codec isn't Vorbis/Theora/Speex/FLAC/Opus/VP8, so the
//! cover art can never ride an `ATTACHED_PIC` video stream in these
//! containers (unlike FLAC, which does accept one).
//!
//! The fix carries the cover as a base64 FLAC `PICTURE` block in a
//! `METADATA_BLOCK_PICTURE` `VorbisComment` field instead of a stream; on
//! read, `FFmpeg` re-exposes it as an `attached_pic` video stream.
//!
//! Self-skips when the `ffmpeg` CLI is absent (used only to build fixtures),
//! mirroring `container_accepts_image_codec.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::FFmpegRunner;

fn ffmpeg_cli_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Render a 1-frame solid-color still cover image.
fn make_cover(dir: &Path, format: &str) -> PathBuf {
    let path = dir.join(format!("cover.{format}"));
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=red:s=32x32:d=1"])
        .args(["-frames:v", "1"])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "failed to build {format} cover fixture");
    path
}

/// Build a 1-second audio-only fixture in `container` using `encoder`.
fn make_audio(dir: &Path, container: &str, encoder: &str) -> PathBuf {
    let path = dir.join(format!("audio.{container}"));
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-c:a", encoder])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(
        status.success(),
        "failed to build {container} audio fixture"
    );
    path
}

/// Demux `media`'s attached-picture video stream to `dst` via the `ffmpeg` CLI.
fn extract_cover(media: &Path, dst: &Path) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-i"])
        .arg(media)
        .args(["-an", "-map", "0:v", "-c", "copy"])
        .arg(dst)
        .status()
        .expect("spawn ffmpeg");
    assert!(
        status.success(),
        "failed to extract cover from {}",
        media.display()
    );
}

/// Load-bearing regression: embedding a thumbnail into Opus must succeed and
/// must NOT reproduce the old "Unsupported codec id" mux failure.
#[tokio::test]
async fn embed_thumbnail_into_opus_round_trips_byte_identical_cover() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let cover = make_cover(dir.path(), "png");
    let audio = make_audio(dir.path(), "opus", "libopus");
    let output = dir.path().join("out.opus");

    let runner = FFmpegRunner::new().expect("FFmpeg");
    let result = runner
        .embed_thumbnail(&audio, &cover, &output, "opus", None, None)
        .await;

    if let Err(e) = &result {
        let msg = format!("{e:#}");
        assert!(
            !msg.contains("Unsupported codec id"),
            "the #531 mux failure recurred: {msg}"
        );
    }
    result.expect("thumbnail embed into opus must succeed");

    let extracted = dir.path().join("extracted.png");
    extract_cover(&output, &extracted);

    let original = std::fs::read(&cover).unwrap();
    let round_tripped = std::fs::read(&extracted).unwrap();
    assert_eq!(
        original, round_tripped,
        "extracted cover must be byte-identical to the source thumbnail"
    );
}

/// Same regression for `.ogg` (Vorbis audio) with a JPEG cover, pinning the
/// other codec/container combination named in #531.
#[tokio::test]
async fn embed_thumbnail_into_ogg_round_trips_byte_identical_cover() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let cover = make_cover(dir.path(), "jpg");
    let audio = make_audio(dir.path(), "ogg", "libvorbis");
    let output = dir.path().join("out.ogg");

    let runner = FFmpegRunner::new().expect("FFmpeg");
    runner
        .embed_thumbnail(&audio, &cover, &output, "ogg", None, None)
        .await
        .expect("thumbnail embed into ogg must succeed");

    let extracted = dir.path().join("extracted.jpg");
    extract_cover(&output, &extracted);

    let original = std::fs::read(&cover).unwrap();
    let round_tripped = std::fs::read(&extracted).unwrap();
    assert_eq!(
        original, round_tripped,
        "extracted cover must be byte-identical to the source thumbnail"
    );
}

/// Non-regression: FLAC's `ATTACHED_PIC`-stream strategy is untouched by the
/// Ogg/Opus fix (`is_ogg_opus` is `false` for `flac`) and must keep working —
/// the shared thumbnail-stream/packet code path is a common seam between the
/// two branches.
#[tokio::test]
async fn embed_thumbnail_into_flac_still_round_trips_byte_identical_cover() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let cover = make_cover(dir.path(), "png");
    let audio = make_audio(dir.path(), "flac", "flac");
    let output = dir.path().join("out.flac");

    let runner = FFmpegRunner::new().expect("FFmpeg");
    runner
        .embed_thumbnail(&audio, &cover, &output, "flac", None, None)
        .await
        .expect("thumbnail embed into flac must still succeed");

    let extracted = dir.path().join("extracted.png");
    extract_cover(&output, &extracted);

    let original = std::fs::read(&cover).unwrap();
    let round_tripped = std::fs::read(&extracted).unwrap();
    assert_eq!(
        original, round_tripped,
        "extracted cover must be byte-identical to the source thumbnail"
    );
}
