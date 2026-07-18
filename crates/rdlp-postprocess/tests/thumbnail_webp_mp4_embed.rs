//! End-to-end proof that embedding a webp thumbnail into an MP4 succeeds
//! (bugfix/thumbnail-webp-mp4-embed).
//!
//! Reproduces the reported bug: embedding a webp thumbnail (as served by
//! xHamster and others) into an MP4 used to fail muxing with "Could not find
//! tag for codec webp in stream #2, codec not currently supported in
//! container" because `embed_thumbnail_sync` stream-copies the thumbnail's
//! source codec and the MP4 muxer has no tag for webp. Verified RED against
//! the unpatched `ThumbnailStage::process` (temporarily bypassing the
//! normalization call to feed the raw webp straight to `embed_thumbnail`,
//! mirroring pre-fix behavior; confirmed the exact `write_header` failure)
//! before the normalization fix landed.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! webp/jpg/mp4 fixtures).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use rdlp_postprocess::pipeline::{FileTracker, PipelineMessage, PipelineStage, TempRegistry};
use rdlp_postprocess::{FFmpegRunner, PostProcess, ThumbnailStage};
use rdlp_types::InfoDict;

fn ffmpeg_cli_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Build a 1-frame H.264 mp4 fixture via the system `ffmpeg` CLI (test-only;
/// mirrors `crates/rdlp-ffmpeg/tests/recode_cancel.rs`'s fixture pattern).
fn build_mp4_fixture(path: &Path) -> Result<(), ()> {
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=320x240",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            path.to_str().unwrap(),
        ])
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(()) }
}

/// Build a single-frame still-image thumbnail via the system `ffmpeg` CLI.
/// The output codec/container is inferred from `path`'s extension (e.g.
/// `.webp` -> webp, `.jpg` -> mjpeg), so this fixture builder covers both the
/// webp regression case and the jpg pass-through case.
fn build_still_image_fixture(path: &Path) -> Result<(), ()> {
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=d=1:s=320x240",
            "-frames:v",
            "1",
            path.to_str().unwrap(),
        ])
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(()) } else { Err(()) }
}

fn make_msg(files: Vec<std::path::PathBuf>, config: PostProcess) -> PipelineMessage {
    let reg = Arc::new(TempRegistry::new());
    let (error_tx, _) = oneshot::channel();
    PipelineMessage {
        info: InfoDict::new(
            "id".to_string(),
            "Test Video".to_string(),
            "TestExtractor".to_string(),
            "https://example.com".to_string(),
        ),
        tracker: FileTracker::new(files, reg),
        config: Arc::new(config),
        original_stem: "video".to_string(),
        is_hls: false,
        verbose: false,
        callback_factory: None,
        error_tx: Some(error_tx),
        warnings: Vec::new(),
        encoding_tool: None,
        cancel: CancellationToken::new(),
    }
}

/// Positive + regression test: a webp thumbnail must embed into an MP4 via
/// `ThumbnailStage::process` with no warnings, and the resulting file must
/// carry a second (attached-pic) video stream encoded as `mjpeg` — proving
/// the thumbnail was normalized to an MP4-safe codec rather than
/// stream-copied as webp.
#[tokio::test]
async fn process_embeds_webp_thumbnail_into_mp4_as_mjpeg() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.mp4");
    let thumb = dir.path().join("video.webp");
    if build_mp4_fixture(&media).is_err() || build_still_image_fixture(&thumb).is_err() {
        eprintln!("[SKIP] fixture build failed");
        return;
    }

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg.clone());

    let config = PostProcess {
        embed_thumbnail: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config);

    let result = stage.process(msg).await.expect("non-fatal stage");
    assert!(
        result.warnings.is_empty(),
        "webp thumbnail embed should succeed with no warnings, got: {:?}",
        result.warnings
    );

    let out_path = result.tracker.primary();
    let info = ffmpeg
        .probe(&out_path)
        .await
        .expect("probing embedded output must succeed");
    assert_eq!(
        info.stream_count, 2,
        "expected media stream + attached-pic thumbnail stream"
    );
    let thumb_stream = info
        .streams
        .iter()
        .find(|s| s.index == 1)
        .expect("second stream (thumbnail) must be present");
    assert_eq!(thumb_stream.codec_type, "video");
    assert_eq!(
        thumb_stream.codec_name.as_deref(),
        Some("mjpeg"),
        "thumbnail stream must be normalized to mjpeg, not stream-copied webp"
    );
}

