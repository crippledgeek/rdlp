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

/// Returns the current time as milliseconds since Unix epoch, saturating at
/// `u64::MAX` (year 584942417 — well past any plausible runtime).
fn now_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

impl From<&Event> for EventDto {
    // 17 Event variants → 17 match arms with structured payloads each;
    // splitting per-variant helpers would just shift the matching cost.
    #[allow(clippy::too_many_lines)]
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
                    // Wire-compat: emit as 0.0..=100.0 so external JSON consumers
                    // see the same shape as before the Progress newtype migration.
                    "percentage": progress.progress.map(rdlp_types::Progress::percent),
                    "segments_downloaded": progress.segments_downloaded,
                    "total_segments": progress.total_segments,
                    "eta_seconds": progress.eta.map(|d| d.as_secs()),
                    "total_bytes_estimated": progress.is_estimated,
                }),
            ),

            Event::PostProcessing { stage, .. } => ("post_processing", json!({ "stage": stage })),

            Event::PostProcessProgress {
                stage,
                progress,
                eta,
                ..
            } => (
                "postprocess_progress",
                json!({
                    "stage": stage,
                    "progress": progress,
                    // Whole seconds (floored) — matches the human-facing countdown display.
                    "eta_seconds": eta.map(|d| d.as_secs()),
                }),
            ),

            Event::SubtitlesFound { langs, .. } => ("subtitles_found", json!({ "langs": langs })),

            Event::SubtitlesMissing { requested, .. } => {
                ("subtitles_missing", json!({ "requested": requested }))
            }

            Event::Warning { message, .. } => (
                "warning",
                json!({ "message": rdlp_redact::redact_str(message) }),
            ),

            Event::Completed { output_files, .. } => {
                let output_files: Vec<String> = output_files
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect();
                ("completed", json!({ "output_files": output_files }))
            }

            Event::Failed { error, .. } => (
                "failed",
                json!({
                    "message": AsRef::<str>::as_ref(&error.user_message()),
                    "retryable": error.is_retryable(),
                }),
            ),

            Event::Cancelled { .. } => ("cancelled", json!({})),

            Event::PlaylistDetected { total_items, .. } => {
                ("playlist_detected", json!({ "total_items": total_items }))
            }

            Event::PlaylistItemStarted {
                index,
                total,
                url,
                title,
                ..
            } => (
                "playlist_item_started",
                json!({
                    "index": index,
                    "total": total,
                    "url": url,
                    "title": title,
                }),
            ),

            Event::UnitCompleted { index, .. } => ("unit_completed", json!({ "index": index })),

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
                    "reason": rdlp_redact::redact_str(reason),
                }),
            ),

            Event::Debug { message, .. } => (
                "debug",
                json!({ "message": rdlp_redact::redact_str(message) }),
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
#[allow(
    clippy::indexing_slicing, // serde_json::Value indexing is the test assertion form
    clippy::float_cmp,        // exact-equality on lossless u64→f64 round-trips
)]
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
    fn progress_event_emits_percentage_in_zero_to_hundred_scale() {
        // Regression guard for the Progress newtype migration: the wire format
        // emitted by EventDto MUST keep "percentage" in 0..=100 even though the
        // internal DownloadProgress.progress is now a Progress fraction in
        // 0..=1. External JSON consumers and CLI display rely on the legacy
        // percentage scale.
        let id = DownloadId::next();
        let progress = rdlp_core::DownloadProgress::new(50, Some(100), 1024.0);
        let event = Event::Progress { id, progress };

        let dto = EventDto::from(&event);

        assert_eq!(dto.event_type, "progress");
        assert_eq!(
            dto.payload["percentage"].as_f64().unwrap(),
            50.0,
            "percentage must serialize as 0..=100, not 0..=1"
        );
    }

    #[test]
    fn postprocess_progress_emits_fraction_in_zero_to_one_scale() {
        // The PostProcessProgress wire format is the dual: it stays in 0..=1
        // because the Tauri / frontend layer reads the fraction directly.
        let id = DownloadId::next();
        let event = Event::PostProcessProgress {
            id,
            stage: "remux".into(),
            progress: rdlp_types::Progress::new(0.42),
            eta: None,
        };

        let dto = EventDto::from(&event);

        assert_eq!(dto.event_type, "postprocess_progress");
        let progress = dto.payload["progress"].as_f64().unwrap();
        assert!(
            (0.0..=1.0).contains(&progress),
            "postprocess progress wire value must stay in 0..=1, got {progress}"
        );
        assert!(
            (progress - 0.42).abs() < 1e-5,
            "expected 0.42 fraction, got {progress}"
        );
        assert!(
            dto.payload["eta_seconds"].is_null(),
            "eta_seconds must serialize to null when eta is None"
        );
    }

    #[test]
    fn progress_event_emits_eta_seconds_and_estimated_flag() {
        // Known total + fast speed → eta Some(10s); not estimated.
        let known = Event::Progress {
            id: DownloadId::next(),
            progress: rdlp_core::DownloadProgress::new(0, Some(10_000), 1000.0),
        };
        let dto = EventDto::from(&known);
        assert_eq!(dto.payload["eta_seconds"].as_u64(), Some(10)); // 10000/1000 = 10s
        assert_eq!(dto.payload["total_bytes_estimated"].as_bool(), Some(false));

        // No total → eta null.
        let unknown = Event::Progress {
            id: DownloadId::next(),
            progress: rdlp_core::DownloadProgress::new(50, None, 1000.0),
        };
        let dto = EventDto::from(&unknown);
        assert!(dto.payload["eta_seconds"].is_null());
        assert_eq!(dto.payload["total_bytes_estimated"].as_bool(), Some(false));
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
