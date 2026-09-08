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
/// `stage` names the caller in the log line, and the line names the path the
/// survivor was moved to (`preserve_current_files` renames it out of the
/// `.rdlp-tmp-` namespace so the stale sweep cannot take it). That path is not
/// the clean output name — the pipeline never knows it on a failing run — so
/// logging it is the difference between a recoverable run and a puzzling one.
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
    /// consider the file deletable, and `current_files` must name where it
    /// actually went.
    #[test]
    fn a_refusal_preserves_the_current_files() {
        let (_dir, mut msg, original) = msg_with_a_real_file();
        keep_download_on_policy_refusal(&refusal(), &mut msg, "RemuxStage");

        let kept = msg.tracker.current_files.clone();
        assert_eq!(kept.len(), 1, "the working set must not lose an entry");
        let kept = kept[0].clone();
        assert_ne!(
            kept, original,
            "the survivor must have been moved out of the temp namespace"
        );

        drop(msg);
        assert!(
            kept.exists(),
            "a policy refusal must not cost the user their download"
        );
        assert!(
            !original.exists(),
            "the temp-named original must not be left behind as a duplicate"
        );
    }

    /// The stale sweep is the second half of the same data-loss story, and the
    /// half a `Drop`-only fix misses. `cleanup_stale` does NOT consult the
    /// registry — it `read_dir`s the output directory and matches
    /// `name.contains(".rdlp-tmp-")`, sparing a file only while another
    /// process holds its `.lock`. A preserved survivor has had its lock
    /// released, so if it kept the temp name it would be deleted here the
    /// moment it aged past the window: at CLI startup, desktop startup, or
    /// desktop exit.
    ///
    /// Fails against a `preserve_current_files` that keeps the temp name.
    /// Same trap and same remedy as #416-M1's `.rdlp-bak-` sidecar.
    #[test]
    fn a_preserved_survivor_outlives_the_stale_sweep() {
        let (dir, mut msg, _original) = msg_with_a_real_file();
        keep_download_on_policy_refusal(&refusal(), &mut msg, "RemuxStage");
        let kept = msg.tracker.current_files[0].clone();
        drop(msg);

        age_past_the_sweep_window(&kept);
        TempRegistry::cleanup_stale(dir.path());

        assert!(
            kept.exists(),
            "the kept download must survive the stale sweep at {}",
            kept.display()
        );
    }

    /// Backdate `path`'s mtime well past `cleanup_stale`'s one-hour window, so
    /// the sweep judges it stale rather than "created moments ago".
    ///
    /// Two hours, not one: the threshold is `age >= one_hour` measured against
    /// wall-clock `now`, so a value exactly on the boundary would make the test
    /// depend on which side of the comparison the clock lands.
    fn age_past_the_sweep_window(path: &std::path::Path) {
        // `duration_suboptimal_units` wants `Duration::from_hours(2)` here.
        // That constructor is Rust **1.91**, and this workspace declares
        // `rust-version = "1.88"` (root `Cargo.toml:48`), which covers a
        // package's tests — so taking the suggestion compiles here on 1.97 and
        // breaks `cargo test` for anyone on the stated floor.
        //
        // The lint fires only because clippy cannot see the floor: no crate
        // sets `rust-version.workspace = true` and `clippy.toml` has no `msrv`
        // key, so clippy assumes the current toolchain and suggests APIs the
        // workspace has not adopted. Setting `msrv = "1.88"` in `clippy.toml`
        // would fix this for every crate at once and retire this allow — a
        // workspace-wide call, not one a test helper should make.
        #[allow(
            clippy::duration_suboptimal_units,
            reason = "suggested Duration::from_hours is Rust 1.91; workspace MSRV is 1.88"
        )]
        const WELL_PAST_THE_WINDOW: std::time::Duration =
            std::time::Duration::from_secs(2 * 60 * 60);

        let backdated = std::time::SystemTime::now() - WELL_PAST_THE_WINDOW;
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open the kept file to backdate it");
        file.set_times(std::fs::FileTimes::new().set_modified(backdated))
            .expect("backdate the kept file's mtime");
    }

    /// The negative half: an ordinary processing failure keeps the existing
    /// cancel-cleanup behaviour. Without this, "preserve everything always"
    /// would pass the test above.
    #[test]
    fn an_ordinary_failure_still_cleans_up() {
        let (_dir, mut msg, file) = msg_with_a_real_file();
        let err = PostProcessError::ffmpeg_failed("encoder exploded");
        keep_download_on_policy_refusal(&err, &mut msg, "RemuxStage");
        assert_eq!(
            msg.tracker.current_files,
            vec![file.clone()],
            "an ordinary failure must not rename anything either"
        );
        drop(msg);
        assert!(
            !file.exists(),
            "a genuine processing failure must still clean up its working set"
        );
    }
}
