//! End-to-end proof that an explicit **recode** container (`--recode-video` /
//! `--recode-container`) is never silently overridden by `ThumbnailStage`'s
//! auto-remux-for-embedding fallback (#551).
//!
//! #548 fixed this for `--remux` only; its guard keyed on `remux_container`
//! alone, so `RecodeStage`'s own target (`recode_container` > `recode_video`)
//! fell straight through to the auto-remux-to-mp4 fallback. Reproduced with
//! the real binary before the fix: `rdlp src.mp4 --recode-video=ts` exited 0,
//! printed "Success!", and produced only `src.mp4` — no `.ts` anywhere.
//!
//! Real, decodable fixtures are mandatory here (not fake paths). #548's task
//! report records the trap: a nonexistent input makes the PRE-EXISTING
//! auto-remux failure warning coincidentally contain the fixture's own
//! extension and the word "thumbnail", so a fake-path test passes against the
//! UNPATCHED code and proves nothing.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixtures), mirroring `thumbnail_explicit_container_548.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, PostProcess, ThumbnailStage};
use rdlp_types::ContainerFormat;

mod common;
use common::{FIXTURE_FAILED, MsgOptions, build_video_fixture, ffmpeg_cli_available, make_msg};

/// Reproduction of the reported bug: an explicit `--recode-video=ts` must be
/// KEPT, the embed skipped, and a warning must name both the flag that won
/// and the skipped embed. Fails against the unpatched guard, which sees
/// `remux_container == None`, abstains, and auto-remuxes the real `.ts`
/// fixture to a real `.mp4`.
#[tokio::test]
async fn process_keeps_explicit_recode_video_container_and_skips_embed() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.ts");
    build_video_fixture(&media, "mpegts").expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        recode_video: Some(ContainerFormat::Ts),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result.tracker.primary(),
        media,
        "explicit --recode-video=ts container must be kept, not auto-remuxed to mp4"
    );
    assert!(
        media.exists(),
        "the original .ts fixture must still exist, unmodified"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("--recode-video=ts") && w.contains("thumbnail")),
        "expected a warning naming the winning flag and the skipped embed, got: {:?}",
        result.warnings
    );
}

/// Same guard via the independent `--recode-container` flag, which takes
/// precedence over `--recode-video` (mirroring `RecodeStage`'s own
/// `recode_container.or(recode_video)` resolution).
#[tokio::test]
async fn process_keeps_explicit_recode_container_and_skips_embed() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.ts");
    build_video_fixture(&media, "mpegts").expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        recode_container: Some(ContainerFormat::Ts),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result.tracker.primary(),
        media,
        "explicit --recode-container=ts container must be kept"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("--recode-container=ts") && w.contains("thumbnail")),
        "expected a warning naming --recode-container, got: {:?}",
        result.warnings
    );
}

/// Precedence pin: when BOTH a recode target and a `--remux` target are set,
/// the recode target owns the final container — `RecodeStage` (index 4) runs
/// AFTER `RemuxStage` (index 3), so the file reaching `ThumbnailStage` is the
/// recode target. Fails against a naive fix that puts `remux_container` first
/// in the chain: the warning would name `--remux=mkv` instead.
#[tokio::test]
async fn recode_target_outranks_remux_target_in_the_kept_container_warning() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.ts");
    build_video_fixture(&media, "mpegts").expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        recode_video: Some(ContainerFormat::Ts),
        remux_container: Some(ContainerFormat::Mkv),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result.tracker.primary(),
        media,
        "the recode target (.ts) is the container that actually reached this stage"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("--recode-video=ts")),
        "the recode target must win the precedence chain, got: {:?}",
        result.warnings
    );
    assert!(
        !result.warnings.iter().any(|w| w.contains("--remux=mkv")),
        "the outranked --remux target must not be named, got: {:?}",
        result.warnings
    );
}

/// Equality pin (mutation guard). The guard is an EQUALITY check against the
/// container actually on disk, not a presence check. A `.ts` file with an
/// explicit but DIFFERENT recode target (`--recode-video=mkv`) must still take
/// the auto-remux-to-mp4 path, and the keep-container warning must NOT fire.
///
/// This is the test that dies under an `is_some()` mutation of the guard —
/// #548's report records that all four of its other tests survived exactly
/// that mutation, so this shape is mandatory, not optional.
#[tokio::test]
async fn process_auto_remuxes_when_explicit_recode_container_differs_from_current() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.ts");
    build_video_fixture(&media, "mpegts").expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        // Explicit, but NOT the container on disk (.ts) — the guard must not
        // treat this as "keep the current container".
        recode_video: Some(ContainerFormat::Mkv),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result
            .tracker
            .primary()
            .extension()
            .and_then(|e| e.to_str()),
        Some("mp4"),
        "a .ts file whose explicit target is mkv must still auto-remux to mp4 here"
    );
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| w.contains("cannot carry an embedded thumbnail")),
        "the keep-container warning must not fire when the explicit target differs, got: {:?}",
        result.warnings
    );
}

/// Regression guard for the rdlp-chosen path: a post-HLS `.ts` with NO
/// explicit container anywhere must still auto-remux to mp4 for embedding.
/// This is the behavior the guard must never swallow — it is the reason the
/// guard is an equality check against an explicit request rather than a blanket
/// "keep whatever extension is on disk".
#[tokio::test]
async fn hls_ts_without_any_explicit_container_still_auto_remuxes() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.ts");
    build_video_fixture(&media, "mpegts").expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        // No recode_video / recode_container / remux_container at all.
        ..PostProcess::default()
    };
    let msg = make_msg(
        vec![media],
        config,
        MsgOptions {
            is_hls: true,
            ..MsgOptions::default()
        },
    );

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result
            .tracker
            .primary()
            .extension()
            .and_then(|e| e.to_str()),
        Some("mp4"),
        "an rdlp-chosen .ts (no explicit request) must still auto-remux to mp4"
    );
}

/// Sidecar lifecycle on the recode guard path: the early return must still
/// mark the orchestrator-downloaded thumbnail temp when `--write-thumbnail`
/// was not requested. This is the defect code review caught on #548 (the
/// early return bypassed the only other `mark_temp` site); the recode path
/// takes the same early return and needs the same coverage.
#[tokio::test]
async fn recode_guard_path_cleans_up_sidecar_thumbnail() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.ts");
    build_video_fixture(&media, "mpegts").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("video.jpg");
    std::fs::write(&thumb, b"fake-jpg-bytes").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        recode_video: Some(ContainerFormat::Ts),
        write_thumbnail: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    assert_eq!(result.tracker.primary(), media);

    result.tracker.cleanup();
    assert!(
        !thumb.exists(),
        "the downloaded sidecar must be cleaned up on the recode keep-container path"
    );
}
