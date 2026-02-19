//! Download queue state.
//!
//! Tracks all queued, active, and completed downloads for the desktop
//! application. [`DownloadQueue`] holds both serializable job metadata
//! and non-serializable [`DownloadHandle`]s for active downloads.

use std::collections::HashMap;
use std::fmt;

use rdlp_api::DownloadHandle;
use serde::Serialize;

/// Lifecycle status of a download job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    /// Queued but not yet started.
    Pending,
    /// Actively downloading.
    Running,
    /// Download finished successfully.
    Completed,
    /// Download failed with an error.
    Failed,
    /// Download was cancelled by the user.
    Cancelled,
}

/// Serializable metadata for a single download job.
///
/// This struct is sent to the frontend over IPC. Progress and speed
/// fields are updated in-place as events arrive from the download
/// engine.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadJob {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// Source URL being downloaded.
    pub url: String,
    /// Human-readable title extracted from metadata.
    pub title: Option<String>,
    /// Current lifecycle status.
    pub status: JobStatus,
    /// Download progress as a fraction in `[0.0, 1.0]`.
    pub progress: Option<f64>,
    /// Human-readable download speed (e.g. `"5.2 MB/s"`).
    pub speed: Option<String>,
    /// Estimated time remaining (e.g. `"00:42"`).
    pub eta: Option<String>,
    /// Error message if the job failed.
    pub error: Option<String>,
    /// Whether a failed job can be retried.
    pub retryable: bool,
    /// Unix timestamp (seconds) when the download started.
    pub started_at: Option<i64>,
    /// Unix timestamp (seconds) when the download completed/failed.
    pub completed_at: Option<i64>,
    /// Final output file path on disk.
    pub output_path: Option<String>,
}

/// In-memory download queue holding all download entries.
///
/// This struct is **not** `Serialize` because it holds live
/// [`DownloadHandle`]s. Individual [`DownloadJob`]s are serializable
/// and sent to the frontend via IPC.
pub struct DownloadQueue {
    /// Job metadata keyed by job ID.
    jobs: HashMap<String, DownloadJob>,
    /// Insertion order of job IDs.
    order: Vec<String>,
    /// Active download handles keyed by job ID.
    handles: HashMap<String, DownloadHandle>,
}

impl DownloadQueue {
    /// Create a new, empty download queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            order: Vec::new(),
            handles: HashMap::new(),
        }
    }

    /// Add a new pending job for the given URL.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique job identifier (typically a UUID v4).
    /// * `url` - Source URL to download.
    ///
    /// # Returns
    ///
    /// A reference to the newly created [`DownloadJob`].
    pub fn add_job(&mut self, id: impl Into<String>, url: impl Into<String>) -> &DownloadJob {
        let id = id.into();
        let job = DownloadJob {
            id: id.clone(),
            url: url.into(),
            title: None,
            status: JobStatus::Pending,
            progress: None,
            speed: None,
            eta: None,
            error: None,
            retryable: false,
            started_at: None,
            completed_at: None,
            output_path: None,
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

    /// Store a [`DownloadHandle`] for an active download.
    ///
    /// # Arguments
    ///
    /// * `id` - The job ID to associate the handle with.
    /// * `handle` - The active download handle.
    pub fn set_handle(&mut self, id: impl Into<String>, handle: DownloadHandle) {
        self.handles.insert(id.into(), handle);
    }

    /// Take (remove) the [`DownloadHandle`] for a job.
    ///
    /// Returns `None` if no handle is stored for the given ID.
    pub fn take_handle(&mut self, id: &str) -> Option<DownloadHandle> {
        self.handles.remove(id)
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
        self.handles.remove(id);
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
            .field("handles", &format_args!("{} active", self.handles.len()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get_job() {
        let mut queue = DownloadQueue::new();
        let job = queue.add_job("job-1", "https://example.com/v1");
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
        queue.add_job("a", "https://example.com/a");
        queue.add_job("b", "https://example.com/b");
        queue.add_job("c", "https://example.com/c");

        let ids: Vec<&str> = queue.all_jobs().iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_remove_completed_job() {
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1");

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
        queue.add_job("job-1", "https://example.com/v1");
        queue.get_job_mut("job-1").unwrap().status = JobStatus::Running;

        assert!(!queue.remove_job("job-1"));
        assert!(queue.get_job("job-1").is_some());
    }

    #[test]
    fn test_job_serializes() {
        let mut queue = DownloadQueue::new();
        queue.add_job("job-1", "https://example.com/v1");

        let job = queue.get_job("job-1").unwrap();
        let json = serde_json::to_value(job).expect("serialization should succeed");

        assert_eq!(json["id"], "job-1");
        assert_eq!(json["url"], "https://example.com/v1");
        assert_eq!(json["status"], "pending");
        assert!(json["title"].is_null());
        assert!(json["progress"].is_null());
        assert_eq!(json["retryable"], false);
    }
}
