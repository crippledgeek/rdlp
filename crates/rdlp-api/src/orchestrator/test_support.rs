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
use rdlp_core::{ExtractionContext, InfoExtractor};
use rdlp_extractor::ExtractorRegistryTrait;
use rdlp_types::{Codec, Config, DownloadProtocol, Format, InfoDict};
use regex::Regex;
use std::sync::Arc;
use std::sync::LazyLock;
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

/// Matches any URL — `FakeRegistry` has exactly one extractor, so routing
/// never needs to discriminate.
static MATCH_ANY_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r".").expect("static pattern is valid"));

/// An [`InfoExtractor`] that ignores its input and hands back a fixed
/// `InfoDict`, so a test can drive any of the orchestrator's three
/// extraction boundaries (`extract_video`, `extract_lazy_formats`,
/// `extract_playlist` — see `Orchestrator::finish_extracted_formats` in
/// `extraction.rs`), as `tests/hls_expansion_guarantee.rs` does, without a
/// real site.
pub(super) struct FakeExtractor {
    pub(super) info: InfoDict,
}

#[async_trait::async_trait]
impl InfoExtractor for FakeExtractor {
    fn name(&self) -> &'static str {
        "FakeExtractor"
    }

    fn valid_url(&self) -> &Regex {
        &MATCH_ANY_URL
    }

    async fn extract(&self, _url: &str, _ctx: &ExtractionContext) -> rdlp_core::Result<InfoDict> {
        Ok(self.info.clone())
    }
}

/// An [`ExtractorRegistryTrait`] that always routes to its one
/// [`FakeExtractor`], regardless of URL.
pub(super) struct FakeRegistry {
    pub(super) extractor: Arc<dyn InfoExtractor>,
}

impl ExtractorRegistryTrait for FakeRegistry {
    fn find_extractor(&self, _url: &str) -> Option<Arc<dyn InfoExtractor>> {
        Some(Arc::clone(&self.extractor))
    }

    fn list_extractors(&self) -> Vec<&str> {
        vec!["FakeExtractor"]
    }
}

/// A row this fake extractor hands back can be seeded at an address the SSRF
/// gate is supposed to reject before any fetch is attempted (see
/// `rdlp_extractor::hls::expand::tests::seed_link_local_metadata_address_rejected`
/// for the gate's own guarantee). If a caller-side regression ever let such a
/// row reach a real fetch anyway, this bounds how long a test can hang
/// waiting for it, rather than the default 30s connect timeout a real
/// download needs.
const HOSTILE_FETCH_BOUND_SECS: u64 = 2;

/// An orchestrator whose extractor is `info` regardless of the URL passed to
/// `extract_video`/`extract_lazy_formats`/`extract_playlist`, and whose HTTP
/// client times out quickly — so a test exercising any of the three
/// `finish_extracted_formats` boundaries against mockito, alongside a
/// deliberately unresolvable URL, fails fast instead of hanging on a real
/// network round trip if that URL is ever mistakenly reached.
pub(super) fn orchestrator_with_fake_extractor(info: InfoDict) -> Orchestrator {
    let config = Config {
        socket_timeout: Some(HOSTILE_FETCH_BOUND_SECS),
        read_timeout: Some(HOSTILE_FETCH_BOUND_SECS),
        ..Config::default()
    };
    let mut orchestrator = orchestrator_with_config(config);
    orchestrator.extractor_registry = Arc::new(FakeRegistry {
        extractor: Arc::new(FakeExtractor { info }),
    });
    orchestrator
}
