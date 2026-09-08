//! Shared fixtures for the orchestrator's tests.
//!
//! `selection/tests.rs` and `tests/abi_mismatch_tests.rs` had grown their own
//! copies of the same orchestrator builder and the same four `Format` shapes.
//! Copies of a fixture drift exactly like copies of production code, and a
//! drifted fixture is worse: the test still passes, against a slightly
//! different world than the one it claims to describe.

use crate::events::Event;
use crate::handle::DownloadId;
use crate::orchestrator::Orchestrator;
use crate::orchestrator::pipeline_availability::PipelineAvailability;
use rdlp_types::{Codec, Config, DownloadProtocol, Format, InfoDict};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// An orchestrator with the given config, and whatever `FFmpeg` the machine
/// running the tests happens to have.
pub(super) fn orchestrator_with_config(config: Config) -> Orchestrator {
    let (tx, _rx) = mpsc::channel::<Event>(64);
    Orchestrator::new(
        Arc::new(config),
        tx,
        DownloadId::next(),
        CancellationToken::new(),
        None,
    )
}

/// An orchestrator whose post-processing pipeline is in a chosen state,
/// independent of the machine's real `FFmpeg`.
pub(super) fn orchestrator_with(config: Config, pipeline: PipelineAvailability) -> Orchestrator {
    let mut orchestrator = orchestrator_with_config(config);
    orchestrator.pipeline = pipeline;
    orchestrator
}

/// A combined video+audio format — needs nothing from `FFmpeg`.
pub(super) fn make_combined(id: &str, height: u32, quality: i32) -> Format {
    let mut f = Format::new(id, format!("url_{id}"), "mp4", DownloadProtocol::Https);
    f.vcodec = Codec::from("h264".to_string());
    f.acodec = Codec::from("aac".to_string());
    f.height = Some(height);
    f.quality = Some(quality);
    f.tbr = Some(f64::from(height) * 2.0);
    f
}

/// The same, delivered over HLS — the pipeline remuxes it regardless of config.
pub(super) fn make_hls(id: &str, height: u32) -> Format {
    let mut f = Format::new(
        id,
        format!("url_{id}.m3u8"),
        "mp4",
        DownloadProtocol::M3u8Native,
    );
    f.vcodec = Codec::from("h264".to_string());
    f.acodec = Codec::from("aac".to_string());
    f.height = Some(height);
    f
}

pub(super) fn make_video_only(id: &str, height: u32) -> Format {
    let mut f = Format::new(id, format!("url_{id}"), "mp4", DownloadProtocol::Https);
    f.vcodec = Codec::from("h264".to_string());
    f.acodec = Codec::Absent;
    f.height = Some(height);
    f.vbr = Some(f64::from(height) * 1.5);
    f
}

pub(super) fn make_audio_only(id: &str, abr: f64) -> Format {
    let mut f = Format::new(id, format!("url_{id}"), "m4a", DownloadProtocol::Https);
    f.vcodec = Codec::Absent;
    f.acodec = Codec::from("aac".to_string());
    f.abr = Some(abr);
    f
}

pub(super) fn test_info_with_formats(formats: Vec<Format>) -> InfoDict {
    let mut info = InfoDict::new(
        "test_id",
        "Test Video",
        "TestExtractor",
        "https://example.com/video",
    );
    info.formats = formats;
    info
}
