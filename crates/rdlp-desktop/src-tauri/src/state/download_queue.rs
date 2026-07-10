//! Download queue state.
//!
//! Tracks all queued, active, and completed downloads for the desktop
//! application. [`DownloadQueue`] holds serializable job metadata and
//! cancel closures for active downloads.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::commands::download::PlaylistContext;

/// Lifecycle status of a download job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// Queued but not yet started.
    Pending,
    /// Actively downloading.
    Running,
    /// Download finished; post-processing (recode/remux/embed) in progress.
    Processing,
    /// Download finished successfully.
    Completed,
    /// Download failed with an error.
    Failed,
    /// Download was cancelled by the user.
    Cancelled,
}

/// Upper bound on the character length of a stored [`CurrentUnit::title`].
///
/// `title` originates from extractor-supplied episode titles (arriving via
/// `Event::PlaylistItemStarted`) and is serialized into every `queue` IPC
/// response for the job's lifetime. A pathologically long title would be
/// re-sent on every poll with no size bound (the UI's `truncate` CSS is
/// visual-only). Clamping at storage time bounds the serialized payload
/// (≤ 4× this value in bytes, worst case for 4-byte UTF-8) regardless of
/// what the extractor produces. 512 characters comfortably fits any
/// legitimate episode/merge-phase label while bounding abuse; the localhost
/// IPC trust boundary makes this defense-in-depth (issue #371).
const MAX_UNIT_TITLE_LEN: usize = 512;

/// Clamp a unit title to [`MAX_UNIT_TITLE_LEN`] characters, cutting on a UTF-8
/// character boundary (never a byte offset — that would panic or split a
/// multi-byte sequence). Returns the input allocation unchanged when within
/// the cap; single-pass via [`str::char_indices`].
fn clamp_unit_title(title: String) -> String {
    match title.char_indices().nth(MAX_UNIT_TITLE_LEN) {
        // The byte offset of the (cap+1)th char is a valid boundary to cut at.
        Some((byte_idx, _)) => title[..byte_idx].to_owned(),
        // Fewer than cap+1 chars — already within bound, no reallocation.
        None => title,
    }
}

/// Live snapshot of the in-progress sub-unit (playlist episode or merge phase).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentUnit {
    pub(crate) index: usize,
    pub(crate) total: usize,
    pub(crate) title: String,
}

/// Serializable metadata for a single download job.
///
/// This struct is sent to the frontend over IPC. Progress and speed
/// fields are updated in-place as events arrive from the download
/// engine.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadJob {
    /// Unique identifier (UUID v4).
    pub(crate) id: String,
    /// Source URL being downloaded.
    pub(crate) url: String,
    /// Human-readable title extracted from metadata.
    pub(crate) title: Option<String>,
    /// Current lifecycle status.
    pub(crate) status: JobStatus,
    /// Download progress as a fraction in `[0.0, 1.0]`.
    pub(crate) progress: Option<f64>,
    /// Human-readable download speed (e.g. `"5.2 MB/s"`).
    pub(crate) speed: Option<String>,
    /// Estimated time remaining (e.g. `"00:42"`).
    pub(crate) eta: Option<String>,
    /// Active post-processing stage name (e.g. "Recode"); `None` outside post-processing.
    pub(crate) stage: Option<String>,
    /// Live snapshot of the in-progress sub-unit; `None` when not in a unit.
    #[serde(rename = "currentUnit")]
    pub(crate) current_unit: Option<CurrentUnit>,
    /// Error message if the job failed.
    pub(crate) error: Option<String>,
    /// Whether a failed job can be retried.
    pub(crate) retryable: bool,
    /// Unix timestamp (seconds) when the download started.
    pub(crate) started_at: Option<i64>,
    /// Unix timestamp (seconds) when the download completed/failed.
    pub(crate) completed_at: Option<i64>,
    /// Final output file path on disk.
    pub(crate) output_path: Option<String>,
    /// The options used when this download was started (for retry).
    pub(crate) options: Option<SavedDownloadOptions>,
    /// Playlist membership — `None` for standalone downloads.
    pub(crate) playlist: Option<PlaylistContext>,
}

