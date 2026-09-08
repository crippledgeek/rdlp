//! Keeping the user's download when rdlp refuses a request (#577).
//!
//! Three stages hand a caller-chosen target container to `FFmpeg`
//! (`RemuxStage`, `RecodeStage`, `MergeStage`) and can therefore be told
//! "no, not into that container". All three are fatal stages, so the error
//! ends the run and drops the `PipelineMessage` — and `FileTracker`'s
//! cancel-`Drop` then deletes `current_files`, i.e. the download that just
//! completed.
//!
//! That is right for a processing *failure* and wrong for a policy
//! *refusal*: the input was never touched, and the operator can fix the flag
//! and re-run — but only if the media is still there. This module is the one
//! place that distinction is applied, so the three stages cannot drift apart
//! on it.
//!
//! `mod policy_refusal;` is private, so the `pub` below reaches no further
//! than this crate — `pub(crate)` inside a private module is what
//! `clippy::redundant_pub_crate` rejects.

use rdlp_ffmpeg::PostProcessError;

use crate::pipeline::PipelineMessage;

/// Preserve the live working set when `error` is a policy refusal rather than
/// a processing failure. A no-op for every other error, so the existing
/// cancel-cleanup behaviour is unchanged.
///
/// `stage` names the caller in the log line: the survivor keeps its temp name
/// (the final rename is the orchestrator's, success-path only — #406), so
/// telling the operator exactly which file was kept is the difference between
/// a recoverable run and a puzzling one.
pub fn keep_download_on_policy_refusal(
    error: &PostProcessError,
    msg: &mut PipelineMessage,
    stage: &'static str,
) {
    if !error.is_audio_only_container_refusal() {
        return;
    }
    msg.tracker.preserve_current_files();
    for kept in &msg.tracker.current_files {
        log::warn!(
            "{stage}: refused the requested container; your downloaded media is kept at {}",
            kept.display()
        );
    }
}

#[cfg(test)]
// Safe: test fixtures — no async runtime in these `#[test]` fns.
#[allow(clippy::disallowed_methods, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use rdlp_ffmpeg::PostProcessError;
    use rdlp_types::{ContainerFormat, InfoDict, PostProcess};
    use tempfile::TempDir;

    use super::keep_download_on_policy_refusal;
    use crate::pipeline::{FileTracker, PipelineMessage, TempRegistry};

    /// A message whose single current file exists on disk, so the assertions
    /// below can observe whether `Drop` deleted it. The `TempDir` is returned
    /// and held by the caller — dropping it early would remove the file for
    /// the wrong reason and make both tests pass vacuously.
    fn msg_with_a_real_file() -> (TempDir, PipelineMessage, PathBuf) {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("download.rdlp-tmp-577.mp4");
        std::fs::write(&file, b"media").unwrap();
        let msg = PipelineMessage {
            info: InfoDict::new(
                "id".to_string(),
                "Test".to_string(),
                "Test".to_string(),
                "https://example.com".to_string(),
            ),
            tracker: FileTracker::new(vec![file.clone()], Arc::new(TempRegistry::new())),
            config: Arc::new(PostProcess::default()),
            original_stem: "test".to_string(),
            is_hls: false,
            verbose: false,
            callback_factory: None,
            warnings: Vec::new(),
            encoding_tool: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        (dir, msg, file)
    }

    fn refusal() -> PostProcessError {
        // Wrapped in `Other(anyhow(..))`, which is the shape the async
        // `FFmpegRunner` wrappers actually produce — a test built on the bare
        // variant would pass against a predicate that cannot see through the
        // wrapper, which is the only interesting way this can be wrong.
        PostProcessError::Other(anyhow::Error::new(
            PostProcessError::AudioOnlyContainerRejectsVideo {
                container: ContainerFormat::Wma,
                codec: "h264".to_string(),
                alternative: ContainerFormat::Wmv,
            },
        ))
    }

    /// A policy refusal preserves the working set: the tracker must no longer
    /// consider the file deletable.
    #[test]
    fn a_refusal_preserves_the_current_files() {
        let (_dir, mut msg, file) = msg_with_a_real_file();
        keep_download_on_policy_refusal(&refusal(), &mut msg, "RemuxStage");
        drop(msg);
        assert!(
            file.exists(),
            "a policy refusal must not cost the user their download"
        );
    }

    /// The negative half: an ordinary processing failure keeps the existing
    /// cancel-cleanup behaviour. Without this, "preserve everything always"
    /// would pass the test above.
    #[test]
    fn an_ordinary_failure_still_cleans_up() {
        let (_dir, mut msg, file) = msg_with_a_real_file();
        let err = PostProcessError::ffmpeg_failed("encoder exploded");
        keep_download_on_policy_refusal(&err, &mut msg, "RemuxStage");
        drop(msg);
        assert!(
            !file.exists(),
            "a genuine processing failure must still clean up its working set"
        );
    }
}
