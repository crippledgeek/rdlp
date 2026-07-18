//! `transcode_image` first-frame-only guarantee (#522).
//!
//! A thumbnail is a single still cover image, but the input may be a multi-frame
//! animation (GIF/animated WEBP) or a multi-page TIFF — both attacker-controlled
//! and, since #521, reachable via thumbnail discovery. `transcode_image` must
//! normalize only the FIRST frame regardless of input frame count.
//!
//! This also pins a correctness fact, not just a resource cap: the `.jpg` output
//! resolves to FFmpeg's `image2` muxer, which rejects a second frame written to
//! the same (non-pattern) filename with `EINVAL`. So the pre-#522 code, which
//! fed every decoded packet to the muxer, returned `Err` for ANY multi-frame
//! input — an animated-GIF thumbnail failed to normalize and was silently
//! skipped. First-frame-only makes it succeed AND bounds the work.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! multi-frame fixture).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;

use rdlp_ffmpeg::FFmpegRunner;

/// Both `ffmpeg` (fixture build + pixel decode) and `ffprobe` (frame count) are
/// used by this test, so the skip guard must confirm both — else a host with
/// only one would fail the test instead of self-skipping.
fn ffmpeg_cli_available() -> bool {
    ["ffmpeg", "ffprobe"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

/// Build a 2-frame animated GIF whose frames are distinguishable solid colors:
/// frame 0 = red, frame 1 = blue. The distinct colors let the test prove the
/// output is the FIRST frame (red), not the last (blue) — guarding against a
/// future "keep the last frame" regression, not merely "keep one frame".
fn build_two_frame_gif(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let f0 = dir.join("f0.png");
    let f1 = dir.join("f1.png");
    let gif = dir.join("anim.gif");

    let solid = |color: &str, out: &Path| -> bool {
        Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:s=64x64"),
                "-frames:v",
                "1",
                out.to_str().unwrap(),
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    if !solid("red", &f0) || !solid("blue", &f1) {
        return Err(());
    }

    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-framerate",
            "2",
            "-start_number",
            "0",
            "-i",
            dir.join("f%d.png").to_str().unwrap(),
            "-frames:v",
            "2",
            gif.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(gif) } else { Err(()) }
}

/// Count decodable frames in a media file via `ffprobe`.
fn frame_count(path: &Path) -> Option<u32> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "csv=p=0",
            path.to_str().unwrap(),
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Decode `path` to a single RGB24 pixel via `ffmpeg` and return `(r, g, b)`.
fn dominant_rgb(path: &Path) -> Option<(u8, u8, u8)> {
    let out = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            path.to_str().unwrap(),
            "-vf",
            "scale=1:1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .ok()?;
    match out.stdout.as_slice() {
        [r, g, b, ..] => Some((*r, *g, *b)),
        _ => None,
    }
}

/// A 2-frame animated GIF normalizes to a single-frame JPEG holding the FIRST
/// frame.
///
/// Fails against the pre-#522 code: feeding the second frame to the `image2`
/// muxer errors with `EINVAL`, so `transcode_image` returned `Err` and this
/// `expect` would panic. After the fix it returns `Ok`, the output has exactly
/// one frame, and that frame is red (frame 0), not blue (frame 1).
#[tokio::test]
async fn multi_frame_gif_normalizes_to_first_frame_only() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg/ffprobe CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let Ok(gif) = build_two_frame_gif(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    // Sanity: the fixture really is multi-frame, else the test proves nothing.
    assert_eq!(
        frame_count(&gif),
        Some(2),
        "fixture must be a 2-frame GIF for this test to be meaningful"
    );

    let out = dir.path().join("thumb.jpg");
    let ffmpeg = FFmpegRunner::new().expect("FFmpeg required");

    ffmpeg
        .transcode_image(&gif, &out)
        .await
        .expect("transcode_image must succeed on a multi-frame input (first-frame-only)");

    assert_eq!(
        frame_count(&out),
        Some(1),
        "normalized thumbnail must be a single frame regardless of input frame count"
    );

    let (r, g, b) = dominant_rgb(&out).expect("output jpg must decode to a pixel");
    assert!(
        r > 128 && r > g.saturating_add(64) && r > b.saturating_add(64),
        "output must be the FIRST frame (red), not the last (blue); got rgb=({r},{g},{b})"
    );
}
