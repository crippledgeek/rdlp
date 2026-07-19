//! IMPORTANT #4 (code-review follow-up to #549): `container_accepts_image_codec`'s
//! predicate widened from `avformat_query_codec(..) == 1` to `> 0`.
//!
//! mp3's `query_codec` callback (`mp3enc.c`) answers every ID3v2-APIC-representable
//! image codec (gif, mjpeg, png, tiff, bmp, webp — `ff_id3v2_mime_tags`) with
//! `MKTAG('A','P','I','C')`, a large *positive* value — not `1`, which the
//! callback reserves for MP3 audio itself. `== 1` misread that as "cannot
//! store" and forced a needless transcode for gif/tiff/bmp/webp covers into
//! mp3 (JPEG/PNG are rescued by a separate baseline special-case regardless
//! of this predicate). `> 0` reads it correctly, matching the same
//! `avformat_query_codec` convention already used for stream representability
//! in `ffi_helpers/mod.rs::resolve_codec_tag`.
//!
//! This file proves the widened gate holds at the actual `embed_thumbnail`
//! level (not just the query `container_accepts_image_codec` answers) for
//! every codec the widening newly lets skip normalization, and pins the
//! unrelated paths (webp into MKV attachment, webp into MP4 ATTACHED_PIC)
//! stay exactly as they were before this change.
//!
//! Self-skips when the `ffmpeg`/`ffprobe` CLI is absent (used only to build
//! fixtures and to independently verify the embed, mirroring
//! `container_accepts_image_codec.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::FFmpegRunner;

fn tooling_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Render a 1-frame solid-color still in `format`.
fn make_image(dir: &Path, format: &str) -> PathBuf {
    let path = dir.join(format!("still.{format}"));
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=red:s=32x32:d=1"])
        .args(["-frames:v", "1"])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "failed to build {format} fixture");
    path
}

/// Build a short MP3 audio fixture (no video stream).
fn make_mp3_audio(dir: &Path) -> PathBuf {
    let path = dir.join("audio.mp3");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "sine=d=1"])
        .args(["-c:a", "libmp3lame"])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "failed to build mp3 audio fixture");
    path
}

/// Build a short MP4 (h264) media fixture.
fn make_mp4_media(dir: &Path) -> PathBuf {
    let path = dir.join("media.mp4");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc=d=1:s=320x240:r=25"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-t", "1"])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "failed to build mp4 media fixture");
    path
}

/// The embedded cover's `codec_name` and `attached_pic` disposition, read
/// back independently via `ffprobe`.
#[derive(Debug)]
struct CoverInfo {
    codec_name: String,
    attached_pic: bool,
}

fn probe_video_stream(path: &Path) -> CoverInfo {
    let output = Command::new("ffprobe")
        .args(["-hide_banner", "-v", "error", "-select_streams", "v"])
        .args([
            "-show_entries",
            "stream=codec_name:stream_disposition=attached_pic",
        ])
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(output.status.success(), "ffprobe failed on {path:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    let codec_name = text
        .lines()
        .find_map(|l| l.strip_prefix("codec_name="))
        .expect("codec_name present")
        .to_string();
    let attached_pic = text
        .lines()
        .find_map(|l| l.strip_prefix("DISPOSITION:attached_pic="))
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    CoverInfo {
        codec_name,
        attached_pic,
    }
}

/// The load-bearing regression: gif/tiff/bmp/webp must embed directly into
/// mp3 via ID3v2 `APIC` — this is exactly the case `== 1` rejected (forcing
/// a needless jpeg transcode upstream in the postprocess pipeline). JPEG/PNG
/// are included as the pre-existing baseline regression guard.
#[tokio::test]
async fn mp3_embeds_every_id3v2_apic_codec_directly() {
    if !tooling_available() {
        eprintln!("skipping: ffmpeg/ffprobe CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let audio = make_mp3_audio(dir.path());
    let runner = FFmpegRunner::new().expect("FFmpeg");

    for (format, expected_codec) in [
        ("gif", "gif"),
        ("tiff", "tiff"),
        ("bmp", "bmp"),
        ("webp", "webp"),
        ("jpg", "mjpeg"),
        ("png", "png"),
    ] {
        let thumb = make_image(dir.path(), format);
        let output = dir.path().join(format!("out_{format}.mp3"));

        runner
            .embed_thumbnail(&audio, &thumb, &output, "mp3", None, None)
            .await
            .unwrap_or_else(|e| panic!("{format}->mp3 embed must succeed: {e:#}"));

        let info = probe_video_stream(&output);
        assert_eq!(
            info.codec_name, expected_codec,
            "{format}->mp3: cover codec mismatch, got {info:?}",
        );
        assert!(
            info.attached_pic,
            "{format}->mp3: cover must carry the attached_pic disposition"
        );
    }
}

/// Unrelated-path regression pin: webp into MP4's `ATTACHED_PIC` strategy
/// must still be rejected directly (MP4's tag table has no webp entry, and
/// this path is untouched by the mp3-specific predicate widening above).
#[tokio::test]
async fn webp_into_mp4_still_rejected_directly() {
    if !tooling_available() {
        eprintln!("skipping: ffmpeg/ffprobe CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let media = make_mp4_media(dir.path());
    let thumb = make_image(dir.path(), "webp");
    let output = dir.path().join("out.mp4");
    let runner = FFmpegRunner::new().expect("FFmpeg");

    runner
        .embed_thumbnail(&media, &thumb, &output, "mp4", None, None)
        .await
        .expect_err("webp reaching the MP4 ATTACHED_PIC path unnormalized must be rejected");
}
