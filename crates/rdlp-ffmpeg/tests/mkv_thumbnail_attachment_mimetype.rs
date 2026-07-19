//! #530: `embed_thumbnail_mkv_raw_ffi` must declare the Matroska attachment's
//! REAL mimetype, and must refuse to attach a format that FFmpeg's own
//! read-back promotion table cannot render as a visible cover.
//!
//! The previous catch-all (`_ => ("image/jpeg", "cover.jpg")`) mislabeled
//! gif/tiff/bmp thumbnails as `image/jpeg`. Separately, `image/webp` was
//! already declared honestly but is a *latent* bug: FFmpeg's Matroska
//! demuxer (`matroskadec.c`'s `mkv_image_mime_tags`) only promotes an
//! attachment to a real, player-visible `attached_pic` video stream for a
//! mimetype string it recognizes (`image/jpeg`, `image/png`, `image/gif`,
//! `image/tiff`) — `image/bmp`/`image/webp` stay a generic, non-rendered
//! attachment forever. Verified empirically (2026-07-19) against the linked
//! `FFmpeg` build via `ffprobe`.
//!
//! Self-skips when the `ffmpeg`/`ffprobe` CLI is absent (used only to build
//! fixtures and probe results, mirroring `container_accepts_image_codec.rs`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;

use rdlp_ffmpeg::FFmpegRunner;

fn ffmpeg_cli_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn ffprobe_cli_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn tooling_available() -> bool {
    ffmpeg_cli_available() && ffprobe_cli_available()
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

/// Build a tiny real Matroska file to embed the thumbnail into.
fn make_media(dir: &Path) -> PathBuf {
    let path = dir.join("media.mkv");
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=blue:s=32x32:d=1"])
        .args(["-c:v", "libx264", "-t", "1"])
        .arg(&path)
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "failed to build base media fixture");
    path
}

/// The last stream's `codec_type`, `attached_pic` disposition, and declared
/// `mimetype` tag, read back via `ffprobe`.
struct AttachmentStreamInfo {
    codec_type: String,
    attached_pic: bool,
    mimetype: Option<String>,
}

fn probe_last_stream(mkv: &Path) -> AttachmentStreamInfo {
    let output = Command::new("ffprobe")
        .args(["-hide_banner", "-v", "error"])
        .args([
            "-show_entries",
            "stream=index,codec_type:stream_disposition=attached_pic:stream_tags=mimetype",
        ])
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(mkv)
        .output()
        .expect("spawn ffprobe");
    assert!(output.status.success(), "ffprobe failed to read {mkv:?}");
    let text = String::from_utf8_lossy(&output.stdout);

    // Blocks start at "index=" — keep the LAST block (the attachment we added).
    let last_block = text
        .split("index=")
        .filter(|b| !b.is_empty())
        .last()
        .unwrap_or_default();

    let codec_type = last_block
        .lines()
        .find_map(|l| l.strip_prefix("codec_type="))
        .expect("codec_type present")
        .to_string();
    let attached_pic = last_block
        .lines()
        .find_map(|l| l.strip_prefix("DISPOSITION:attached_pic="))
        .map(|v| v.trim() == "1")
        .unwrap_or(false);
    let mimetype = last_block
        .lines()
        .find_map(|l| l.strip_prefix("TAG:mimetype="))
        .map(str::to_string);

    AttachmentStreamInfo {
        codec_type,
        attached_pic,
        mimetype,
    }
}

/// Positive + regression guard: GIF and TIFF must attach as a REAL,
/// player-visible cover (`Video` stream, `attached_pic=1`) declaring their
/// own honest mimetype — not the pre-fix catch-all's `image/jpeg`.
///
/// Fails against the unpatched catch-all, which declared `image/jpeg` for
/// both formats.
#[tokio::test]
async fn gif_and_tiff_attach_as_honest_visible_cover_art() {
    if !tooling_available() {
        eprintln!("skipping: ffmpeg/ffprobe CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let media = make_media(dir.path());
    let runner = FFmpegRunner::new().expect("FFmpeg");

    for (format, expected_mime) in [("gif", "image/gif"), ("tiff", "image/tiff")] {
        let thumb = make_image(dir.path(), format);
        let output = dir.path().join(format!("out_{format}.mkv"));

        runner
            .embed_thumbnail(&media, &thumb, &output, "mkv", None, None)
            .await
            .unwrap_or_else(|e| panic!("{format} embed must succeed: {e}"));

        let info = probe_last_stream(&output);
        assert_eq!(
            info.codec_type, "video",
            "{format} must promote to a video (attached_pic) stream on read-back"
        );
        assert!(
            info.attached_pic,
            "{format} attachment must set the attached_pic disposition"
        );
        assert_eq!(
            info.mimetype.as_deref(),
            Some(expected_mime),
            "{format} must declare its own mimetype, not a fallback"
        );
    }
}

/// Regression pin: JPEG/PNG must keep working exactly as before.
#[tokio::test]
async fn jpeg_and_png_still_attach_as_visible_cover_art() {
    if !tooling_available() {
        eprintln!("skipping: ffmpeg/ffprobe CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let media = make_media(dir.path());
    let runner = FFmpegRunner::new().expect("FFmpeg");

    for (format, expected_mime) in [("jpg", "image/jpeg"), ("png", "image/png")] {
        let thumb = make_image(dir.path(), format);
        let output = dir.path().join(format!("out_{format}.mkv"));

        runner
            .embed_thumbnail(&media, &thumb, &output, "mkv", None, None)
            .await
            .unwrap_or_else(|e| panic!("{format} embed must succeed: {e}"));

        let info = probe_last_stream(&output);
        assert_eq!(info.codec_type, "video");
        assert!(info.attached_pic);
        assert_eq!(info.mimetype.as_deref(), Some(expected_mime));
    }
}

/// #530 regression guard, failing-first: a BMP/WebP thumbnail reaching the
/// MKV embed path DIRECTLY (i.e. bypassing the postprocess-stage
/// normalization this task also adds) must be REJECTED, not silently
/// attached under a fallback mimetype. Unpatched behavior: BMP hit the
/// catch-all and was silently mislabeled `image/jpeg`; WebP was silently
/// accepted as `image/webp`, an invisible attachment. Both are wrong —
/// callers are expected to normalize first, and this path must say so rather
/// than produce a broken or invisible cover.
#[tokio::test]
async fn bmp_and_webp_reaching_mkv_embed_directly_are_rejected() {
    if !tooling_available() {
        eprintln!("skipping: ffmpeg/ffprobe CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let media = make_media(dir.path());
    let runner = FFmpegRunner::new().expect("FFmpeg");

    for format in ["bmp", "webp"] {
        let thumb = make_image(dir.path(), format);
        let output = dir.path().join(format!("out_{format}.mkv"));

        let result = runner
            .embed_thumbnail(&media, &thumb, &output, "mkv", None, None)
            .await;

        assert!(
            result.is_err(),
            "{format} reaching the raw MKV embed path unnormalized must be \
             rejected, not silently attached under a fallback mimetype"
        );
    }
}
