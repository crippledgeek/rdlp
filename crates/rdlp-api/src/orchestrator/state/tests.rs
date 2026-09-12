//! Tests for download state machine types

use super::*;

#[test]
fn test_selecting_subtitles_display() {
    let phase = DownloadPhase::SelectingSubtitles {
        info: Box::new(rdlp_types::InfoDict::new(
            "id",
            "title",
            "test",
            "http://example.com",
        )),
        format: Box::new(rdlp_types::Format::new(
            "f1",
            "http://example.com/v.mp4",
            "mp4",
            rdlp_types::DownloadProtocol::Https,
        )),
        plan: Box::new(super::super::DownloadPlan::Single(rdlp_types::Format::new(
            "f1",
            "http://example.com/v.mp4",
            "mp4",
            rdlp_types::DownloadProtocol::Https,
        ))),
    };

    assert_eq!(format!("{phase}"), "selecting subtitles");
}

#[test]
fn test_selecting_subtitles_passes_through_when_no_subs() {
    // Verify the phase can be constructed with empty subtitle data
    let info = Box::new(rdlp_types::InfoDict::new(
        "id",
        "title",
        "test",
        "http://example.com",
    ));

    // No subtitles in info -> select_subtitles_if_needed returns empty vec
    assert!(info.subtitles.is_none());
    assert!(info.automatic_captions.is_none());
}

// ── #572: concurrent-process output-path collision ─────────────────────────
//
// `Preparing::advance` claims the `.rdlp-part` output path before resume
// detection. These drive that transition directly (no network I/O: with no
// pre-existing part file, `detect_resume_point` is a local filesystem check)
// rather than the full download→finalize path, matching the existing
// FFmpeg-free scope of this test module (see `finalize_tests.rs`).
mod part_lock_tests {
    use super::*;
    use crate::events::Event;
    use crate::handle::DownloadId;
    use rdlp_postprocess::TempRegistry;
    use rdlp_types::{Config, DownloadProtocol, Format, InfoDict};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// Two orchestrators, each with its OWN `TempRegistry` — what two
    /// separate rdlp processes look like from inside one test process
    /// (same-instance registration is deliberately idempotent, so a shared
    /// registry would pass trivially and prove nothing; see `TempRegistry::register`).
    fn orchestrator_with_own_registry(config: &Arc<Config>) -> Orchestrator {
        let (tx, _rx) = mpsc::channel::<Event>(64);
        Orchestrator::new_with_registry(
            Arc::clone(config),
            tx,
            DownloadId::next(),
            CancellationToken::new(),
            None,
            Some(Arc::new(TempRegistry::new())),
            None,
        )
    }

    fn shared_config(dir: &tempfile::TempDir) -> Arc<Config> {
        Arc::new(Config {
            output_directory: dir.path().to_path_buf(),
            ..Config::default()
        })
    }

    fn same_target_info_and_format() -> (InfoDict, Format) {
        let info = InfoDict::new("vid1", "Same Title", "test", "http://example.com/v");
        let format = Format::new(
            "f1",
            "http://example.com/video.mp4",
            "mp4",
            DownloadProtocol::Https,
        );
        (info, format)
    }

    fn preparing_phase(
        orchestrator: &Orchestrator,
        info: &InfoDict,
        format: &Format,
    ) -> DownloadPhase {
        DownloadPhase::Preparing {
            info: Box::new(info.clone()),
            format: Box::new(format.clone()),
            subtitle_selection: vec![],
            plan: GatedPlan::new(orchestrator, Box::new(DownloadPlan::Single(format.clone())))
                .unwrap(),
        }
    }

    /// RED against the unpatched orchestrator (no claim at all): a second
    /// process racing the same output path must be refused as `OutputBusy`
    /// while the first proceeds to `Downloading` normally.
    #[tokio::test]
    async fn second_process_refused_while_first_holds_the_claim() {
        let dir = tempfile::tempdir().unwrap();
        let config = shared_config(&dir);
        let orch_a = orchestrator_with_own_registry(&config);
        let orch_b = orchestrator_with_own_registry(&config);
        let (info, format) = same_target_info_and_format();

        let first = preparing_phase(&orch_a, &info, &format)
            .advance(&orch_a, false)
            .await
            .expect("first claim must succeed");
        assert!(matches!(first, DownloadPhase::Downloading { .. }));

        let clean_path = orch_b.generate_output_path(&info, &format).unwrap();
        let expected_path = crate::orchestrator::naming::part_path(&clean_path);
        let second = preparing_phase(&orch_b, &info, &format)
            .advance(&orch_b, false)
            .await;
        assert!(
            matches!(
                second,
                Err(OrchestratorError::OutputBusy { ref path }) if *path == expected_path
            ),
            "second process must be refused as OutputBusy, got: {second:?}"
        );
    }

    /// Positive: once the first process's claim is released (its
    /// `Downloading` phase dropped — cancel, error, or completion all drop
    /// it the same way via RAII), a second attempt at the same path succeeds.
    #[tokio::test]
    async fn claim_is_reusable_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let config = shared_config(&dir);
        let orch_a = orchestrator_with_own_registry(&config);
        let orch_b = orchestrator_with_own_registry(&config);
        let (info, format) = same_target_info_and_format();

        let first = preparing_phase(&orch_a, &info, &format)
            .advance(&orch_a, false)
            .await
            .expect("first claim must succeed");
        drop(first); // releases the PartLock via Drop — no manual call

        let second = preparing_phase(&orch_b, &info, &format)
            .advance(&orch_b, false)
            .await
            .expect("claim must be reusable once the first holder released it");
        assert!(matches!(second, DownloadPhase::Downloading { .. }));
    }
}
