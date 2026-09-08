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
use crate::orchestrator::pipeline_availability::PipelineAvailability;
use crate::orchestrator::pipeline_availability::fixture::{
    A_PREFIX, COMPILED_MAJOR, LINKED_MAJOR, mismatches,
};
use rdlp_types::{Codec, ContainerFormat, DownloadProtocol, Format, PostProcess};

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

/// The metadata a post-process run carries; none of these tests read it.
fn test_info() -> InfoDict {
    InfoDict::new("id", "title", "test", "https://example.test/v")
}

fn one_file() -> Vec<PathBuf> {
    vec![PathBuf::from("/tmp/rdlp-test-video.mkv")]
}

/// A combined progressive format — needs nothing from `FFmpeg`.
fn progressive() -> Format {
    let mut f = Format::new(
        "c720",
        "https://example.test/v.mp4",
        "mp4",
        DownloadProtocol::Https,
    );
    f.vcodec = Codec::from("h264".to_string());
    f.acodec = Codec::from("aac".to_string());
    f.height = Some(720);
    f
}

/// The same, delivered over HLS — the pipeline remuxes it regardless of config.
fn hls() -> Format {
    let mut f = Format::new(
        "hls720",
        "https://example.test/v.m3u8",
        "mp4",
        DownloadProtocol::M3u8Native,
    );
    f.vcodec = Codec::from("h264".to_string());
    f.acodec = Codec::from("aac".to_string());
    f.height = Some(720);
    f
}

fn video_only() -> Format {
    let mut f = Format::new(
        "v1080",
        "https://example.test/v",
        "mp4",
        DownloadProtocol::Https,
    );
    f.vcodec = Codec::from("h264".to_string());
    f.acodec = Codec::Absent;
    f.height = Some(1080);
    f
}

fn audio_only() -> Format {
    let mut f = Format::new(
        "a256",
        "https://example.test/a",
        "m4a",
        DownloadProtocol::Https,
    );
    f.vcodec = Codec::Absent;
    f.acodec = Codec::from("aac".to_string());
    f.abr = Some(256.0);
    f
}

fn info_with(formats: Vec<Format>) -> InfoDict {
    let mut info = test_info();
    info.formats = formats;
    info
}

// ── Rule 1: refused before the download, where refusing costs nothing ────────

#[tokio::test]
async fn a_merge_plan_is_refused_before_anything_is_downloaded() {
    // The case a config-shaped predicate missed entirely: nothing in the config
    // asks for post-processing, but only FFmpeg can join two streams. Letting
    // this through produced a silently audio-less video under the clean name.
    let orch = skewed_orchestrator(Config::default());

    let result = orch
        .select_format(&info_with(vec![video_only(), audio_only()]), false)
        .await;

    assert!(
        matches!(result, Err(OrchestratorError::FFmpegAbiMismatch(_))),
        "a merge needs FFmpeg to exist at all — got {result:?}"
    );
}

#[tokio::test]
async fn an_hls_plan_is_refused_before_anything_is_downloaded() {
    let orch = skewed_orchestrator(Config::default());

    let result = orch.select_format(&info_with(vec![hls()]), false).await;

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
            .select_format(&info_with(vec![progressive()]), false)
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
        .select_format(&info_with(vec![progressive()]), false)
        .await
        .expect("nothing here needs FFmpeg");

    assert!(matches!(plan, Some(DownloadPlan::Single(_))));
}

#[tokio::test]
async fn a_working_ffmpeg_refuses_nothing() {
    // Mutation guard: the refusal must be reached via the mismatch, not by the
    // pipeline merely being absent — `FfmpegUnavailable` degrades as always.
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let mut config = Config::default();
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    let mut orch = Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    );
    orch.pipeline = PipelineAvailability::FfmpegUnavailable;

    let plan = orch
        .select_format(&info_with(vec![progressive()]), false)
        .await
        .expect("a missing FFmpeg degrades, it does not refuse");

    assert!(plan.is_some());
}

#[test]
fn the_plan_answers_for_its_own_shape() {
    assert!(
        DownloadPlan::Merge {
            video: video_only(),
            audio: audio_only(),
        }
        .requires_ffmpeg(),
        "two streams need joining"
    );
    assert!(
        DownloadPlan::Single(hls()).requires_ffmpeg(),
        "HLS needs remuxing"
    );
    assert!(
        !DownloadPlan::Single(progressive()).requires_ffmpeg(),
        "a progressive file is already what was asked for"
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
    // Belt and braces for the same rule: even the shape that motivated the
    // pre-download refusal must not lose a file if it ever reaches here.
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
    let (tx, _rx) = mpsc::channel::<Event>(64);
    let mut config = Config::default();
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    let mut orch = Orchestrator::new(
        Arc::new(config),
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

// ── The remedy survives to every surface that shows it ───────────────────────

#[tokio::test]
async fn the_refusal_carries_the_remedy_intact() {
    let mut config = Config::default();
    config.postprocess.remux_container = Some(ContainerFormat::Mp4);
    let orch = skewed_orchestrator(config);

    let err = orch
        .select_format(&info_with(vec![progressive()]), false)
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
