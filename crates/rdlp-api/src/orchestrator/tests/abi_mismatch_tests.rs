//! What an ABI-skewed `FFmpeg` does to a run (rdlp#727).
//!
//! The defect these pin: both ways of having no pipeline were reported as
//! "`FFmpeg` NOT found" and both returned the files unchanged, so a download
//! whose remux never happened looked like a success.
//!
//! Two rules, and the tests below are organised by them:
//!
//! * A run that only `FFmpeg` could complete is refused **before the
//!   download**, where refusing costs nothing.
//! * A run that already holds bytes is never failed for this, because the
//!   caller finalizes the clean name only on the `Ok` path — failing would
//!   abandon a complete download under a `.rdlp-tmp-` name that
//!   `cleanup_stale` deletes an hour later.

use super::*;
use crate::orchestrator::DownloadPlan;
use crate::orchestrator::errors::OrchestratorError;
use crate::orchestrator::gated_plan::GatedPlan;
use crate::orchestrator::pipeline_availability::PipelineAvailability;
use crate::orchestrator::pipeline_availability::fixture::{
    A_PREFIX, COMPILED_MAJOR, LINKED_MAJOR, mismatches,
};
use crate::orchestrator::test_support::{
    make_audio_only, make_combined, make_hls, make_video_only, orchestrator_with,
    test_info_with_formats,
};
use rdlp_types::{ContainerFormat, PostProcess};

/// An orchestrator whose `FFmpeg` is present but ABI-skewed.
fn skewed_orchestrator(config: Config) -> Orchestrator {
    orchestrator_with(config, PipelineAvailability::AbiMismatch(mismatches()))
}

/// The metadata a post-process run carries; none of these tests read it.
fn test_info() -> InfoDict {
    test_info_with_formats(Vec::new())
}

fn one_file() -> Vec<PathBuf> {
    vec![PathBuf::from("/tmp/rdlp-test-video.mkv")]
}

// ── Rule 1: refused before the download, where refusing costs nothing ────────

#[tokio::test]
async fn a_merge_plan_is_refused_before_anything_is_downloaded() {
    // The case a config-shaped predicate missed entirely: nothing in the config
    // asks for post-processing, but only FFmpeg can join two streams. Letting
    // this through produced a silently audio-less video under the clean name.
    let orch = skewed_orchestrator(Config::default());

    let result = orch
        .select_format(
            &test_info_with_formats(vec![
                make_video_only("v1080", 1080),
                make_audio_only("a256", 256.0),
            ]),
            false,
        )
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "a merge needs FFmpeg to exist at all — got {result:?}"
    );
}

#[tokio::test]
async fn an_hls_plan_is_refused_before_anything_is_downloaded() {
    let orch = skewed_orchestrator(Config::default());

    let result = orch
        .select_format(
            &test_info_with_formats(vec![make_hls("hls720", 720)]),
            false,
        )
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "HLS segments may be unplayable until remuxed — got {result:?}"
    );
}

