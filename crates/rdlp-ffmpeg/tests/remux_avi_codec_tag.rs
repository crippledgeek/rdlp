//! Regression test for #549: AVI remux codec-tag policy.
//!
//! `add_stream_copy` unconditionally zeroed the source codec tag for every
//! non-Matroska output. For AVI that is actively harmful:
//! - h264 (`avc1`) -> AVI: the zeroed tag lets AVI's muxer auto-fill a
//!   literal `'H264'` fourcc, which arms `avienc.c`'s start-code guard and
//!   hard-fails AVCC-packaged H.264 (no start codes in the bitstream).
//! - hevc (`hvc1`) -> AVI: AVI's tag table has NO entry for HEVC at all, so
//!   the tag stays 0 and `riff.c` silently maps a zero fourcc to raw video
//!   — exit 0, corrupt file.
//!
//! Fix: query the target muxer's codec-tag table (`avformat_query_codec`)
//! per stream and PRESERVE the source tag when the table has an entry,
//! REJECT before writing when it does not — rather than let a zeroed tag
//! silently decode as something else. This is a convention this `FFmpeg`
//! build happens to follow, not a documented container standard — no spec
//! covers H.264/HEVC-in-AVI.
//!
//! Self-skips when the `ffmpeg`/`ffprobe` CLI is absent (used only to build
//! fixtures and to independently verify decode integrity — never rdlp's own
//! code doing the checking).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use rdlp_ffmpeg::{FFmpegRunner, RemuxOptions};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("ffprobe")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Build a short (1s) fixture with the given video encoder and container.
/// Returns `None` if the encoder isn't compiled into the linked `FFmpeg`.
fn build_fixture(dir: &Path, stem: &str, ext: &str, vcodec: &str) -> Option<PathBuf> {
    let path = dir.join(format!("{stem}.{ext}"));
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc=d=1:s=320x240:r=25"])
        .args(["-f", "lavfi", "-i", "sine=d=1"])
        .args(["-c:v", vcodec, "-pix_fmt", "yuv420p"])
        .args(["-c:a", "aac", "-b:a", "64k", "-shortest"])
        .arg(&path)
        .status()
        .ok()?;
    status.success().then_some(path)
}

/// True if `ffmpeg -f null` can decode every frame of `path` without error.
/// An independent check via the system CLI, never rdlp's own decode path.
fn decodes_cleanly(path: &Path) -> bool {
    Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .map(|o| o.status.success() && o.stderr.is_empty())
        .unwrap_or(false)
}

/// Fixtures shared across every test in this file, built once (an `ffmpeg`
/// subprocess per source is real work — this is the "cached, not
/// regenerated per test" fixture idiom used across this crate's test suite).
struct Fixtures {
    dir: tempfile::TempDir,
    h264_mp4: Option<PathBuf>,
    hevc_mp4: Option<PathBuf>,
    ts: Option<PathBuf>,
    mkv: Option<PathBuf>,
}

static FIXTURES: OnceLock<Fixtures> = OnceLock::new();

fn fixtures() -> &'static Fixtures {
    FIXTURES.get_or_init(|| {
        let dir = tempfile::tempdir().expect("tempdir");
        let h264_mp4 = build_fixture(dir.path(), "h264", "mp4", "libx264");
        let hevc_mp4 = build_fixture(dir.path(), "hevc", "mp4", "libx265");
        let ts = build_fixture(dir.path(), "src", "ts", "libx264");
        let mkv = build_fixture(dir.path(), "src", "mkv", "libx264");
        Fixtures {
            dir,
            h264_mp4,
            hevc_mp4,
            ts,
            mkv,
        }
    })
}

/// Remux `src` to `<label>.<target_ext>` and assert the output decodes
/// cleanly (an independent `ffmpeg -f null -` check, not rdlp's own path).
async fn assert_remux_decodes(runner: &FFmpegRunner, src: &Path, target_ext: &str, label: &str) {
    let dst = fixtures().dir.path().join(format!("{label}.{target_ext}"));
    runner
        .remux(src, &dst, &RemuxOptions::default(), None)
        .await
        .unwrap_or_else(|e| panic!("{label}: remux failed: {e:#}"));
    assert!(
        decodes_cleanly(&dst),
        "{label}: output does not decode cleanly"
    );
}

/// The positive case: h264 must remux into AVI and the result must decode.
/// Fails against unpatched code (`avienc.c`'s start-code guard rejects the
/// muxer-auto-filled `'H264'` tag on AVCC-packaged H.264).
#[tokio::test]
async fn h264_mp4_to_avi_preserves_tag_and_decodes() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg/ffprobe not available");
        return;
    }
    let Some(src) = fixtures().h264_mp4.clone() else {
        eprintln!("[SKIP] libx264 fixture build failed");
        return;
    };
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");
    assert_remux_decodes(&runner, &src, "avi", "h264_to_avi").await;
}

/// The negative case: HEVC cannot be represented in AVI (this build's tag
/// table has no entry for it) — the remux must be REJECTED with an
/// actionable message, not silently written as a corrupt (rawvideo-tagged)
/// file. Fails against unpatched code, which returns `Ok(())` and leaves a
/// corrupt file with `AV_CODEC_ID_RAWVIDEO` on read-back.
#[tokio::test]
async fn hevc_mp4_to_avi_is_rejected_with_actionable_message() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg/ffprobe not available");
        return;
    }
    let Some(src) = fixtures().hevc_mp4.clone() else {
        eprintln!("[SKIP] libx265 fixture build failed");
        return;
    };
    let dst = fixtures().dir.path().join("hevc_to_avi.avi");
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let err = runner
        .remux(&src, &dst, &RemuxOptions::default(), None)
        .await
        .expect_err("AVI cannot represent HEVC; the remux must be rejected");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("avi") && msg.contains("hevc"),
        "error should name the container/codec pairing, got: {msg}"
    );
}

/// Regression guard matrix: every pairing that worked before this fix must
/// keep working — the MKV raw-FFI path (untouched) and every non-AVI,
/// non-tag-rejecting target reachable from the generic remux path.
#[tokio::test]
async fn remux_regression_matrix_still_decodes() {
    if !ffmpeg_available() {
        eprintln!("[SKIP] ffmpeg/ffprobe not available");
        return;
    }
    let runner = FFmpegRunner::new().expect("FFmpegRunner::new");

    let cases: &[(&str, Option<PathBuf>, &str)] = &[
        ("mp4_to_mkv", fixtures().h264_mp4.clone(), "mkv"),
        ("ts_to_mkv", fixtures().ts.clone(), "mkv"),
        ("hevc_to_mkv", fixtures().hevc_mp4.clone(), "mkv"),
        ("mp4_to_mp4", fixtures().h264_mp4.clone(), "mp4"),
        ("mp4_to_mov", fixtures().h264_mp4.clone(), "mov"),
        ("mp4_to_flv", fixtures().h264_mp4.clone(), "flv"),
        ("mp4_to_ts", fixtures().h264_mp4.clone(), "ts"),
        ("mkv_to_mp4", fixtures().mkv.clone(), "mp4"),
    ];

    for (label, src, target_ext) in cases {
        let Some(src) = src else {
            eprintln!("[SKIP] {label}: source fixture build failed");
            continue;
        };
        assert_remux_decodes(&runner, src, target_ext, label).await;
    }
}