impl DownloadJob {
    /// Mark the job as entering a post-processing stage (download finished,
    /// `FFmpeg` work in progress). Sets status to [`JobStatus::Processing`].
    pub(crate) fn begin_postprocessing(&mut self, stage: String) {
        self.status = JobStatus::Processing;
        self.stage = Some(stage);
    }

    /// Update post-processing progress for the current stage.
    pub(crate) fn set_postprocess_progress(&mut self, stage: String, fraction: f64) {
        self.status = JobStatus::Processing;
        self.stage = Some(stage);
        self.progress = Some(fraction);
    }

    /// Record the in-progress sub-unit (playlist episode or merge Video/Audio).
    ///
    /// `title` is extractor-supplied and reaches the IPC boundary; it is
    /// clamped to [`MAX_UNIT_TITLE_LEN`] characters before storage so a
    /// pathologically long title cannot bloat every `queue` response (#371).
    pub(crate) fn set_current_unit(&mut self, index: usize, total: usize, title: String) {
        self.current_unit = Some(CurrentUnit {
            index,
            total,
            title: clamp_unit_title(title),
        });
    }

    /// Clear the in-progress sub-unit (unit finished / job done).
    pub(crate) fn clear_current_unit(&mut self) {
        self.current_unit = None;
    }
}

/// A serializable snapshot of the options used when a download was started.
///
/// Stored on [`DownloadJob`] so that retry operations can reconstruct the
/// original [`crate::commands::download::DownloadOptions`] exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SavedDownloadOptions {
    /// Raw JSON of the original `DownloadOptions` sent by the frontend.
    pub(crate) json: serde_json::Value,
}

/// In-memory download queue holding all download entries.
///
/// This struct is **not** `Serialize` because it holds live cancel
/// closures. Individual [`DownloadJob`]s are serializable and sent to
/// the frontend via IPC.
pub struct DownloadQueue {
    /// Job metadata keyed by job ID.
    jobs: HashMap<String, DownloadJob>,
    /// Insertion order of job IDs.
    order: Vec<String>,
    /// Cancel functions for active downloads, keyed by job ID.
    cancel_fns: HashMap<String, Box<dyn Fn() + Send + Sync>>,
}

