//! #577 at the pipeline level: every stage that takes a caller-chosen target
//! container refuses an audio-only one for a video-bearing source, and no
//! refusal costs the user their completed download.
//!
//! The unit suite in `rdlp-ffmpeg` proves the guard itself. These two
//! properties are only observable from here:
//!
//! - **Every entry point.** `RemuxStage` was guarded first, but `RecodeStage`
//!   routes to a full *transcode* when the codec cannot be stream-copied
//!   (`--recode-video=wma` on vp9 took that branch and would have encoded
//!   video into a `.wma`), and `MergeStage` calls a separate `FFmpegRunner`
//!   implementation that never reaches `remux_sync` at all.
//! - **The download survives.** All three are fatal stages, so the error drops
//!   the `PipelineMessage` and `FileTracker`'s cancel-`Drop` would otherwise
//!   delete `current_files` — i.e. the media that just finished downloading,
//!   over a flag the operator could have fixed and re-run.
//!
//! Self-skips when the system `ffmpeg` CLI is absent (fixtures only).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, MergeStage, PostProcess, RecodeStage, RemuxStage};
use rdlp_types::ContainerFormat;

mod common;
use common::{
    FIXTURE_FAILED, MsgOptions, build_av_fixture, build_video_fixture, ffmpeg_cli_available,
    make_msg,
};

/// Assert an error names both the container the user asked for and the one
/// the refusal points at — the two halves that make the message actionable.
fn assert_names_wma_and_wmv(err: &anyhow::Error, label: &str) {
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("wma") && msg.contains("wmv"),
        "{label}: refusal must name the asked-for container and the alternative, got: {msg}"
    );
}

/// `--remux=wma` on a video source: refused, and the download survives the
/// stage's fatal error.
///
/// The preservation half fails against a guard that refuses without telling
/// the tracker: the media is deleted by `Drop` the instant the message goes
/// out of scope.
#[tokio::test]
async fn remux_refuses_and_keeps_the_download() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.rdlp-tmp-577.mp4");
    build_av_fixture(&media).expect(FIXTURE_FAILED);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = RemuxStage::new(ffmpeg);
    let config = PostProcess {
        remux_container: Some(ContainerFormat::Wma),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    // `let Err(..) else` rather than `expect_err`: `PipelineMessage` is not
    // `Debug`, and this shape never binds the Ok value.
    let Err(err) = stage.process(msg).await else {
        panic!("RemuxStage must refuse an audio-only target for a video source");
    };
    assert_names_wma_and_wmv(&err, "remux");

    assert!(
        media.exists(),
        "the completed download must survive a policy refusal — it is intact, \
         untouched, and the operator only needs to re-run with a different flag"
    );
}

/// `--recode-video=wma`: the transcode branch, and the reason the guard could
/// not live in `remux_sync` alone. `RecodeStage` stream-copies when it can and
/// otherwise *encodes*; h264 into ASF is remuxable, so this uses a **vp9**
/// source, which is not — exactly the case that used to encode video into a
/// `.wma` without ever touching the remux path.
#[tokio::test]
async fn recode_refuses_the_transcode_branch_and_keeps_the_download() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    if rdlp_ffmpeg::ffmpeg::video_codecs::resolve_encoder("vp9").is_none() {
        eprintln!("[SKIP] this FFmpeg build has no vp9 encoder");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.rdlp-tmp-577.webm");
    build_vp9_fixture(&media);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = RecodeStage::new(ffmpeg);
    let config = PostProcess {
        recode_video: Some(ContainerFormat::Wma),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media.clone()], config, MsgOptions::default());

    let Err(err) = stage.process(msg).await else {
        panic!("RecodeStage must refuse an audio-only target before transcoding");
    };
    assert_names_wma_and_wmv(&err, "recode");

    assert!(
        media.exists(),
        "a refused recode must not cost the user their download"
    );
}

/// `postprocess.merge_output_format = wma`: `MergeStage` calls
/// `FFmpegRunner::merge`, a separate mux implementation that never reaches
/// `remux_sync`, so it needs — and now has — the same guard.
#[tokio::test]
async fn merge_refuses_an_audio_only_output_format_and_keeps_the_inputs() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("video.rdlp-tmp-577.mp4");
    let audio = dir.path().join("audio.rdlp-tmp-577.m4a");
    build_video_fixture(&video, "mp4").expect(FIXTURE_FAILED);
    build_audio_fixture(&audio);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = MergeStage::new(ffmpeg);
    let config = PostProcess {
        merge_output_format: Some(ContainerFormat::Wma),
        ..PostProcess::default()
    };
    let msg = make_msg(
        vec![video.clone(), audio.clone()],
        config,
        MsgOptions::default(),
    );

    let Err(err) = stage.process(msg).await else {
        panic!("MergeStage must refuse an audio-only merge_output_format");
    };
    assert_names_wma_and_wmv(&err, "merge");

    assert!(
        video.exists() && audio.exists(),
        "a refused merge must not cost the user either downloaded stream"
    );
}

/// Control: an audio-only *source* is what `.wma` is for. `RemuxStage` must
/// carry it through untouched, and the run must not be turned into a refusal
/// by a guard that looks only at the target container.
#[tokio::test]
async fn remux_of_an_audio_only_source_into_wma_still_succeeds() {
    if !ffmpeg_cli_available() {
        eprintln!("[SKIP] ffmpeg CLI not available");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("audio.rdlp-tmp-577.m4a");
    build_audio_fixture(&media);

    let ffmpeg = Arc::new(FFmpegRunner::new().expect("FFmpeg required"));
    let stage = RemuxStage::new(ffmpeg);
    let config = PostProcess {
        remux_container: Some(ContainerFormat::Wma),
        ..PostProcess::default()
    };
    let msg = make_msg(vec![media], config, MsgOptions::default());

    let result = stage
        .process(msg)
        .await
        .expect("an audio-only source into .wma is ordinary work");
    assert_eq!(
        result.tracker.primary().extension().unwrap(),
        "wma",
        "the remux must have produced the .wma the user asked for"
    );
}

/// A vp9 source: `RecodeStage`'s ASF rule lists no vp9, so this cannot be
/// stream-copied and takes the transcode branch.
fn build_vp9_fixture(path: &std::path::Path) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "testsrc=d=1:s=160x120:r=10"])
        .args(["-c:v", "libvpx-vp9", "-pix_fmt", "yuv420p", "-b:v", "50k"])
        .arg(path)
        .status()
        .expect("failed to spawn ffmpeg");
    assert!(status.success(), "{FIXTURE_FAILED} (vp9)");
}

/// An audio-only AAC source — no video stream of any kind.
fn build_audio_fixture(path: &std::path::Path) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "sine=d=1"])
        .args(["-c:a", "aac", "-b:a", "64k"])
        .arg(path)
        .status()
        .expect("failed to spawn ffmpeg");
    assert!(status.success(), "{FIXTURE_FAILED} (audio-only)");
}