#[tokio::test]
async fn every_media_altering_option_is_refused_before_the_download() {
    // One disjunct per branch of `postprocessing_alters_the_media`; each would
    // otherwise be an untested arm.
    /// One config mutation, named so a failure says which branch broke.
    type Case = (&'static str, fn(&mut PostProcess));

    let cases: [Case; 5] = [
        ("extract_audio", |pp| pp.extract_audio = true),
        ("recode_video", |pp| {
            pp.recode_video = Some(ContainerFormat::Mkv);
        }),
        ("recode_container", |pp| {
            pp.recode_container = Some(ContainerFormat::Mkv);
        }),
        ("remux_container", |pp| {
            pp.remux_container = Some(ContainerFormat::Mp4);
        }),
        ("normalize_audio", |pp| pp.normalize_audio = true),
    ];

    for (name, set) in cases {
        let mut config = Config::default();
        set(&mut config.postprocess);
        let orch = skewed_orchestrator(config);

        let result = orch
            .select_format(
                &test_info_with_formats(vec![make_combined("c720", 720, 2)]),
                false,
            )
            .await;

        assert!(
            matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
            "{name} changes the media, so an unusable FFmpeg must refuse it — got {result:?}"
        );
    }
}

#[tokio::test]
async fn a_plain_progressive_download_is_not_refused() {
    // The boundary. `embed_thumbnail` and `fixup` are on in `Config::default()`,
    // so a request-shaped predicate would refuse this and thereby refuse every
    // unconfigured download on a skewed machine.
    let config = Config::default();
    assert!(
        config.postprocess.embed_thumbnail,
        "this test is only meaningful while embed_thumbnail defaults on"
    );
    assert_ne!(
        config.postprocess.fixup,
        rdlp_types::FixupPolicy::Never,
        "...and while fixup defaults to something other than Never"
    );
    let orch = skewed_orchestrator(config);

    let plan = orch
        .select_format(
            &test_info_with_formats(vec![make_combined("c720", 720, 2)]),
            false,
        )
        .await
        .expect("nothing here needs FFmpeg");

    assert!(matches!(plan, Some(DownloadPlan::Single(_))));
}

#[tokio::test]
async fn a_missing_ffmpeg_refuses_nothing() {
    // Mutation guard: the refusal must be reached via the mismatch, not by the
    // pipeline merely being absent — `FfmpegUnavailable` degrades as always.
    let orch = orchestrator_with(
        Config {
            postprocess: PostProcess {
                remux_container: Some(ContainerFormat::Mp4),
                ..Default::default()
            },
            ..Default::default()
        },
        PipelineAvailability::FfmpegUnavailable,
    );

    let plan = orch
        .select_format(
            &test_info_with_formats(vec![make_combined("c720", 720, 2)]),
            false,
        )
        .await
        .expect("a missing FFmpeg degrades, it does not refuse");

    assert!(plan.is_some());
}

#[test]
fn the_plan_answers_for_its_own_shape() {
    assert!(
        DownloadPlan::Merge {
            video: make_video_only("v1080", 1080),
            audio: make_audio_only("a256", 256.0),
        }
        .requires_ffmpeg(),
        "two streams need joining"
    );
    assert!(
        DownloadPlan::Single(make_hls("hls720", 720)).requires_ffmpeg(),
        "HLS needs remuxing"
    );
    assert!(
        !DownloadPlan::Single(make_combined("c720", 720, 2)).requires_ffmpeg(),
        "a progressive file is already what was asked for"
    );
}

#[test]
fn a_resumed_plan_cannot_reach_the_download_ungated() {
    // `select_format` is not the only producer of a plan: resuming a saved
    // session reconstructs one directly. That path once skipped the check, and
    // a resumed merge downloaded both streams while only the video was
    // finalized. `GatedPlan` is why it cannot skip it again — this constructor
    // is the only way to fill `Preparing`'s plan field, and it checks.
    let orch = skewed_orchestrator(Config::default());
    let merge = DownloadPlan::Merge {
        video: make_video_only("v1080", 1080),
        audio: make_audio_only("a256", 256.0),
    };

    let result = GatedPlan::new(&orch, Box::new(merge));

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "a resumed merge must be refused exactly like a freshly selected one"
    );
}

#[test]
fn the_gate_lets_an_unaffected_plan_through() {
    let orch = skewed_orchestrator(Config::default());
    let single = DownloadPlan::Single(make_combined("c720", 720, 2));

    assert!(
        GatedPlan::new(&orch, Box::new(single)).is_ok(),
        "a progressive download needs nothing from FFmpeg"
    );
}

// ── Rule 2: a run holding bytes is never failed for this ─────────────────────

#[tokio::test]
async fn a_run_holding_downloaded_bytes_is_never_failed() {
    // The bytes are at a `.rdlp-tmp-` seam and only the caller's `Ok` path
    // finalizes them, so returning `Err` here abandons a complete download to
    // `cleanup_stale`. Refusal belongs before the download; here it warns.
    let mut config = Config::default();
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    let orch = skewed_orchestrator(config);
    let files = one_file();

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), false, false)
        .await
        .expect("failing here would strand the download it was protecting");

    assert_eq!(result, files, "the downloaded bytes are handed back intact");
}