impl DownloadQueue {
    /// Create a new, empty download queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            order: Vec::new(),
            cancel_fns: HashMap::new(),
        }
    }

    /// Add a new pending job for the given URL.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique job identifier (typically a UUID v4).
    /// * `url` - Source URL to download.
    /// * `title` - Optional title to display immediately (e.g. from search results).
    /// * `playlist` - Optional playlist membership context for batch downloads.
    ///
    /// # Panics
    ///
    /// Panics if the job cannot be found immediately after insertion, which
    /// indicates a logic error in the internal [`HashMap`] (should never occur).
    ///
    /// # Returns
    ///
    /// A reference to the newly created [`DownloadJob`].
    #[allow(clippy::expect_used)]
    pub fn add_job(
        &mut self,
        id: impl Into<String>,
        url: impl Into<String>,
        title: Option<String>,
        playlist: Option<PlaylistContext>,
    ) -> &DownloadJob {
        let id = id.into();
        let job = DownloadJob {
            id: id.clone(),
            url: url.into(),
            title,
            status: JobStatus::Pending,
            progress: None,
            speed: None,
            eta: None,
            stage: None,
            current_unit: None,
            error: None,
            retryable: false,
            started_at: None,
            completed_at: None,
            output_path: None,
            options: None,
            playlist,
        };
        self.order.push(id.clone());
        self.jobs.insert(id.clone(), job);
        self.jobs.get(&id).expect("just inserted")
    }

    /// Look up a job by ID.
    #[must_use]
    pub fn get_job(&self, id: &str) -> Option<&DownloadJob> {
        self.jobs.get(id)
    }

    /// Look up a job by ID (mutable).
    pub fn get_job_mut(&mut self, id: &str) -> Option<&mut DownloadJob> {
        self.jobs.get_mut(id)
    }

    /// Return all jobs in insertion order.
    #[must_use]
    pub fn all_jobs(&self) -> Vec<&DownloadJob> {
        self.order
            .iter()
            .filter_map(|id| self.jobs.get(id))
            .collect()
    }

    /// Store a cancel function for an active download.
    ///
    /// # Arguments
    ///
    /// * `id` - The job ID to associate the cancel function with.
    /// * `cancel_fn` - A function that cancels the download when called.
    pub fn set_cancel(&mut self, id: impl Into<String>, cancel_fn: Box<dyn Fn() + Send + Sync>) {
        self.cancel_fns.insert(id.into(), cancel_fn);
    }

    /// Atomically store the cancel function and transition a job to Running.
    ///
    /// Combines `set_cancel` + status transition into a single operation so
    /// that no concurrent observer can see the job in Pending-with-cancel or
    /// Running-without-cancel intermediate states.
    ///
    /// # Arguments
    ///
    /// * `id` - The job ID to update.
    /// * `cancel_fn` - A function that cancels the download when called.
    /// * `started_at` - Unix timestamp (seconds) when the download started.
    ///
    /// # Returns
    ///
    /// `true` if the job was found and updated, `false` otherwise.
    pub fn start_job(
        &mut self,
        id: &str,
        cancel_fn: Box<dyn Fn() + Send + Sync>,
        started_at: i64,
    ) -> bool {
        self.cancel_fns.insert(id.to_owned(), cancel_fn);
        if let Some(job) = self.jobs.get_mut(id) {
            job.status = JobStatus::Running;
            job.started_at = Some(started_at);
            true
        } else {
            false
        }
    }

    /// Take (remove) the cancel function for a job.
    ///
    /// Returns `None` if no cancel function is stored for the given ID.
    pub fn take_cancel(&mut self, id: &str) -> Option<Box<dyn Fn() + Send + Sync>> {
        self.cancel_fns.remove(id)
    }

    /// Remove all jobs in a terminal state from the queue.
    ///
    /// Removes all jobs with status `Completed`, `Failed`, or `Cancelled`.
    /// Returns the number of jobs removed.
    pub fn clear_completed(&mut self) -> usize {
        let terminal_ids: Vec<String> = self
            .jobs
            .iter()
            .filter(|(_, j)| {
                matches!(
                    j.status,
                    JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled
                )
            })
            .map(|(id, _)| id.clone())
            .collect();

        let count = terminal_ids.len();
        for id in &terminal_ids {
            self.jobs.remove(id);
            self.cancel_fns.remove(id);
        }
        self.order.retain(|id| !terminal_ids.contains(id));
        count
    }

    /// Remove a job from the queue.
    ///
    /// Only jobs in a terminal state (`Completed`, `Failed`, or
    /// `Cancelled`) may be removed. Returns `true` if the job was
    /// removed, `false` if the job was not found or is still active.
    pub fn remove_job(&mut self, id: &str) -> bool {
        let dominated = matches!(
            self.jobs.get(id).map(|j| &j.status),
            Some(JobStatus::Completed | JobStatus::Failed | JobStatus::Cancelled)
        );
        if !dominated {
            return false;
        }
        self.jobs.remove(id);
        self.order.retain(|oid| oid != id);
        self.cancel_fns.remove(id);
        true
    }
}

