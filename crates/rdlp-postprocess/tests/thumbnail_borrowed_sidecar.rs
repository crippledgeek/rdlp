//! A user-owned sidecar image next to a **borrowed** (local, user-supplied)
//! input must never be deleted by `ThumbnailStage`.
//!
//! `ThumbnailStage` discovers its thumbnail by stem-matching next to the media
//! file (`{stem}.{jpg,png,webp,...}`) and, when `--write-thumbnail` was not
//! requested, marks that file temp so the pipeline deletes it after embedding.
//! That is correct for a thumbnail *rdlp itself downloaded* — but for
//! `rdlp /home/me/myvideo.mp4` (local-file post-processing, `new_borrowing`),
//! the discovered `myvideo.jpg` is the USER'S OWN FILE and deleting it is
//! silent data loss.
//!
//! Verified against the release binary before the fix:
//!   `rdlp myvideo.mp4` and `rdlp myvideo.mp4 --remux=ts` both destroyed a real
//!   JPEG sitting next to the user's own source file.
//!
//! This is the #414 incident class (local-file source preservation) applied to
//! sidecars rather than to the media file. `FileTracker::mark_temp`'s
//! `debug_assert!(!is_borrowed(..))` does NOT cover it: the assert is a no-op
//! in release builds, and the sidecar was never in the borrowed set anyway —
//! only the video was.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, PostProcess, RemuxStage, ThumbnailStage};
use rdlp_types::ContainerFormat;

mod common;
use common::{
    FIXTURE_FAILED, build_jpeg_fixture, build_video_fixture, ffmpeg_cli_available, make_msg, opts,
};

/// A run over the user's OWN local file.
fn borrowed_opts() -> common::MsgOptions {
    opts("myvideo", true)
}

/// A run over a file rdlp downloaded, sidecar included.
fn owned_opts() -> common::MsgOptions {
    opts("myvideo", false)
}

/// The embed-success path (`thumbnail.rs`'s `mark_temp(path)` after a
/// successful embed): an mp4 CAN carry the
/// thumbnail, so the embed runs and then cleans up the sidecar. With a
/// borrowed input that sidecar is the user's own file. Fails against the
/// unpatched code, which deletes it.
#[tokio::test]
async fn embed_path_never_deletes_a_user_owned_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("myvideo.jpg");
    build_jpeg_fixture(&thumb).expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        write_thumbnail: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, borrowed_opts());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        thumb.exists(),
        "the user's own myvideo.jpg must survive post-processing of their own local file"
    );
}

/// The keep-container guard path (`ThumbnailStage::process`'s early return
/// for an explicitly-requested container, added by #548 and
/// widened by #551): `.ts` cannot carry a thumbnail, so the guard returns
/// early and cleans up the sidecar on the way out. Same data loss, different
/// branch — both sites need the check.
///
/// **Runs the REAL two-stage sequence** (`RemuxStage` → `ThumbnailStage`)
/// rather than feeding a `.ts` in directly. That distinction is the whole
/// point of this test: `RemuxStage` replaces the user's borrowed `myvideo.mp4`
/// with an rdlp-created `myvideo.ts`, so by the time `ThumbnailStage` runs,
/// `is_borrowed(current_file)` is FALSE even though the run started from the
/// user's own file. A single-stage test that passes a borrowed `.ts` straight
/// in cannot observe that, and an earlier version of this fix passed such a
/// test while the release binary still destroyed the user's JPEG.
#[tokio::test]
async fn keep_container_guard_never_deletes_a_user_owned_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("myvideo.jpg");
    build_jpeg_fixture(&thumb).expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));

    let config = PostProcess {
        embed_thumbnail: true,
        write_thumbnail: false,
        remux_container: Some(ContainerFormat::Ts),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, borrowed_opts());

    // Stage 3: turns the borrowed .mp4 into an rdlp-created .ts.
    let remuxed = RemuxStage::new(Arc::clone(&ffmpeg))
        .process(msg)
        .await
        .expect("remux stage");
    assert_eq!(
        remuxed
            .tracker
            .primary()
            .extension()
            .and_then(|e| e.to_str()),
        Some("ts"),
        "precondition: RemuxStage must have produced the .ts the guard then keeps"
    );

    // Stage 7: the keep-container guard fires and cleans up on the way out.
    let mut result = ThumbnailStage::new(ffmpeg)
        .process(remuxed)
        .await
        .expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        thumb.exists(),
        "the user's own sidecar must survive the keep-container guard path too"
    );
}

/// Regression guard for the DOWNLOAD path, which must be unaffected: when the
/// media file is NOT borrowed, the sidecar is a thumbnail rdlp downloaded
/// itself and `--write-thumbnail=false` must still delete it. This is the test
/// that dies if the fix is over-broad (e.g. "never delete any sidecar").
#[tokio::test]
async fn downloaded_sidecar_is_still_cleaned_up_when_not_borrowed() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("myvideo.jpg");
    build_jpeg_fixture(&thumb).expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        write_thumbnail: false,
        ..PostProcess::default()
    };
    // NOT borrowing: this is a file rdlp downloaded, sidecar included.
    let msg = make_msg(vec![media], config, owned_opts());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        !thumb.exists(),
        "an rdlp-downloaded sidecar must still be cleaned up without --write-thumbnail"
    );
}

/// Disk-leak guard on the embed-FAILURE branch (found by the pre-push
/// security review of #553). The `Err(..)` arm marked the stage's own
/// `temp_output` temp but never the discovered sidecar, so an rdlp-downloaded
/// thumbnail that failed to embed was left on disk next to the output.
///
/// Deliberately on the DOWNLOAD path: with a user-owned sidecar the correct
/// behaviour is to retain it, so a borrowed fixture could not distinguish the
/// leak from the intended retention. The embed is made to fail deterministically
/// by giving the sidecar a valid image EXTENSION but garbage bytes — nothing
/// can decode it, so both the transcode and the embed fail.
#[tokio::test]
async fn embed_failure_still_cleans_up_a_downloaded_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("myvideo.jpg");
    std::fs::write(&thumb, b"not-a-decodable-image").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        write_thumbnail: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, owned_opts());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        !thumb.exists(),
        "an rdlp-downloaded sidecar must not be left on disk when the embed fails"
    );
}

/// Companion: the same failure on the BORROWED path must still retain the
/// user's file. This is what stops the leak fix from becoming a data-loss fix
/// in disguise.
#[tokio::test]
async fn embed_failure_still_retains_a_user_owned_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("myvideo.jpg");
    std::fs::write(&thumb, b"not-a-decodable-image").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        write_thumbnail: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, borrowed_opts());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        thumb.exists(),
        "the user's own sidecar must survive even when the embed fails"
    );
}

/// Pins the OTHER half of the embed-path gate: `--write-thumbnail` must
/// retain the sidecar on the DOWNLOAD path, where ownership alone would
/// happily delete it. Deliberately NOT the borrowed path — with
/// `borrowing: true` the ownership term already makes
/// `!write_thumbnail && is_disposable()` false, so the assertion holds no
/// matter what the `write_thumbnail` term does, and deleting that term from
/// the embed-success site leaves the whole suite green (verified by
/// mutation — the same trap that was caught in the subtitle suite).
#[tokio::test]
async fn write_thumbnail_retains_downloaded_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("myvideo.jpg");
    build_jpeg_fixture(&thumb).expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        write_thumbnail: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, owned_opts());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(thumb.exists(), "--write-thumbnail must retain the sidecar");
}
