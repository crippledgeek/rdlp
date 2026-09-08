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
//! Fails closed when the system `ffmpeg` CLI is absent (fixtures only) —
//! see `require_ffmpeg` for why this suite does not take the local self-skip
//! convention.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::sync::Arc;

use tempfile::TempDir;

use rdlp_postprocess::pipeline::PipelineStage;
use rdlp_postprocess::{FFmpegRunner, MergeStage, PostProcess, RecodeStage, RemuxStage};
use rdlp_types::ContainerFormat;

mod common;
use common::{
    FIXTURE_FAILED, MsgOptions, build_audio_fixture, build_av_fixture, build_video_fixture,
    build_vp9_fixture, ffmpeg_cli_available, make_msg,
};

/// Require the `ffmpeg` CLI, and FAIL rather than skip when it is missing
/// unless `RDLP_ALLOW_SKIP_FFMPEG_TESTS` opts in — the same fail-closed policy
/// as `rdlp-ffmpeg/tests/audio_only_container_rejects_video.rs`, deliberately
/// adopted here against this crate's local self-skip convention.
///
/// The convention exists for suites whose property is a warning or a container
/// choice. This suite's property is that a refusal does not delete the user's
/// completed download; a machine that silently skips it reports green while
/// leaving the most expensive failure in the change unverified. Where the two
/// conventions disagree, the cost of the property being wrong decides.
fn require_ffmpeg() -> bool {
    if ffmpeg_cli_available() {
        return true;
    }
    if std::env::var_os("RDLP_ALLOW_SKIP_FFMPEG_TESTS").is_some() {
        eprintln!("[SKIP] ffmpeg CLI not available (RDLP_ALLOW_SKIP_FFMPEG_TESTS set)");
        return false;
    }
    panic!(
        "ffmpeg not found on PATH. This suite builds real fixtures to prove a policy \
         refusal never costs the user their download. Set RDLP_ALLOW_SKIP_FFMPEG_TESTS=1 \
         to explicitly opt into skipping it."
    );
}

/// The files `preserve_current_files` moved out of the `.rdlp-tmp-` namespace,
/// newest-name-sorted for stable assertions.
///
/// The stage consumes the `PipelineMessage` when it fails, so the survivor
/// cannot be read back from the tracker — the output directory is the only
/// observable, which is also exactly what the operator has. Asserting on
/// non-empty content, not mere existence: a zero-byte file would satisfy
/// `exists()` and be worthless.
fn kept_survivors(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut kept: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("read the output dir")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(".rdlp-kept-"))
        })
        .collect();
    for path in &kept {
        assert!(
            std::fs::metadata(path).is_ok_and(|m| m.len() > 0),
            "kept survivor {} is empty — preserved in name only",
            path.display()
        );
    }
    kept.sort();
    kept
}

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
    if !require_ffmpeg() {
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

    // The download must survive — moved out of the temp namespace so the stale
    // sweep cannot take it later, not left where it was.
    let kept = kept_survivors(dir.path());
    assert_eq!(
        kept.len(),
        1,
        "the completed download must survive a policy refusal — it is intact, \
         untouched, and the operator only needs to re-run with a different flag"
    );
    assert_eq!(
        kept[0].extension().and_then(|e| e.to_str()),
        Some("mp4"),
        "the survivor must be the source, extension intact: {kept:?}"
    );
    assert!(
        !media.exists(),
        "the temp-named original must be gone, not duplicated"
    );
}

/// `--recode-video=wma`: the transcode branch, and the reason the guard could
/// not live in `remux_sync` alone. `RecodeStage` stream-copies when it can and
/// otherwise *encodes*; h264 into ASF is remuxable, so this uses a **vp9**
/// source, which is not — exactly the case that used to encode video into a
/// `.wma` without ever touching the remux path.
#[tokio::test]
async fn recode_refuses_the_transcode_branch_and_keeps_the_download() {
    if !require_ffmpeg() {
        return;
    }
    if rdlp_ffmpeg::ffmpeg::video_codecs::resolve_encoder("vp9").is_none() {
        eprintln!("[SKIP] this FFmpeg build has no vp9 encoder");
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("video.rdlp-tmp-577.webm");
    build_vp9_fixture(&media).expect(FIXTURE_FAILED);

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

    let kept = kept_survivors(dir.path());
    assert_eq!(
        kept.len(),
        1,
        "a refused recode must not cost the user their download"
    );
    assert_eq!(
        kept[0].extension().and_then(|e| e.to_str()),
        Some("webm"),
        "the survivor must be the source, extension intact: {kept:?}"
    );
    assert!(
        !media.exists(),
        "the temp-named original must be gone, not duplicated"
    );
}

/// `postprocess.merge_output_format = wma`: `MergeStage` calls
/// `FFmpegRunner::merge`, a separate mux implementation that never reaches
/// `remux_sync`, so it needs — and now has — the same guard.
#[tokio::test]
async fn merge_refuses_an_audio_only_output_format_and_keeps_the_inputs() {
    if !require_ffmpeg() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let video = dir.path().join("video.rdlp-tmp-577.mp4");
    let audio = dir.path().join("audio.rdlp-tmp-577.m4a");
    build_video_fixture(&video, "mp4").expect(FIXTURE_FAILED);
    build_audio_fixture(&audio).expect(FIXTURE_FAILED);

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

    // Identity, not just a count: `len() == 2` does not say the two survivors
    // are the video and the audio, and a merge that kept one stream twice
    // would satisfy a bare count.
    let kept: Vec<String> = kept_survivors(dir.path())
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_owned))
        .collect();
    assert_eq!(kept.len(), 2, "kept survivors: {kept:?}");
    assert!(
        kept.iter().any(|n| n.starts_with("video.")),
        "the video stream must survive: {kept:?}"
    );
    assert!(
        kept.iter().any(|n| n.starts_with("audio.")),
        "the audio stream must survive: {kept:?}"
    );
    assert!(
        !video.exists() && !audio.exists(),
        "the temp-named originals must be gone, not duplicated"
    );
}

/// Control: an audio-only *source* is what `.wma` is for. `RemuxStage` must
/// carry it through untouched, and the run must not be turned into a refusal
/// by a guard that looks only at the target container.
#[tokio::test]
async fn remux_of_an_audio_only_source_into_wma_still_succeeds() {
    if !require_ffmpeg() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let media = dir.path().join("audio.rdlp-tmp-577.m4a");
    build_audio_fixture(&media).expect(FIXTURE_FAILED);

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
