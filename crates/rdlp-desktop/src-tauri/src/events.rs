//! Tauri event types for frontend notifications.
//!
//! This module defines serializable event payloads and an [`emit_event`]
//! function that maps rdlp-api [`Event`] variants to Tauri frontend
//! events. The frontend listens for these events via Tauri's IPC
//! event system to update the download queue UI in real time.

use log::debug;
use rdlp_api::Event;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Download progress payload emitted as `"download-progress"`.
///
/// Sent for every [`Event::Progress`] update. The `progress` field is a
/// fraction in `[0.0, 1.0]` (percentage divided by 100).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgressPayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
    /// Download progress as a fraction in `[0.0, 1.0]`.
    pub(crate) progress: f64,
    /// Human-readable speed string (e.g. `"5.2 MB/s"`), if available.
    pub(crate) speed: Option<String>,
    /// Formatted ETA string (e.g. `"02:30"`), if available.
    pub(crate) eta: Option<String>,
    /// Total bytes downloaded so far.
    pub(crate) downloaded_bytes: u64,
    /// Total expected bytes, if known.
    pub(crate) total_bytes: Option<u64>,
    /// `true` when `total_bytes` is an extrapolated estimate (segmented download).
    pub(crate) is_estimated: bool,
    /// Fragments completed so far (segmented HLS/DASH downloads); `None` for
    /// progressive HTTP. Drives the desktop `frag N/M` secondary counter.
    pub(crate) segments_downloaded: Option<u64>,
    /// Total fragment count (segmented HLS/DASH downloads); `None` for
    /// progressive HTTP.
    pub(crate) total_segments: Option<u64>,
}

/// Download completion payload emitted as `"download-complete"`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadCompletePayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
    /// Path to the primary output file on disk.
    pub(crate) filepath: String,
}

/// Download error payload emitted as `"download-error"`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadErrorPayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
    /// Human-readable error message.
    pub(crate) error: String,
    /// Whether the frontend should offer a retry button.
    pub(crate) retryable: bool,
}

/// Download cancellation payload emitted as `"download-cancelled"`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadCancelledPayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
}

/// Log severity level for download events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    /// Informational messages (metadata ready, post-processing stages).
    Info,
    /// Warning messages (retries, quality fallbacks).
    Warn,
    /// Debug/verbose messages (detailed extraction and download steps).
    Debug,
}

/// Post-processing progress payload emitted as `"postprocess-progress"`.
///
/// Sent for every [`Event::PostProcessProgress`] update. The `progress`
/// field is a fraction in `[0.0, 1.0]`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessProgressPayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
    /// Name of the post-processing stage (e.g. `"remux"`, `"normalize"`).
    pub(crate) stage: String,
    /// Progress as a fraction in `[0.0, 1.0]`.
    pub(crate) progress: f64,
    /// Formatted ETA string (e.g. `"02:30"`), once past warm-up.
    pub(crate) eta: Option<String>,
}

/// Payload for "unit-started" Tauri event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnitStartedPayload {
    pub(crate) job_id: String,
    pub(crate) unit_index: usize,
    pub(crate) unit_total: usize,
    pub(crate) unit_title: String,
}

/// Log message payload emitted as `"download-log"`.
///
/// Used for informational events (metadata ready, post-processing
/// stages, warnings, retry attempts) that the frontend may display
/// in a log panel or status bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadLogPayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
    /// Log severity level.
    pub(crate) level: LogLevel,
    /// Human-readable log message.
    pub(crate) message: String,
}

