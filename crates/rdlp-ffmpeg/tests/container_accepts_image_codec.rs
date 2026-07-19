//! `container_accepts_image_codec` asks the muxer, not a hardcoded whitelist (#525).
//!
//! Whether a thumbnail must be transcoded before embedding is a property of the
//! linked `FFmpeg` build's muxer tables — the same lookup that produced the
//! original failure, `Could not find tag for codec webp in stream #2`. These
//! tests pin the answers this build actually gives, so a hand-maintained
//! whitelist can never drift from the muxer.
//!
//! Self-skips when the `ffmpeg` CLI is absent (used only to build fixtures).
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

/// The load-bearing case: MP4 cannot store WebP, which is why an unnormalized
/// webp thumbnail failed to mux. If this ever returns true for this build, the
/// normalization step would be skipped and the embed would break again.
#[tokio::test]
async fn mp4_rejects_webp() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let webp = make_image(dir.path(), "webp");
    let runner = FFmpegRunner::new().expect("FFmpeg");

    assert!(
        !runner
            .container_accepts_image_codec("mp4", &webp)
            .await
            .expect("query must succeed"),
        "mp4 must NOT accept webp — this is the #525 mux failure"
    );
}

/// The formats the previous hardcoded whitelist allowed must still be allowed,
/// or every thumbnail would take a pointless transcode.
#[tokio::test]
async fn mp4_accepts_jpeg_and_png() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let runner = FFmpegRunner::new().expect("FFmpeg");

    for format in ["jpg", "png"] {
        let img = make_image(dir.path(), format);
        assert!(
            runner
                .container_accepts_image_codec("mp4", &img)
                .await
                .expect("query must succeed"),
            "mp4 must accept {format} without transcoding"
        );
    }
}

/// Matroska ALSO reports webp as unstorable — and that is correct, because
/// this query asks whether a codec can be stored as a STREAM.
///
/// rdlp's MKV path does not embed the thumbnail as a stream at all: it writes a
/// Matroska *attachment*, an arbitrary file carried with a MIME type, which
/// bypasses codec tags entirely. So the muxer's "no" here does not contradict
/// MKV thumbnail support — the two mechanisms are different.
///
/// This is why the embed gate must test `is_native_attachment_container` BEFORE
/// consulting this query: asking the stream question about a container that
/// uses attachments would force a needless transcode for every MKV thumbnail.
#[tokio::test]
async fn matroska_rejects_webp_as_a_stream_despite_attachment_support() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let webp = make_image(dir.path(), "webp");
    let runner = FFmpegRunner::new().expect("FFmpeg");

    assert!(
        !runner
            .container_accepts_image_codec("mkv", &webp)
            .await
            .unwrap(),
        "matroska cannot carry webp as a stream; MKV thumbnails ride the \
         attachment path instead, which this query deliberately does not model"
    );
}

// Codec-specificity within a container is already pinned by the pair above:
// mp4 accepts jpeg/png and rejects webp. A separate assertion that Matroska
// distinguishes codecs was removed after the muxer answered "no" for png as a
// stream too — MKV thumbnails ride the attachment path, so the stream-codec
// question is simply not the one that governs that container.

/// An unrecognized container is a "cannot store it" answer, not an error.
#[tokio::test]
async fn unknown_container_is_not_supported() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let jpg = make_image(dir.path(), "jpg");
    let runner = FFmpegRunner::new().expect("FFmpeg");

    assert!(
        !runner
            .container_accepts_image_codec("notacontainer", &jpg)
            .await
            .expect("an unknown container must not be an error"),
    );
}

/// Container matrix for the JPEG/PNG baseline.
///
/// This is the coverage gap that let a regression through: the suite only
/// exercised mp4 and mkv, so it never noticed that `avformat_query_codec`
/// reports "cannot store" for jpeg/png on mp3, flac, m4a and m4v — containers
/// that embed them perfectly well. Trusting the raw query there would
/// re-encode a lossless PNG cover to lossy JPEG.
///
/// Every container rdlp lists as thumbnail-capable must accept both baseline
/// formats, regardless of what the muxer's tag table says.
#[tokio::test]
async fn jpeg_and_png_are_accepted_by_every_supported_container() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let runner = FFmpegRunner::new().expect("FFmpeg");

    // Containers where the jpeg/png baseline is VERIFIED to embed.
    //
    // Deliberately narrower than SUPPORTED_CONTAINERS: `ogg` and `opus` are
    // listed there but genuinely cannot carry an image STREAM at all — their
    // muxer's `ogg_init()` hard-rejects any stream whose codec isn't
    // Vorbis/Theora/Speex/FLAC/Opus/VP8, which is exactly the question this
    // function asks. #531 fixed the actual embed by carrying the cover as a
    // `METADATA_BLOCK_PICTURE` VorbisComment field instead of an
    // `ATTACHED_PIC` stream (see `rdlp-ffmpeg/src/ffmpeg/thumbnail/
    // vorbis_picture.rs`), so the embed itself no longer fails — but that
    // path bypasses this stream-codec query entirely, the same situation as
    // Matroska's attachment path documented above. Do NOT "complete" this
    // list with ogg/opus: the gate would still (correctly) answer `true` for
    // jpeg/png here via the baseline, which says nothing about whether the
    // embed works — asserting on it would test the wrong mechanism.
    let containers = ["mp4", "m4a", "m4v", "mov", "mkv", "mka", "mp3", "flac"];

    for format in ["jpg", "png"] {
        let img = make_image(dir.path(), format);
        for container in containers {
            assert!(
                runner
                    .container_accepts_image_codec(container, &img)
                    .await
                    .expect("query must succeed"),
                "{format} must be accepted by {container} without transcoding — \
                 the muxer's tag table under-reports here, so the baseline must \
                 carry it"
            );
        }
    }
}

/// The baseline must not swallow the actual bug: webp is still refused by the
/// MP4 family, which is what forces normalization.
#[tokio::test]
async fn webp_is_still_refused_by_mp4_family() {
    if !ffmpeg_cli_available() {
        eprintln!("skipping: ffmpeg CLI not available");
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let webp = make_image(dir.path(), "webp");
    let runner = FFmpegRunner::new().expect("FFmpeg");

    for container in ["mp4", "m4a", "m4v", "mov"] {
        assert!(
            !runner
                .container_accepts_image_codec(container, &webp)
                .await
                .expect("query must succeed"),
            "{container} must still refuse webp — this is the #525 mux failure"
        );
    }
}
