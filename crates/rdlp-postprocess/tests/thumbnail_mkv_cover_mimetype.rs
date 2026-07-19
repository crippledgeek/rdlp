//! #530 end-to-end: `ThumbnailStage::process` must route BMP/GIF/TIFF/WebP
//! thumbnails correctly for Matroska, mirroring `thumbnail_webp_mp4_embed.rs`'s
//! pattern for the MP4-family path.
//!
//! GIF and TIFF attach as a real, player-visible Matroska cover with no
//! transcoding (`FFmpeg`'s own attachment-mimetype read-back table
//! recognizes `image/gif`/`image/tiff`); BMP does not (like WebP, covered in
//! `thumbnail_webp_mp4_embed.rs`) and must be normalized to mjpeg first, or
//! the resulting attachment silently fails to render as a cover in any
//! player.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! image/mkv fixtures).
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
/// The output codec/container is inferred from `path`'s extension.
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

fn build_mkv_fixture(dir: &Path) -> Result<std::path::PathBuf, ()> {
    let mp4_src = dir.join("src.mp4");
    let mkv = dir.join("video.mkv");
    build_mp4_fixture(&mp4_src)?;
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-i",
            mp4_src.to_str().unwrap(),
            "-c",
            "copy",
            mkv.to_str().unwrap(),
        ])
        .status()
        .is_ok_and(|s| s.success());
    if ok { Ok(mkv) } else { Err(()) }
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

/// Positive + regression guard: GIF and TIFF attach into Matroska as a real,
/// player-visible cover with NO transcoding — the resulting attached-pic
/// stream's codec must be the thumbnail's own (`gif`/`tiff`), not `mjpeg`.
#[tokio::test]
async fn process_embeds_gif_and_tiff_thumbnails_into_mkv_natively() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();

    for (format, expected_codec) in [("gif", "gif"), ("tiff", "tiff")] {
        let media = dir.path().join(format!("media_{format}.mkv"));
        let thumb = dir.path().join(format!("media_{format}.{format}"));
        let Ok(built) = build_mkv_fixture(dir.path()) else {
            eprintln!("[SKIP] fixture build failed");
            return;
        };
        std::fs::rename(&built, &media).expect("rename mkv fixture");
        if build_still_image_fixture(&thumb).is_err() {
            eprintln!("[SKIP] fixture build failed");
            return;
        }

        let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
        let stage = ThumbnailStage::new(ffmpeg.clone());

        let config = PostProcess {
            embed_thumbnail: true,
            ..PostProcess::default()
        };
        let mut msg = make_msg(vec![media], config);
        msg.original_stem = format!("media_{format}");

        let result = stage.process(msg).await.expect("non-fatal stage");
        assert!(
            result.warnings.is_empty(),
            "{format} under mkv should embed with no warnings, got: {:?}",
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
            .expect("second stream (thumbnail attachment) must be present");
        assert_eq!(thumb_stream.codec_type, "video");
        assert_eq!(
            thumb_stream.codec_name.as_deref(),
            Some(expected_codec),
            "{format} must attach natively (own codec), not be transcoded"
        );
    }
}

/// #530 regression guard: a BMP thumbnail is NOT recognized by `FFmpeg`'s
/// Matroska read-back mimetype table and must be normalized to mjpeg first
/// — otherwise the previous catch-all mislabeled it `image/jpeg` (wrong
/// codec, corrupt attachment) and a fixed-but-unnormalized path would still
/// attach it invisibly.
#[tokio::test]
async fn process_normalizes_bmp_thumbnail_into_mkv_as_mjpeg() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.mkv");
    let thumb = dir.path().join("video.bmp");
    let Ok(built) = build_mkv_fixture(dir.path()) else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    std::fs::rename(&built, &media).expect("rename mkv fixture");
    if build_still_image_fixture(&thumb).is_err() {
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
        "mkv bmp normalization should succeed with no warnings, got: {:?}",
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
        .expect("second stream (thumbnail attachment) must be present");
    assert_eq!(thumb_stream.codec_type, "video");
    assert_eq!(
        thumb_stream.codec_name.as_deref(),
        Some("mjpeg"),
        "bmp under mkv must be normalized to mjpeg — its mimetype is not \
         recognized by FFmpeg's Matroska read-back and would attach \
         invisibly (#530)"
    );

    // A `codec_name` of "mjpeg" is necessary but NOT sufficient: the
    // pre-#530 catch-all also declared a `mjpeg`-labeled attachment for
    // formats it never transcoded (it just lied about the mimetype), and
    // `matroskadec.c` assigns the read-back codec from the mimetype STRING
    // alone — so a mislabeled attachment reports the same codec_name as a
    // genuinely transcoded one. Decoding the actual frame is what tells them
    // apart: real transcoded mjpeg bytes decode cleanly; raw BMP bytes
    // wearing an mjpeg label fail to decode ("No JPEG data found").
    let decode = Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(["-i", out_path.to_str().unwrap()])
        .args(["-map", "0:1", "-frames:v", "1", "-f", "null", "-"])
        .output()
        .expect("spawn ffmpeg decode check");
    assert!(
        decode.status.success() && decode.stderr.is_empty(),
        "the normalized bmp-as-mjpeg attachment must decode cleanly as a real \
         mjpeg frame, not just carry the label: stderr={}",
        String::from_utf8_lossy(&decode.stderr)
    );
}
