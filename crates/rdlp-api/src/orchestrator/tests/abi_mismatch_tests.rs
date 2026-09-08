//! What an ABI-skewed `FFmpeg` does to a run (rdlp#727).
//!
//! The defect these pin: both ways of having no pipeline were reported as
//! "`FFmpeg` NOT found" and both returned the files unchanged, so a download
//! whose remux never happened looked like a success. An installed-but-unusable
//! `FFmpeg` now fails the run it was needed for, and still leaves alone a run
//! that asked nothing of it.

use super::*;
use crate::orchestrator::errors::OrchestratorError;
use crate::orchestrator::pipeline_availability::PipelineAvailability;
use rdlp_ffmpeg::ffmpeg::abi::{
    AbiMismatch, AbiMismatches, AbiVersion, BuildPrefix, FfmpegLibrary, MismatchKind,
};
use rdlp_types::ContainerFormat;

/// `libavcodec`'s major in this build's bindings, and the next one up — the
/// drift rdlp#656 observed on a system whose `FFmpeg` moved.
const COMPILED_MAJOR: i64 = 62;
const LINKED_MAJOR: i64 = 63;
const A_MINOR: i64 = 11;

fn mismatches() -> AbiMismatches {
    AbiMismatches::new(
        vec![AbiMismatch {
            library: FfmpegLibrary::Avcodec,
            kind: MismatchKind::DifferentMajor,
            compiled: AbiVersion::new(COMPILED_MAJOR, A_MINOR),
            linked: AbiVersion::new(LINKED_MAJOR, A_MINOR),
        }],
        BuildPrefix::from_env_value("/home/user/.local/mediaforge"),
    )
    .expect("a non-empty mismatch list is a mismatch")
}

/// An orchestrator whose `FFmpeg` is present but ABI-skewed.
fn skewed_orchestrator(config: Config) -> Orchestrator {
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let mut orch = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    orch.pipeline = PipelineAvailability::AbiMismatch(mismatches());
    orch
}

/// A config that asks for post-processing.
fn remux_requested() -> Config {
    let mut config = Config::default();
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    config
}

/// The metadata a post-process run carries; none of these tests read it.
fn test_info() -> InfoDict {
    InfoDict::new("id", "title", "test", "https://example.test/v")
}

fn one_file() -> Vec<PathBuf> {
    vec![PathBuf::from("/tmp/rdlp-test-video.mkv")]
}

#[tokio::test]
async fn abi_mismatch_fails_a_run_that_needed_postprocessing() {
    let orch = skewed_orchestrator(remux_requested());

    let result = orch
        .run_postprocessing(&test_info(), one_file(), false, false)
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "a requested remux that cannot run must fail the run, not return the \
         unprocessed file as if it had succeeded"
    );
}

#[tokio::test]
async fn abi_mismatch_failure_carries_the_remedy_intact() {
    let orch = skewed_orchestrator(remux_requested());

    let err = orch
        .run_postprocessing(&test_info(), one_file(), false, false)
        .await
        .expect_err("a requested remux against a skewed FFmpeg must fail");
    let message = err.to_string();

    for expected in [
        "regenerate the bindings",
        "LD_LIBRARY_PATH",
        "/home/user/.local/mediaforge",
    ] {
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

#[tokio::test]
async fn abi_mismatch_fails_an_hls_run_that_asked_for_nothing_else() {
    // `is_hls` alone requires the pipeline (RemuxStage handles TS → MP4), so it
    // is the boundary between the two branches below.
    let orch = skewed_orchestrator(Config::default());

    let result = orch
        .run_postprocessing(&test_info(), one_file(), true, false)
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "an HLS download needs the remux stage, so a skewed FFmpeg must fail it"
    );
}

#[tokio::test]
async fn the_on_by_default_options_alone_do_not_fail_a_run() {
    // The boundary that decides how much this change costs on a skewed system.
    // `embed_thumbnail` and `fixup` are both on in `Config::default()`, so
    // treating either as a request would refuse every download rather than
    // every download whose media would come out wrong.
    let mut config = Config::default();
    config.postprocess.fixup = rdlp_types::FixupPolicy::DetectOrWarn;
    assert!(
        config.postprocess.embed_thumbnail,
        "this test is only meaningful while embed_thumbnail defaults on"
    );
    let orch = skewed_orchestrator(config);
    let files = one_file();

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), false, false)
        .await
        .expect("an unconfigured run must not be refused");

    assert_eq!(result, files);
}

#[tokio::test]
async fn a_missing_decoration_warns_where_wrong_media_fails() {
    // The other side of that boundary, set explicitly: subtitles and metadata
    // are things around the media, so their absence is a warning. Skipping the
    // remux in `abi_mismatch_fails_a_run_that_needed_postprocessing` is not.
    let mut config = Config::default();
    config.postprocess.embed_subtitles = true;
    config.postprocess.embed_metadata = true;
    let orch = skewed_orchestrator(config);
    let files = one_file();

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), false, false)
        .await
        .expect("a missing thumbnail or tag does not make the media wrong");

    assert_eq!(result, files);
}

#[tokio::test]
async fn abi_mismatch_leaves_a_run_that_needed_nothing_alone() {
    let orch = skewed_orchestrator(Config::default());
    let files = one_file();

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), false, false)
        .await
        .expect("a download that asked nothing of FFmpeg is complete and correct");

    assert_eq!(
        result, files,
        "nothing was requested, so nothing is missing — failing here would be gratuitous"
    );
}

#[tokio::test]
async fn a_missing_ffmpeg_still_degrades_gracefully() {
    // The behaviour rdlp has always had, and the one this change must not
    // convert into a failure: no FFmpeg at all is a degraded mode.
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let mut orch = Orchestrator::new(
        Arc::new(remux_requested()),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    orch.pipeline = PipelineAvailability::FfmpegUnavailable;
    let files = one_file();

    let result = orch
        .run_postprocessing(&test_info(), files.clone(), false, false)
        .await
        .expect("a missing FFmpeg degrades, it does not fail the download");

    assert_eq!(result, files);
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