impl Default for DownloadQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DownloadQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloadQueue")
            .field("jobs", &self.jobs)
            .field("order", &self.order)
            .field(
                "cancel_fns",
                &format_args!("{} active", self.cancel_fns.len()),
            )
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get_job() {
        let mut queue = DownloadQueue::new();
        let job = queue.add_job("job-1", "https://example.com/v1", None, None);
        assert_eq!(job.id, "job-1");
        assert_eq!(job.url, "https://example.com/v1");
        assert_eq!(job.status, JobStatus::Pending);

        let fetched = queue.get_job("job-1").expect("job should exist");
        assert_eq!(fetched.id, "job-1");

        assert!(queue.get_job("nonexistent").is_none());
    }

    #[test]
    fn test_all_jobs_order() {
        let mut queue = DownloadQueue::new();
        queue.add_job("a", "https://example.com/a", None, None);
        queue.add_job("b", "https://example.com/b", None, None);
        queue.add_job("c", "https://example.com/c", None, None);

        let ids: Vec<&str> = queue.all_jobs().iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_remove_completed_job() {
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1", None, None);

        // Cannot remove a pending job
        assert!(!queue.remove_job("job-1"));

        // Mark as completed, then remove
        queue.get_job_mut("job-1").unwrap().status = JobStatus::Completed;
        assert!(queue.remove_job("job-1"));
        assert!(queue.get_job("job-1").is_none());
        assert!(queue.all_jobs().is_empty());
    }

    #[test]
    fn test_cannot_remove_running_job() {
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1", None, None);
        queue.get_job_mut("job-1").unwrap().status = JobStatus::Running;

        assert!(!queue.remove_job("job-1"));
        assert!(queue.get_job("job-1").is_some());
    }

    #[test]
    fn test_job_serializes() {
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1", None, None);

        let job = queue.get_job("job-1").unwrap();
        let json = serde_json::to_value(job).expect("serialization should succeed");

        assert_eq!(json["id"], "job-1");
        assert_eq!(json["url"], "https://example.com/v1");
        assert_eq!(json["status"], "pending");
        assert!(json["title"].is_null());
        assert!(json["progress"].is_null());
        assert_eq!(json["retryable"], false);
    }

    #[test]
    fn test_start_job_sets_running_and_cancel() {
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1", None, None);

        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        let cancel_fn: Box<dyn Fn() + Send + Sync> = Box::new(move || {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        let ts = 1_700_000_000;
        assert!(queue.start_job("job-1", cancel_fn, ts));

        let job = queue.get_job("job-1").expect("job should exist");
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.started_at, Some(ts));

        // Cancel function should be stored and invocable
        let cancel = queue.take_cancel("job-1").expect("cancel fn should exist");
        cancel();
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_clear_completed_removes_terminal_jobs() {
        let mut queue = DownloadQueue::new();
        queue.add_job("a", "https://example.com/a", None, None);
        queue.add_job("b", "https://example.com/b", None, None);
        queue.add_job("c", "https://example.com/c", None, None);

        // a: completed, b: running, c: cancelled
        queue.get_job_mut("a").unwrap().status = JobStatus::Completed;
        queue.get_job_mut("b").unwrap().status = JobStatus::Running;
        queue.get_job_mut("c").unwrap().status = JobStatus::Cancelled;

        let removed = queue.clear_completed();
        assert_eq!(removed, 2); // a and c
        assert!(queue.get_job("a").is_none());
        assert!(queue.get_job("b").is_some());
        assert!(queue.get_job("c").is_none());

        let remaining: Vec<&str> = queue.all_jobs().iter().map(|j| j.id.as_str()).collect();
        assert_eq!(remaining, vec!["b"]);
    }

    #[test]
    fn test_clear_completed_empty_queue() {
        let mut queue = DownloadQueue::new();
        assert_eq!(queue.clear_completed(), 0);
    }

    #[test]
    fn test_start_job_returns_false_for_missing_job() {
        let mut queue = DownloadQueue::new();
        let cancel_fn: Box<dyn Fn() + Send + Sync> = Box::new(|| {});
        assert!(!queue.start_job("nonexistent", cancel_fn, 0));
    }

    fn running_job(q: &mut DownloadQueue) -> &mut DownloadJob {
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").expect("job exists");
        job.status = JobStatus::Running;
        job
    }

    #[test]
    fn begin_postprocessing_flips_to_processing_and_records_stage() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        job.begin_postprocessing("Recode".to_owned());
        assert_eq!(job.status, JobStatus::Processing);
        assert_eq!(job.stage.as_deref(), Some("Recode"));
    }

    #[test]
    fn set_postprocess_progress_sets_status_stage_progress() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        job.set_postprocess_progress("Recode".to_owned(), 0.5);
        assert_eq!(job.status, JobStatus::Processing);
        assert_eq!(job.stage.as_deref(), Some("Recode"));
        assert_eq!(job.progress, Some(0.5));
    }

    #[test]
    fn postprocess_progress_resets_per_stage() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        job.set_postprocess_progress("Recode".to_owned(), 0.9);
        job.set_postprocess_progress("EmbedThumbnail".to_owned(), 0.1);
        assert_eq!(job.stage.as_deref(), Some("EmbedThumbnail"));
        assert_eq!(job.progress, Some(0.1));
    }

    #[test]
    fn begin_postprocessing_is_idempotent() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        job.begin_postprocessing("Recode".to_owned());
        job.begin_postprocessing("Recode".to_owned());
        assert_eq!(job.status, JobStatus::Processing);
    }

    #[test]
    fn fresh_running_job_is_not_processing() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.stage, None);
    }

    /// Documents the field shape the event-loop `Completed` arm produces
    /// (that arm lives in a spawn closure and isn't unit-testable here).
    #[test]
    fn completed_job_shape_has_no_stage() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        job.set_postprocess_progress("Recode".to_owned(), 0.4);
        job.status = JobStatus::Completed;
        job.stage = None;
        job.progress = Some(1.0);
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.stage, None);
    }

    #[test]
    fn processing_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(JobStatus::Processing).unwrap(),
            serde_json::json!("processing")
        );
    }

    #[test]
    fn all_statuses_serialize_to_expected_strings() {
        for (variant, expected) in [
            (JobStatus::Pending, "pending"),
            (JobStatus::Running, "running"),
            (JobStatus::Processing, "processing"),
            (JobStatus::Completed, "completed"),
            (JobStatus::Failed, "failed"),
            (JobStatus::Cancelled, "cancelled"),
        ] {
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::json!(expected)
            );
        }
    }

    #[test]
    fn download_job_serializes_stage_field() {
        let mut q = DownloadQueue::new();
        let job = running_job(&mut q);
        job.begin_postprocessing("Recode".to_owned());
        let v = serde_json::to_value(&*job).unwrap();
        assert_eq!(v["stage"], serde_json::json!("Recode"));

        q.add_job("k", "https://example.com/w", None, None);
        let other = q.get_job("k").unwrap();
        let v2 = serde_json::to_value(other).unwrap();
        assert_eq!(v2["stage"], serde_json::json!(null));
    }

    #[test]
    fn clear_completed_does_not_remove_processing_job() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        q.get_job_mut("j")
            .unwrap()
            .begin_postprocessing("Recode".to_owned());
        let removed = q.clear_completed();
        assert_eq!(removed, 0);
        assert!(q.get_job("j").is_some());
    }

    #[test]
    fn test_start_job_no_intermediate_state() {
        // Verifies that after start_job, the job is atomically
        // Running with a cancel function — never one without the other
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1", None, None);

        // Before start_job: Pending, no cancel
        assert_eq!(queue.get_job("job-1").unwrap().status, JobStatus::Pending);
        assert!(queue.take_cancel("job-1").is_none());

        // Re-add job since take_cancel removed nothing
        let cancel_fn: Box<dyn Fn() + Send + Sync> = Box::new(|| {});
        queue.start_job("job-1", cancel_fn, 1_700_000_000);

        // After start_job: Running AND has cancel — both set atomically
        let job = queue.get_job("job-1").unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.started_at, Some(1_700_000_000));
        assert!(queue.take_cancel("job-1").is_some());
    }

    #[test]
    fn set_current_unit_records_snapshot() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();
        job.set_current_unit(1, 2, "Video".to_owned());
        let u = job.current_unit.as_ref().expect("set");
        assert_eq!((u.index, u.total, u.title.as_str()), (1, 2, "Video"));
    }

    #[test]
    fn set_current_unit_clamps_overlong_title() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();

        // A pathologically long extractor-supplied title.
        let long = "x".repeat(MAX_UNIT_TITLE_LEN * 4);
        job.set_current_unit(1, 2, long);

        let u = job.current_unit.as_ref().expect("set");
        assert_eq!(u.title.chars().count(), MAX_UNIT_TITLE_LEN);
    }

    #[test]
    fn set_current_unit_preserves_short_title() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();
        job.set_current_unit(1, 2, "Ep 1 — The Beginning".to_owned());
        let u = job.current_unit.as_ref().expect("set");
        assert_eq!(u.title, "Ep 1 — The Beginning");
    }

    #[test]
    fn set_current_unit_preserves_title_at_exact_cap() {
        // Off-by-one guard: a title of EXACTLY the cap must pass through
        // unchanged. A `>=` clamp instead of `>` would wrongly truncate a
        // legitimate 512-char label; the far-over clamp tests can't catch that.
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();
        let at_cap = "x".repeat(MAX_UNIT_TITLE_LEN);
        job.set_current_unit(1, 2, at_cap.clone());
        let u = job.current_unit.as_ref().expect("set");
        assert_eq!(u.title, at_cap);
        assert_eq!(u.title.chars().count(), MAX_UNIT_TITLE_LEN);
    }

    #[test]
    fn set_current_unit_clamps_one_over_cap() {
        // The other side of the boundary: one char past the cap IS truncated.
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();
        job.set_current_unit(1, 2, "x".repeat(MAX_UNIT_TITLE_LEN + 1));
        let u = job.current_unit.as_ref().expect("set");
        assert_eq!(u.title.chars().count(), MAX_UNIT_TITLE_LEN);
    }

    #[test]
    fn set_current_unit_clamps_on_char_boundary() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();

        // Multi-byte chars: naive byte-slicing at the cap would panic /
        // corrupt UTF-8. Clamping MUST count and cut on char boundaries.
        let long = "é".repeat(MAX_UNIT_TITLE_LEN + 100);
        job.set_current_unit(1, 2, long);
        let u = job.current_unit.as_ref().expect("set");
        assert_eq!(u.title.chars().count(), MAX_UNIT_TITLE_LEN);
        // Round-trips as valid UTF-8 (no split multi-byte sequence).
        assert!(u.title.chars().all(|c| c == 'é'));
    }

    #[test]
    fn clear_current_unit_resets() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();
        job.set_current_unit(3, 12, "Ep 3".to_owned());
        job.clear_current_unit();
        assert!(job.current_unit.is_none());
    }

    #[test]
    fn current_unit_serializes_to_camelcase_key_or_null() {
        let mut q = DownloadQueue::new();
        q.add_job("j", "https://example.com/v", None, None);
        let job = q.get_job_mut("j").unwrap();
        job.set_current_unit(2, 2, "Audio".to_owned());
        let v = serde_json::to_value(&*job).unwrap();
        assert_eq!(
            v["currentUnit"],
            serde_json::json!({"index":2,"total":2,"title":"Audio"})
        );

        q.add_job("k", "https://example.com/w", None, None);
        let v2 = serde_json::to_value(q.get_job("k").unwrap()).unwrap();
        assert_eq!(v2["currentUnit"], serde_json::json!(null));
    }
}