#[tokio::test]
async fn a_merge_that_slips_through_hands_back_both_streams() {
    // Honest about what this does and does not guarantee: the FUNCTION returns
    // both streams, but the caller finalizes only the first, so a run reaching
    // here would still produce a video without its audio. That is why the path
    // is closed by `GatedPlan` before the download rather than repaired here,
    // and why reaching it now logs a BUG line. This pins the function's half.
    let orch = skewed_orchestrator(Config::default());
    let files = vec![
        PathBuf::from("/tmp/rdlp-test.rdlp-tmp-abc.video.mkv"),
        PathBuf::from("/tmp/rdlp-test.rdlp-tmp-abc.audio.m4a"),
    ];

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), true, false)
        .await
        .expect("both streams must survive");

    assert_eq!(result, files, "neither stream may be dropped");
}

#[tokio::test]
async fn a_borrowed_input_fails_honestly() {
    // `process_local_file` borrows the user's own file and never deletes it, so
    // there is nothing of ours to strand and the run can fail properly.
    let mut config = Config::default();
    config.postprocess.recode_video = Some(ContainerFormat::Mkv);
    let orch = skewed_orchestrator(config);

    let result = orch
        .run_postprocessing(&test_info(), one_file(), false, true)
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "a recode of a user-supplied file cannot silently not happen — got {result:?}"
    );
}

#[tokio::test]
async fn a_missing_ffmpeg_still_degrades_gracefully() {
    // The behaviour rdlp has always had, and the one this change must not
    // convert into a failure: no FFmpeg at all is a degraded mode.
    let orch = orchestrator_with(
        Config {
            postprocess: PostProcess {
                remux_container: Some(ContainerFormat::Mp4),
                ..Default::default()
            },
            ..Default::default()
        },
        PipelineAvailability::FfmpegUnavailable,
    );
    let files = one_file();

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), false, false)
        .await
        .expect("a missing FFmpeg degrades, it does not fail the download");

    assert_eq!(result, files);
}

#[tokio::test]
async fn stdout_mode_is_not_refused() {
    // Stdout mode never post-processes, so an unusable FFmpeg costs it nothing
    // and refusing would block a download that would have worked.
    let orch = skewed_orchestrator(Config {
        output_to_stdout: true,
        ..Default::default()
    });

    let plan = orch
        .select_format(
            &test_info_with_formats(vec![make_hls("hls720", 720)]),
            false,
        )
        .await
        .expect("stdout mode asks nothing of FFmpeg");

    assert!(plan.is_some());
}

// ── The remedy survives to every surface that shows it ───────────────────────

#[tokio::test]
async fn the_refusal_carries_the_remedy_intact() {
    let mut config = Config::default();
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    let orch = skewed_orchestrator(config);

    let err = orch
        .select_format(
            &test_info_with_formats(vec![make_combined("c720", 720, 2)]),
            false,
        )
        .await
        .expect_err("a requested remux against a skewed FFmpeg must be refused");
    let message = err.to_string();

    for expected in ["regenerate the bindings", "LD_LIBRARY_PATH", A_PREFIX] {
        assert!(
            message.contains(expected),
            "the remedy must reach the user whole; {expected:?} missing from: {message}"
        );
    }
    assert!(
        message.contains(&COMPILED_MAJOR.to_string())
            && message.contains(&LINKED_MAJOR.to_string()),
        "both versions name the problem; neither may be dropped: {message}"
    );
    assert!(
        !message.contains("NOT found"),
        "an installed FFmpeg must not be reported as missing — rdlp#727: {message}"
    );
}

#[test]
fn the_api_boundary_keeps_the_remedy() {
    let api_error =
        crate::errors::RdlpApiError::from(OrchestratorError::FFmpegAbiMismatch(mismatches()));

    let message = api_error.to_string();
    assert!(
        message.contains("regenerate the bindings"),
        "the desktop and CLI see this error through RdlpApiError; summarising \
         it there loses the remedy just as thoroughly: {message}"
    );
}
