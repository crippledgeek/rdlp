//! `SubtitleStage` must never delete a user-owned subtitle sidecar.
//!
//! Same defect class as the thumbnail sidecar bug (see
//! `thumbnail_borrowed_sidecar.rs`) in a second stage, surfaced by the
//! pre-push security review of that fix: `SubtitleStage::find_subtitle_files`
//! discovers `{stem}.{lang}.srt` beside the media file and, when
//! `--write-subtitles` was not requested, marks every one temp — with no
//! ownership gate at all.
//!
//! On the borrowed path (`rdlp /home/me/clip.mp4 --embed-subtitles`) those are
//! the user's own subtitle files. Subtitle *embedding* is not implemented yet,
//! so the stage's only real effect there was deleting them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, PostProcess, SubtitleStage};

mod common;
use common::{FIXTURE_FAILED, MsgOptions, build_video_fixture, ffmpeg_cli_available, make_msg};

fn opts(borrowing: bool) -> MsgOptions {
    MsgOptions {
        borrowing,
        original_stem: "myvideo",
        ..MsgOptions::default()
    }
}

/// The user's own `myvideo.en.srt` next to their own `myvideo.mp4` must
/// survive. Fails against the unpatched stage, which marks it temp
/// unconditionally.
#[tokio::test]
async fn subtitle_stage_never_deletes_a_user_owned_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let subs = dir.path().join("myvideo.en.srt");
    std::fs::write(&subs, b"1\n00:00:00,000 --> 00:00:01,000\nhello\n").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = SubtitleStage::new(ffmpeg);

    let config = PostProcess {
        embed_subtitles: true,
        write_subtitles: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, opts(true));

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        subs.exists(),
        "the user's own myvideo.en.srt must survive post-processing of their own local file"
    );
}

/// Regression guard for the DOWNLOAD path: an rdlp-fetched subtitle sidecar
/// must still be cleaned up without `--write-subtitles`. Dies if the fix is
/// over-broad ("never delete any subtitle sidecar").
#[tokio::test]
async fn downloaded_subtitle_sidecar_is_still_cleaned_up() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let subs = dir.path().join("myvideo.en.srt");
    std::fs::write(&subs, b"1\n00:00:00,000 --> 00:00:01,000\nhello\n").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = SubtitleStage::new(ffmpeg);

    let config = PostProcess {
        embed_subtitles: true,
        write_subtitles: false,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, opts(false));

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(
        !subs.exists(),
        "an rdlp-downloaded subtitle sidecar must still be cleaned up"
    );
}

/// `--write-subtitles` retains the user's sidecar too.
#[tokio::test]
async fn write_subtitles_retains_user_owned_sidecar() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("myvideo.mp4");
    build_video_fixture(&media, "mp4").expect(FIXTURE_FAILED);
    let subs = dir.path().join("myvideo.en.srt");
    std::fs::write(&subs, b"1\n00:00:00,000 --> 00:00:01,000\nhello\n").unwrap();

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = SubtitleStage::new(ffmpeg);

    let config = PostProcess {
        embed_subtitles: true,
        write_subtitles: true,
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, opts(true));

    let mut result = stage.process(msg).await.expect("non-fatal stage");
    result.tracker.cleanup();

    assert!(subs.exists(), "--write-subtitles must retain the sidecar");
}
