//! Tauri event types for frontend notifications.
//!
//! This module defines serializable event payloads and an [`emit_event`]
//! function that maps rdlp-api [`Event`] variants to Tauri frontend
//! events. The frontend listens for these events via Tauri's IPC
//! event system to update the download queue UI in real time.

use log::warn;
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

/// Format-selected payload emitted as `"format-selected"`.
///
/// Sent when the download engine selects a format, informing the
/// frontend of the chosen quality and format identifier.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatSelectedPayload {
    /// The UUID of the download job.
    pub(crate) job_id: String,
    /// The selected format identifier (e.g. `"hls-720"`).
    pub(crate) format_id: String,
    /// Human-readable quality description (e.g. `"1080p"`).
    pub(crate) quality: String,
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
pub fn emit_event(app: &AppHandle, job_id: &str, event: &Event) {
    match event {
        Event::Progress { progress, .. } => {
            let pct = progress
                .percentage
                .map(|p| (p / 100.0).clamp(0.0, 1.0))
                .unwrap_or(0.0);

            let payload = DownloadProgressPayload {
                job_id: job_id.to_owned(),
                progress: pct,
                speed: Some(progress.speed_string()),
                eta: progress.eta.as_ref().map(format_eta),
                downloaded_bytes: progress.bytes_downloaded,
                total_bytes: progress.total_bytes,
            };

            if let Err(e) = app.emit("download-progress", &payload) {
                warn!("Failed to emit download-progress for job {job_id}: {e}");
            }
        }

        Event::Completed { result, .. } => {
            let filepath = result
                .output_files
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default();

            let payload = DownloadCompletePayload {
                job_id: job_id.to_owned(),
                filepath,
            };

            if let Err(e) = app.emit("download-complete", &payload) {
                warn!("Failed to emit download-complete for job {job_id}: {e}");
            }
        }

        Event::Failed { error, .. } => {
            let payload = DownloadErrorPayload {
                job_id: job_id.to_owned(),
                error: error.user_message().into_owned(),
                retryable: error.is_retryable(),
            };

            if let Err(e) = app.emit("download-error", &payload) {
                warn!("Failed to emit download-error for job {job_id}: {e}");
            }
        }

        Event::MetadataReady { info, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Metadata ready: {}", info.title),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                warn!("Failed to emit download-log (metadata-ready) for job {job_id}: {e}");
            }
        }

        Event::PostProcessing { stage, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Post-processing: {stage}"),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                warn!("Failed to emit download-log (post-processing) for job {job_id}: {e}");
            }
        }

        Event::Warning { message, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Warn,
                message: message.clone(),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                warn!("Failed to emit download-log (warning) for job {job_id}: {e}");
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
                warn!("Failed to emit download-log (retrying) for job {job_id}: {e}");
            }
        }

        Event::FormatSelected {
            format_id, quality, ..
        } => {
            let payload = FormatSelectedPayload {
                job_id: job_id.to_owned(),
                format_id: format_id.clone(),
                quality: quality.clone(),
            };
            if let Err(e) = app.emit("format-selected", &payload) {
                warn!("Failed to emit format-selected for job {job_id}: {e}");
            }

            // Also emit as a log message for the status bar
            let log = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Info,
                message: format!("Format selected: {quality} ({format_id})"),
            };
            if let Err(e) = app.emit("download-log", &log) {
                warn!("Failed to emit download-log (format-selected) for job {job_id}: {e}");
            }
        }

        Event::Debug { message, .. } => {
            let payload = DownloadLogPayload {
                job_id: job_id.to_owned(),
                level: LogLevel::Debug,
                message: message.clone(),
            };

            if let Err(e) = app.emit("download-log", &payload) {
                warn!("Failed to emit download-log (debug) for job {job_id}: {e}");
            }
        }

        // All other events are not forwarded to the frontend.
        _ => {}
    }
}

#[cfg(test)]
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
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["progress"], 0.5);
        assert_eq!(json["speed"], "2.1 MB/s");
        assert_eq!(json["eta"], "01:30");
        assert_eq!(json["downloadedBytes"], 1024);
        assert_eq!(json["totalBytes"], 2048);
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
    fn test_format_selected_payload_serializes() {
        let payload = FormatSelectedPayload {
            job_id: "abc-123".to_owned(),
            format_id: "hls-720".to_owned(),
            quality: "720p".to_owned(),
        };
        let json = serde_json::to_value(&payload).expect("serialization should succeed");
        assert_eq!(json["jobId"], "abc-123");
        assert_eq!(json["formatId"], "hls-720");
        assert_eq!(json["quality"], "720p");
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
}
