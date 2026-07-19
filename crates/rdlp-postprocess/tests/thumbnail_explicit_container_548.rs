//! End-to-end proof that an explicit `--remux` container is never silently
//! overridden by `ThumbnailStage`'s auto-remux-for-embedding fallback (#548).
//!
//! Reproduces the reported bug: `rdlp src.mp4 --remux=f4v` (embed_thumbnail
//! defaults to true) reported success but produced a `.mp4`, silently
//! discarding the user's explicit `--remux=f4v` instruction. Real, decodable
//! fixtures are required here (not fake paths) so the positive assertions are
//! trustworthy: a fake/nonexistent input makes the PRE-EXISTING auto-remux
//! failure warning ("Auto-remux for thumbnail embedding failed: ...")
//! coincidentally contain the fixture's own extension and the word
//! "thumbnail", which would pass even against the unpatched code.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! fixtures), mirroring `thumbnail_webp_mp4_embed.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, PostProcess, ThumbnailStage};
use rdlp_types::ContainerFormat;

mod common;
use common::{FIXTURE_FAILED, MsgOptions, build_video_fixture, ffmpeg_cli_available, make_msg};

/// Positive + regression: an explicit `--remux=f4v` container must be KEPT
/// (never auto-remuxed to mp4), the embed must be skipped, and a warning
/// must name both facts. Fails against the unpatched code, which
/// auto-remuxes the real, decodable `.f4v` fixture to a real, playable
/// `.mp4` and discards the explicit container entirely.
#[tokio::test]
async fn process_keeps_explicit_f4v_container_and_skips_embed() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.f4v");
    build_video_fixture(&media, "flv").expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        remux_container: Some(ContainerFormat::F4v),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result.tracker.primary(),
        media,
        "explicit f4v container must be kept, not auto-remuxed to mp4"
    );
    assert!(
        media.exists(),
        "the original .f4v fixture must still exist, unmodified"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("--remux=f4v") && w.contains("thumbnail")),
        "expected a warning naming both the kept container and the skipped embed, got: {:?}",
        result.warnings
    );
}

/// Positive companion: same guard for an explicit `--remux=ts` container.
#[tokio::test]
async fn process_keeps_explicit_ts_container_and_skips_embed() {
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
        remux_container: Some(ContainerFormat::Ts),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    assert_eq!(
        result.tracker.primary(),
        media,
        "explicit ts container must be kept, not auto-remuxed to mp4"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("--remux=ts") && w.contains("thumbnail")),
        "expected a warning naming both the kept container and the skipped embed, got: {:?}",
        result.warnings
    );
}

/// Code-review finding 1 (blocking): the keep-container guard's early
/// return must still clean up the orchestrator-downloaded thumbnail sidecar
/// when `--write-thumbnail` was NOT requested — mirroring the normal
/// embed-success path's `mark_temp` call. Fails against the unpatched guard,
/// which returns before `find_thumbnail` ever runs, so the sidecar is never
/// marked temp and survives `tracker.cleanup()`.
#[tokio::test]
async fn process_keeps_explicit_container_and_cleans_up_sidecar_thumbnail() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.f4v");
    build_video_fixture(&media, "flv").expect(FIXTURE_FAILED);
    // Real sidecar thumbnail discoverable via original_stem ("video").
    let thumb = dir.path().join("video.jpg");
    std::fs::write(&thumb, b"fake-jpg-bytes").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        remux_container: Some(ContainerFormat::F4v),
        write_thumbnail: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    assert_eq!(
        result.tracker.primary(),
        media,
        "explicit f4v container must still be kept"
    );

    // cleanup() deletes every path marked temp; the sidecar must be among them.
    result.tracker.cleanup();
    assert!(
        !thumb.exists(),
        "the downloaded thumbnail sidecar must be cleaned up on the \
         keep-container guard path when --write-thumbnail was not requested"
    );
}

/// Positive companion: `--write-thumbnail` must still RETAIN the sidecar on
/// the same keep-container guard path.
#[tokio::test]
async fn process_keeps_explicit_container_and_retains_sidecar_with_write_thumbnail() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.f4v");
    build_video_fixture(&media, "flv").expect(FIXTURE_FAILED);
    let thumb = dir.path().join("video.jpg");
    std::fs::write(&thumb, b"fake-jpg-bytes").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        remux_container: Some(ContainerFormat::F4v),
        write_thumbnail: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();
    assert!(
        thumb.exists(),
        "the thumbnail sidecar must be RETAINED when --write-thumbnail was requested"
    );
}

/// Code-review finding 2 (blocking): the guard condition is an EQUALITY
/// check (`remux_container == Some(current_container)`), not a presence
/// check (`remux_container.is_some()`). A `.ts` fixture with an explicit but
/// DIFFERENT requested container (`--remux=mkv`) must still take the
/// auto-remux-to-mp4 path, and the "cannot carry an embedded thumbnail"
/// warning must NOT fire. All four pre-existing #548 tests pass unchanged
/// under an `is_some()` mutation of the guard; this test is the one that
/// pins equality and fails under that mutation (verified manually — see
/// PR discussion / task report).
#[tokio::test]
async fn process_auto_remuxes_ts_when_explicit_container_differs_from_current() {
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
        // Explicit container requested, but it is NOT the current container
        // (.ts) — the guard must NOT treat this as "keep the current
        // container", since the requested container differs from it.
        remux_container: Some(ContainerFormat::Mkv),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    let out_path = result.tracker.primary();
    assert_eq!(
        out_path.extension().and_then(|e| e.to_str()),
        Some("mp4"),
        "with remux_container=Some(Mkv) over a .ts current file, the guard's \
         equality check must not match, so auto-remux-to-mp4 must still run"
    );
    assert!(
        out_path.exists(),
        "the auto-remuxed .mp4 must actually have been produced"
    );
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| w.contains("cannot carry an embedded thumbnail")),
        "the keep-container guard must not fire when the explicit container \
         differs from the current container"
    );
}

/// Negative / regression guard: with NO explicit `--remux` (rdlp-chosen
/// container, e.g. post-HLS `.ts`), the auto-remux-to-mp4 fallback must
/// still run and actually produce a real `.mp4` — proving the new
/// keep-container guard is scoped strictly to an explicit user request and
/// does not regress the HLS-style auto-remux path.
#[tokio::test]
async fn process_still_auto_remuxes_ts_to_mp4_when_no_explicit_container() {
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
        remux_container: None,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, MsgOptions::default());

    let result = stage.process(msg).await.expect("non-fatal stage");

    let out_path = result.tracker.primary();
    assert_eq!(
        out_path.extension().and_then(|e| e.to_str()),
        Some("mp4"),
        "with no explicit container, .ts must still auto-remux to a real .mp4 (no regression)"
    );
    assert!(
        out_path.exists(),
        "the auto-remuxed .mp4 must actually have been produced"
    );
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| w.contains("cannot carry an embedded thumbnail")),
        "the keep-container guard must not fire when no explicit container was requested"
    );
}
