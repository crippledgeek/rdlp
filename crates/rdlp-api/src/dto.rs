//! Serializable event DTO for UI bridges (Tauri, Leptos SSE).
//!
//! This module is only available when the `serde` feature is enabled.

use crate::events::Event;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

/// Flat, serializable representation of an [`Event`].
///
/// Converts the strongly-typed [`Event`] enum into a JSON-friendly struct
/// with a string tag, opaque payload, and timestamp. Designed for transport
/// over IPC channels (Tauri commands, SSE streams).
///
/// # Examples
///
/// ```ignore
/// use rdlp_api::events::Event;
/// use rdlp_api::dto::EventDto;
///
/// let event = Event::Cancelled { id };
/// let dto = EventDto::from(&event);
/// let json = serde_json::to_string(&dto).unwrap();
/// ```
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EventDto {
    /// Download this event belongs to.
    pub download_id: u64,
    /// Event type tag (e.g., "started", "progress", "completed").
    pub event_type: String,
    /// Variant-specific data.
    pub payload: serde_json::Value,
    /// Timestamp in milliseconds since Unix epoch.
    pub timestamp_ms: u64,
}

/// Returns the current time as milliseconds since Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl From<&Event> for EventDto {
    fn from(event: &Event) -> Self {
        let download_id = event.download_id().as_u64();
        let timestamp_ms = now_ms();

        let (event_type, payload) = match event {
            Event::Started { url, .. } => ("started", json!({ "url": url })),

            Event::MetadataReady { info, .. } => (
                "metadata_ready",
                json!({
                    "title": info.title,
                    "id": info.id,
                    "extractor": info.extractor,
                }),
            ),

            Event::FormatSelected {
                format_id, quality, ..
            } => (
                "format_selected",
                json!({
                    "format_id": format_id,
                    "quality": quality,
                }),
            ),

            Event::Progress { progress, .. } => (
                "progress",
                json!({
                    "bytes_downloaded": progress.bytes_downloaded,
                    "total_bytes": progress.total_bytes,
                    "speed": progress.speed,
                    "percentage": progress.percentage,
                    "segments_downloaded": progress.segments_downloaded,
                    "total_segments": progress.total_segments,
                }),
            ),

            Event::PostProcessing { stage, .. } => ("post_processing", json!({ "stage": stage })),

            Event::SubtitlesFound { langs, .. } => ("subtitles_found", json!({ "langs": langs })),

            Event::SubtitlesMissing { requested, .. } => {
                ("subtitles_missing", json!({ "requested": requested }))
            }

            Event::Warning { message, .. } => ("warning", json!({ "message": message })),

            Event::Completed { result, .. } => {
                let output_files: Vec<String> = result
                    .output_files
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                ("completed", json!({ "output_files": output_files }))
            }

            Event::Failed { error, .. } => (
                "failed",
                json!({
                    "message": error.user_message().as_ref(),
                    "retryable": error.is_retryable(),
                }),
            ),

            Event::Cancelled { .. } => ("cancelled", json!({})),

            Event::PlaylistDetected { total_items, .. } => {
                ("playlist_detected", json!({ "total_items": total_items }))
            }

            Event::PlaylistItemStarted {
                index, total, url, ..
            } => (
                "playlist_item_started",
                json!({
                    "index": index,
                    "total": total,
                    "url": url,
                }),
            ),

            Event::Retrying {
                attempt,
                max_attempts,
                reason,
                ..
            } => (
                "retrying",
                json!({
                    "attempt": attempt,
                    "max_attempts": max_attempts,
                    "reason": reason,
                }),
            ),
        };

        Self {
            download_id,
            event_type: event_type.to_owned(),
            payload,
            timestamp_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RdlpApiError;
    use crate::handle::DownloadId;

    #[test]
    fn test_started_dto() {
        let id = DownloadId::next();
        let event = Event::Started {
            id,
            url: "https://example.com/video".into(),
        };

        let dto = EventDto::from(&event);

        assert_eq!(dto.download_id, id.as_u64());
        assert_eq!(dto.event_type, "started");
        assert_eq!(dto.payload["url"], "https://example.com/video");
        assert!(dto.timestamp_ms > 0);
    }

    #[test]
    fn test_failed_dto() {
        let id = DownloadId::next();
        let event = Event::Failed {
            id,
            error: RdlpApiError::NetworkError {
                message: "connection reset".into(),
                status: Some(503),
            },
        };

        let dto = EventDto::from(&event);

        assert_eq!(dto.event_type, "failed");
        assert!(dto.payload["retryable"].as_bool().unwrap());
        assert!(dto.payload["message"].as_str().unwrap().contains("503"));
    }

    #[test]
    fn test_dto_round_trip_json() {
        let id = DownloadId::next();
        let event = Event::Started {
            id,
            url: "https://example.com/video".into(),
        };

        let dto = EventDto::from(&event);
        let json_str = serde_json::to_string(&dto).unwrap();
        let restored: EventDto = serde_json::from_str(&json_str).unwrap();

        assert_eq!(restored.download_id, dto.download_id);
        assert_eq!(restored.event_type, dto.event_type);
        assert_eq!(restored.payload, dto.payload);
        assert_eq!(restored.timestamp_ms, dto.timestamp_ms);
    }
}
