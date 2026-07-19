//! #549 follow-up: only `jpeg`/`png` embed natively as mp3 `ID3v2` `APIC`
//! covers; `gif`, `tiff`, `bmp` and `webp` are all normalized to mjpeg first.
//!
//! `container_accepts_image_codec`'s `> 0` fix means mp3's `query_codec`
//! callback reports all six as representable (it answers via the shared
//! `APIC` tag rather than a bare `1`). But that answers only "can the muxer
//! store these bytes", not "will an `ID3v2` reader display them as a cover" —
//! the distinction #530 established for Matroska.
//!
//! ID3v2.3 §4.15 / ID3v2.4 §4.14 (id3.org): *"The 'image/png' or
//! 'image/jpeg' picture format should be used when interoperability is
//! wanted."* Advisory, but every maintained reader converges on it: `TagLib`
//! restates it verbatim, `mutagen`/`jaudiotagger` expose only JPEG/PNG MIME
//! constants, and `Mp3tag`'s "Adjust Cover" offers only Original/JPEG/PNG.
//!
//! An earlier revision passed `gif`/`tiff` through, inheriting the tier from
//! `FFmpeg`'s `matroskadec.c` `mkv_image_mime_tags[]`. That is a Matroska
//! *decoder* table for turning attachments into `attached_pic` streams; the
//! mp3 muxer has none, so the carve-out does not transfer and no surveyed
//! reader treats gif/tiff as safer than bmp/webp.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (used only to build the
//! image/mp3 fixtures).
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

fn build_mp3_fixture(path: &Path) -> Result<(), ()> {
    let ok = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=d=1",
            "-c:a",
            "libmp3lame",
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
        original_stem: "audio".to_string(),
        is_hls: false,
        verbose: false,
        callback_factory: None,
        error_tx: Some(error_tx),
        warnings: Vec::new(),
        encoding_tool: None,
        cancel: CancellationToken::new(),
    }
}

async fn thumbnail_codec_in_mp3(format: &str) -> Option<String> {
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("audio.mp3");
    let thumb = dir.path().join(format!("audio.{format}"));
    if build_mp3_fixture(&media).is_err() || build_still_image_fixture(&thumb).is_err() {
        return None;
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
        "{format} thumbnail embed into mp3 should succeed with no warnings, got: {:?}",
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
        .find(|s| s.codec_type == "video")
        .expect("attached-pic video stream must be present");
    thumb_stream.codec_name.clone()
}

/// #530-mirrored regression guard: bmp must NOT pass through natively into
/// mp3 even though `container_accepts_image_codec`'s `> 0` fix now reports
/// mp3's `query_codec` as accepting it — it must be normalized to mjpeg.
/// Fails against a version of `normalize_thumbnail_for_embed` with no mp3
/// carve-out (bmp would stream-copy unchanged, `codec_name == "bmp"`).
#[tokio::test]
async fn process_normalizes_bmp_thumbnail_into_mp3_as_mjpeg() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let Some(codec) = thumbnail_codec_in_mp3("bmp").await else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    assert_eq!(
        codec, "mjpeg",
        "bmp under mp3 must be normalized to mjpeg — no evidence an ID3v2 \
         reader renders a raw bmp APIC frame (mirrors #530's Matroska policy)"
    );
}

/// Same guard for webp.
#[tokio::test]
async fn process_normalizes_webp_thumbnail_into_mp3_as_mjpeg() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let Some(codec) = thumbnail_codec_in_mp3("webp").await else {
        eprintln!("[SKIP] fixture build failed");
        return;
    };
    assert_eq!(
        codec, "mjpeg",
        "webp under mp3 must be normalized to mjpeg — no evidence an ID3v2 \
         reader renders a raw webp APIC frame (mirrors #530's Matroska policy)"
    );
}

/// gif and tiff are normalized too. They were briefly passed through on the
/// strength of `FFmpeg`'s `matroskadec.c` `mkv_image_mime_tags[]` table
/// (#530), but that is a Matroska *decoder* table for resolving attachments
/// into `attached_pic` streams; the mp3 muxer has no equivalent, so the
/// carve-out does not transfer to `ID3v2` `APIC`. No surveyed reader
/// (`TagLib`, `mutagen`, `jaudiotagger`, `Mp3tag`) treats gif/tiff as safer
/// than bmp/webp — all four sit outside the spec's recommended set.
#[tokio::test]
async fn process_normalizes_gif_and_tiff_thumbnails_into_mp3_as_mjpeg() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    for format in ["gif", "tiff"] {
        let Some(codec) = thumbnail_codec_in_mp3(format).await else {
            eprintln!("[SKIP] fixture build failed for {format}");
            continue;
        };
        assert_eq!(
            codec, "mjpeg",
            "{format} under mp3 must be normalized to mjpeg — ID3v2.3 §4.15 / \
             ID3v2.4 §4.14 recommend image/png or image/jpeg for \
             interoperability, and no surveyed ID3v2 reader treats {format} as \
             a safer tier than bmp/webp"
        );
    }
}

/// Positive: the two formats the `ID3v2` spec actually recommends must pass
/// through untouched — the policy must not over-normalize those.
#[tokio::test]
async fn process_embeds_jpeg_and_png_thumbnails_into_mp3_without_normalizing() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    for (format, expected_codec) in [("jpg", "mjpeg"), ("png", "png")] {
        let Some(codec) = thumbnail_codec_in_mp3(format).await else {
            eprintln!("[SKIP] fixture build failed for {format}");
            continue;
        };
        assert_eq!(
            codec, expected_codec,
            "{format} is spec-recommended for APIC and must embed natively, \
             not be transcoded"
        );
    }
}
