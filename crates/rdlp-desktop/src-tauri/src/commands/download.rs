//! Download lifecycle IPC commands.
//!
//! Provides Tauri commands to start, cancel, query, and remove
//! downloads from the managed download queue. Each download runs as
//! a background Tokio task, forwarding rdlp-api events to the
//! frontend via [`crate::events::emit_event`].

use std::path::PathBuf;

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::events;
use crate::state::{AppState, DownloadJob, JobStatus};
use rdlp_api::Event;
use rdlp_api::request::{
    DownloadRequest, FormatOptions, OutputOptions, PostProcessOptions, SubtitleOptions,
};

/// Frontend-supplied download options.
///
/// Deserialized from JSON sent by the React frontend. String-typed
/// fields (e.g. `remux`, `extract_audio`) are parsed into typed enums
/// at the boundary.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOptions {
    /// Format selector string (e.g. `"bestvideo+bestaudio"`).
    pub format: Option<String>,
    /// Directory to save the downloaded file.
    pub output_dir: Option<String>,
    /// Whether to download subtitles.
    pub subtitles: bool,
    /// Subtitle language codes (e.g. `["en", "sv"]`).
    pub subtitle_langs: Vec<String>,
    /// Container format to remux into (e.g. `"mp4"`, `"mkv"`).
    pub remux: Option<String>,
    /// Audio format to extract (e.g. `"mp3"`, `"opus"`).
    pub extract_audio: Option<String>,
    /// Whether to embed a thumbnail in the output file.
    pub embed_thumbnail: bool,
}

/// Start a new download for the given URL.
///
/// Validates the URL, adds a job to the queue, builds a
/// [`DownloadRequest`] from the provided options merged with app
/// settings defaults, and spawns a background task to drive the
/// download and forward events to the frontend.
///
/// # Arguments
///
/// * `url` - Source URL to download.
/// * `options` - Frontend-supplied download options.
/// * `state` - Managed application state.
/// * `app` - Tauri application handle for emitting events.
///
/// # Returns
///
/// The UUID string of the newly created download job.
///
/// # Errors
///
/// Returns [`AppError::InvalidInput`] if URL validation fails.
#[tauri::command]
pub async fn start_download(
    url: String,
    options: DownloadOptions,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, AppError> {
    // Validate URL at the boundary
    rdlp_security::validate_url_security(&url).map_err(|e| AppError::InvalidInput {
        field: "url".to_owned(),
        message: e.to_string(),
    })?;

    // Generate a unique job ID
    let job_id = uuid::Uuid::new_v4().to_string();

    // Read settings defaults
    let settings = state
        .settings
        .lock()
        .expect("settings lock poisoned")
        .clone();

    // Add job to queue as Pending
    {
        let mut queue = state.queue.lock().expect("queue lock poisoned");
        queue.add_job(&job_id, &url);
    }

    // Build the download request from options + settings
    let output_dir = options
        .output_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| settings.output_dir.clone());

    let remux = match options.remux {
        Some(ref s) => Some(s.parse().map_err(|_| AppError::InvalidInput {
            field: "remux".to_owned(),
            message: format!("Unknown container format: {s}"),
        })?),
        None => settings.default_remux,
    };

    let extract_audio = match options.extract_audio {
        Some(ref s) => Some(s.parse().map_err(|_| AppError::InvalidInput {
            field: "extractAudio".to_owned(),
            message: format!("Unknown audio format: {s}"),
        })?),
        None => settings.default_extract_audio,
    };

    let request = DownloadRequest {
        url: url.clone(),
        output: OutputOptions {
            output_dir: Some(output_dir),
            ..OutputOptions::default()
        },
        format: FormatOptions {
            selector: options.format,
            ..FormatOptions::default()
        },
        subtitles: SubtitleOptions {
            write_subs: if options.subtitles { Some(true) } else { None },
            sub_langs: options.subtitle_langs,
            sub_format: settings.default_subtitle_format,
            ..SubtitleOptions::default()
        },
        postprocess: PostProcessOptions {
            remux,
            extract_audio,
            embed_thumbnail: Some(options.embed_thumbnail),
            embed_metadata: Some(settings.embed_metadata),
            ..PostProcessOptions::default()
        },
        ..DownloadRequest::default()
    };

    // Start the download via rdlp-api
    let mut handle = state.client.download(request);

    // Mark job as running and set started_at
    let now = chrono::Utc::now().timestamp();
    {
        let mut queue = state.queue.lock().expect("queue lock poisoned");
        if let Some(job) = queue.get_job_mut(&job_id) {
            job.status = JobStatus::Running;
            job.started_at = Some(now);
        }
    }

    // Store the handle for cancellation
    // We need to take the events receiver before storing, so we
    // extract it via a split: store cancel token, spawn event loop.
    let queue = state.queue.clone();
    let id = job_id.clone();

    // Spawn background event loop
    tokio::spawn(async move {
        while let Some(event) = handle.events().recv().await {
            // Forward to frontend
            events::emit_event(&app, &id, &event);

            // Update queue state based on event
            let mut q = queue.lock().expect("queue lock poisoned");
            if let Some(job) = q.get_job_mut(&id) {
                match &event {
                    Event::Progress { progress, .. } => {
                        job.progress = progress.percentage.map(|p| (p / 100.0).clamp(0.0, 1.0));
                        job.speed = Some(progress.speed_string());
                        job.eta = progress.eta.as_ref().map(|d| {
                            let secs = d.as_secs();
                            let m = (secs % 3600) / 60;
                            let s = secs % 60;
                            let h = secs / 3600;
                            if h > 0 {
                                format!("{h:02}:{m:02}:{s:02}")
                            } else {
                                format!("{m:02}:{s:02}")
                            }
                        });
                    }
                    Event::MetadataReady { info, .. } => {
                        job.title = Some(info.title.clone());
                    }
                    Event::Completed { result, .. } => {
                        job.status = JobStatus::Completed;
                        job.progress = Some(1.0);
                        job.completed_at = Some(chrono::Utc::now().timestamp());
                        job.output_path =
                            result.output_files.first().map(|p| p.display().to_string());
                    }
                    Event::Failed { error, .. } => {
                        job.status = JobStatus::Failed;
                        job.completed_at = Some(chrono::Utc::now().timestamp());
                        job.error = Some(error.user_message().into_owned());
                        job.retryable = error.is_retryable();
                    }
                    Event::Cancelled { .. } => {
                        job.status = JobStatus::Cancelled;
                        job.completed_at = Some(chrono::Utc::now().timestamp());
                    }
                    _ => {}
                }
            }
        }
    });

    Ok(job_id)
}