/// Format a [`std::time::Duration`] as `"MM:SS"` or `"HH:MM:SS"`.
pub(crate) fn format_eta(eta: &std::time::Duration) -> String {
    let total_secs = eta.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// Forward an rdlp-api [`Event`] to the Tauri frontend.
///
/// Maps each relevant event variant to the appropriate frontend event
/// name and payload. Events that have no frontend representation
/// (e.g. `Started`, `SubtitlesFound`) are silently ignored.
///
/// # Arguments
///
/// * `app` - The Tauri application handle for emitting events.
/// * `job_id` - The UUID string identifying the download job.
/// * `event` - The rdlp-api event to forward.
#[allow(clippy::too_many_lines)]
pub fn emit_event(app: &AppHandle, job_id: &str, event: &Event) {
    match event {
        Event::Progress { progress, .. } => {
            // DownloadProgress.progress is already clamped to [0.0, 1.0] by the
            // Progress newtype — emit the fraction directly without rescaling.
            let pct = progress.progress.map_or(0.0, |p| f64::from(p.fraction()));

            let payload = DownloadProgressPayload {
                job_id: job_id.to_owned(),
                progress: pct,
                speed: Some(progress.speed_string()),
                eta: progress.eta.as_ref().map(format_eta),
                downloaded_bytes: progress.bytes_downloaded,
                total_bytes: progress.total_bytes,
                is_estimated: progress.is_estimated,
                segments_downloaded: progress.segments_downloaded,
                total_segments: progress.total_segments,
            };

            if let Err(e) = app.emit("download-progress", &payload) {
                debug!("Failed to emit download-progress for job {job_id}: {e}");
            }
        }

        Event::Completed { output_files, .. } => {
            let filepath = output_files
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default();

            let payload = DownloadCompletePayload {
                job_id: job_id.to_owned(),
                filepath,
            };

            if let Err(e) = app.emit("download-complete", &payload) {
                debug!("Failed to emit download-complete for job {job_id}: {e}");
            }
        }

        Event::Failed { error, .. } => {
            let payload = DownloadErrorPayload {
                job_id: job_id.to_owned(),
                error: error.user_message().into_owned(),
                retryable: error.is_retryable(),
            };

            if let Err(e) = app.emit("download-error", &payload) {
                debug!("Failed to emit download-error for job {job_id}: {e}");
            }
        }

        Event::MetadataReady { info, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Metadata ready: {}", info.title),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                debug!("Failed to emit download-log (metadata-ready) for job {job_id}: {e}");
            }
        }

        Event::PostProcessing { stage, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Post-processing: {stage}"),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                debug!("Failed to emit download-log (post-processing) for job {job_id}: {e}");
            }
        }

        Event::PostProcessProgress {
            stage,
            progress,
            eta,
            ..
        } => {
            let payload = PostProcessProgressPayload {
                job_id: job_id.to_owned(),
                stage: stage.clone(),
                progress: f64::from(progress.fraction()),
                eta: eta.as_ref().map(format_eta),
            };
            if let Err(e) = app.emit("postprocess-progress", &payload) {
                debug!("Failed to emit postprocess-progress for job {job_id}: {e}");
            }
        }

        Event::Warning { message, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Warn,
                message: message.clone(),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                debug!("Failed to emit download-log (warning) for job {job_id}: {e}");
            }
        }

        Event::Retrying {
            attempt,
            max_attempts,
            reason,
            ..
        } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Warn,
                message: format!("Retrying ({attempt}/{max_attempts}): {reason}"),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                debug!("Failed to emit download-log (retrying) for job {job_id}: {e}");
            }
        }

        Event::FormatSelected {
            format_id, quality, ..
        } => {
            // Emit as a log message for the status bar
            let log = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Format selected: {quality} ({format_id})"),
            };
            if let Err(e) = app.emit("download-log", &log) {
                debug!("Failed to emit download-log (format-selected) for job {job_id}: {e}");
            }
        }

        Event::Debug { message, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Debug,
                message: message.clone(),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                debug!("Failed to emit download-log (debug) for job {job_id}: {e}");
            }
        }

        Event::PlaylistDetected { total_items, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Playlist detected: {total_items} items"),
            };
            if let Err(e) = app.emit("download-log", &payload) {
                debug!("Failed to emit download-log (playlist-detected) for job {job_id}: {e}");
            }
        }

        Event::PlaylistItemStarted {
            index,
            total,
            title,
            ..
        } => {
            let payload = UnitStartedPayload {
                job_id: job_id.to_owned(),
                unit_index: *index,
                unit_total: *total,
                unit_title: title.clone(),
            };
            if let Err(e) = app.emit("unit-started", &payload) {
                debug!("Failed to emit unit-started for job {job_id}: {e}");
            }
        }

        Event::Cancelled { .. } => {
            let payload = DownloadCancelledPayload {
                job_id: job_id.to_owned(),
            };
            if let Err(e) = app.emit("download-cancelled", &payload) {
                debug!("Failed to emit download-cancelled for job {job_id}: {e}");
            }
        }

        // Remaining unhandled events (SubtitlesFound, SubtitlesMissing, Started,
        // UnitCompleted, Retrying from playlist)
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_format_eta_minutes_seconds() {
        let dur = std::time::Duration::from_secs(150);
        assert_eq!(format_eta(&dur), "02:30");
    }

    #[test]
    fn test_format_eta_hours() {
        let dur = std::time::Duration::from_secs(3661);
        assert_eq!(format_eta(&dur), "01:01:01");
    }

    #[test]
    fn test_format_eta_zero() {
        let dur = std::time::Duration::from_secs(0);
        assert_eq!(format_eta(&dur), "00:00");
    }

    #[test]
    fn test_progress_payload_serializes() {
        let payload = DownloadProgressPayload {
            job_id: "abc-123".to_owned(),
            progress: 0.5,
            speed: Some("2.1 MB/s".to_owned()),
            eta: Some("01:30".to_owned()),
            downloaded_bytes: 1024,
            total_bytes: Some(2048),
            is_estimated: true,
            segments_downloaded: Some(12),
            total_segments: Some(40),
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["progress"], 0.5);
        assert_eq!(json["speed"], "2.1 MB/s");
        assert_eq!(json["eta"], "01:30");
        assert_eq!(json["downloadedBytes"], 1024);
        assert_eq!(json["totalBytes"], 2048);
        assert_eq!(json["isEstimated"], true);
        assert_eq!(
            json["segmentsDownloaded"], 12,
            "frag-done counter on the wire"
        );
        assert_eq!(json["totalSegments"], 40, "frag-total counter on the wire");
    }

    #[test]
    fn test_complete_payload_serializes() {
        let payload = DownloadCompletePayload {
            job_id: "abc-123".to_owned(),
            filepath: "/tmp/video.mp4".to_owned(),
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["filepath"], "/tmp/video.mp4");
    }

    #[test]
    fn test_error_payload_serializes() {
        let payload = DownloadErrorPayload {
            job_id: "abc-123".to_owned(),
            error: "connection timeout".to_owned(),
            retryable: true,
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["error"], "connection timeout");
        assert_eq!(json["retryable"], true);
    }

    #[test]
    fn test_log_payload_serializes() {
        let payload = DownloadLogPayload {
            job_id: "abc-123".to_owned(),
            level: LogLevel::Warn,
            message: "low quality fallback".to_owned(),
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["level"], "warn");
        assert_eq!(json["message"], "low quality fallback");
    }

    #[test]
    fn test_postprocess_progress_payload_serializes() {
        let payload = PostProcessProgressPayload {
            job_id: "abc-123".to_owned(),
            stage: "remux".to_owned(),
            progress: 0.45,
            eta: Some("02:30".to_owned()),
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["stage"], "remux");
        assert_eq!(json["progress"], 0.45);
        assert_eq!(json["eta"], "02:30");
    }

    #[test]
    fn test_cancelled_payload_serializes_camel_case() {
        let payload = DownloadCancelledPayload {
            job_id: "abc-123".to_owned(),
        };
        let v = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(v["jobId"], "abc-123");
        assert!(v.get("job_id").is_none(), "must be camelCase on the wire");
    }
}