/// Negative companion: a jpg thumbnail is already MP4-embeddable and must
/// pass straight through the normalization gate (no re-transcode attempt) —
/// the embed must still succeed with no warnings.
#[tokio::test]
async fn process_embeds_jpg_thumbnail_without_normalizing() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.mp4");
    let thumb = dir.path().join("video.jpg");
    if build_mp4_fixture(&media).is_err() || build_still_image_fixture(&thumb).is_err() {
        eprintln!("[SKIP] fixture build failed");
        return;
    }

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg.clone());

    let config = PostProcess {
        embed_thumbnail: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config);

    let result = stage.process(msg).await.expect("non-fatal stage");
    assert!(
        result.warnings.is_empty(),
        "jpg thumbnail embed should succeed with no warnings, got: {:?}",
        result.warnings
    );
}

/// Regression guard: the Matroska path is untouched by the normalization
/// fix — an mkv container must still embed a webp thumbnail directly (native
/// Matroska attachment, no normalization), unlike the MP4-family path above.
#[tokio::test]
async fn process_embeds_webp_thumbnail_into_mkv_natively() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.mkv");
    let thumb = dir.path().join("video.webp");
    let mp4_src = dir.path().join("src.mp4");
    if build_mp4_fixture(&mp4_src).is_err() || build_still_image_fixture(&thumb).is_err() {
        eprintln!("[SKIP] fixture build failed");
        return;
    }
    // Remux the mp4 fixture into mkv so the media file under test is a real MKV container.
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            mp4_src.to_str().unwrap(),
            "-c",
            "copy",
            media.to_str().unwrap(),
        ])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("[SKIP] mkv fixture build failed");
        return;
    }

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg);

    let config = PostProcess {
        embed_thumbnail: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config);

    let result = stage.process(msg).await.expect("non-fatal stage");
    assert!(
        result.warnings.is_empty(),
        "mkv native webp attachment should succeed with no warnings, got: {:?}",
        result.warnings
    );
}

/// The reported #525 shape, end to end: WebP bytes carried by a file NAMED
/// `.jpg`, because the CDN served WebP from a `.jpg` URL path.
///
/// The unit tests pin the individual decisions; this pins the symptom at the
/// level it was reported. It fails against the pre-#525 code, where the gate
/// read the `.jpg` name as "already embeddable", skipped normalization, and
/// stream-copied raw WebP into the MP4 muxer:
///
/// ```text
/// Could not find tag for codec webp in stream #2, codec not currently supported in container
/// ```
///
/// Covers gate + normalize + mux + covr in one pass.
#[tokio::test]
async fn process_embeds_mislabeled_webp_named_jpg_into_mp4() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.mp4");

    // Build a genuine WebP, then give it a .jpg name — the mislabel itself.
    let real_webp = dir.path().join("source.webp");
    if build_mp4_fixture(&media).is_err() || build_still_image_fixture(&real_webp).is_err() {
        eprintln!("[SKIP] fixture build failed");
        return;
    }
    let mislabeled = dir.path().join("video.jpg");
    std::fs::rename(&real_webp, &mislabeled).expect("rename webp to .jpg");

    // Sanity-check the fixture really is the mislabel we intend to test.
    let header = std::fs::read(&mislabeled).expect("read fixture");
    assert_eq!(
        &header[0..4],
        b"RIFF",
        "fixture must actually be webp bytes under a .jpg name"
    );

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = ThumbnailStage::new(ffmpeg.clone());

    let config = PostProcess {
        embed_thumbnail: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config);

    let result = stage.process(msg).await.expect("non-fatal stage");
    assert!(
        result.warnings.is_empty(),
        "a mislabeled webp-as-.jpg thumbnail must still embed cleanly, got: {:?}",
        result.warnings
    );

    let out_path = result.tracker.primary();
    let info = ffmpeg
        .probe(&out_path)
        .await
        .expect("probing embedded output must succeed");
    let thumb_stream = info
        .streams
        .iter()
        .find(|s| s.index == 1)
        .expect("thumbnail stream must be present");
    assert_eq!(
        thumb_stream.codec_name.as_deref(),
        Some("mjpeg"),
        "the mislabeled webp must be normalized to mjpeg — its .jpg NAME must \
         not have been taken as proof it was already embeddable"
    );
}