/// Cancel an in-progress download by its job ID.
///
/// Takes the download handle from the queue and calls `cancel()` on it.
/// The job is immediately marked as [`JobStatus::Cancelled`].
///
/// # Arguments
///
/// * `job_id` - UUID of the download job to cancel.
/// * `state` - Managed application state.
///
/// # Errors
///
/// Returns [`AppError::DownloadFailed`] if the job is not found or
/// has no active handle.
#[tauri::command]
pub async fn cancel_download(job_id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let mut queue = state.queue.lock().expect("queue lock poisoned");

    let handle = queue
        .take_handle(&job_id)
        .ok_or_else(|| AppError::DownloadFailed {
            job_id: job_id.clone(),
            message: "No active download handle found".to_owned(),
            retryable: false,
        })?;

    handle.cancel();

    if let Some(job) = queue.get_job_mut(&job_id) {
        job.status = JobStatus::Cancelled;
        job.completed_at = Some(chrono::Utc::now().timestamp());
    }

    Ok(())
}

/// Retrieve the current download queue.
///
/// Returns all queued, active, and completed download jobs in
/// insertion order.
///
/// # Arguments
///
/// * `state` - Managed application state.
///
/// # Returns
///
/// A vector of all [`DownloadJob`]s, cloned from the queue.
#[tauri::command]
pub async fn get_queue(state: State<'_, AppState>) -> Result<Vec<DownloadJob>, AppError> {
    let queue = state.queue.lock().expect("queue lock poisoned");
    Ok(queue.all_jobs().into_iter().cloned().collect())
}

/// Remove a completed, failed, or cancelled download from the queue.
///
/// Only jobs in a terminal state may be removed. Active or pending
/// jobs cannot be removed (cancel them first).
///
/// # Arguments
///
/// * `job_id` - UUID of the download job to remove.
/// * `state` - Managed application state.
///
/// # Errors
///
/// Returns [`AppError::DownloadFailed`] if the job cannot be removed
/// (not found or still active).
#[tauri::command]
pub async fn remove_job(job_id: String, state: State<'_, AppState>) -> Result<(), AppError> {
    let mut queue = state.queue.lock().expect("queue lock poisoned");

    if !queue.remove_job(&job_id) {
        return Err(AppError::DownloadFailed {
            job_id,
            message: "Job not found or still active".to_owned(),
            retryable: false,
        });
    }

    Ok(())
}
