//! Download lifecycle events.
//!
//! [`Event`] represents every observable state change during a download.
//! All variants carry a [`DownloadId`] so consumers can correlate events
//! with the originating download.

use crate::errors::RdlpApiError;
use crate::handle::DownloadId;
use rdlp_core::DownloadProgress;
use rdlp_types::{InfoDict, Progress};
use std::time::Duration;

/// A download lifecycle event.
///
/// Every variant includes a [`DownloadId`] accessible via
/// [`Event::download_id()`] so consumers can route events to the
/// correct download context.
#[derive(Debug, Clone)]
pub enum Event {
    /// Download has been accepted and queued.
    Started {
        /// The download this event belongs to.
        id: DownloadId,
        /// The URL being downloaded.
        url: String,
    },

    /// Metadata extraction completed successfully.
    MetadataReady {
        /// The download this event belongs to.
        id: DownloadId,
        /// Extracted metadata (boxed to reduce enum size).
        info: Box<InfoDict>,
    },

    /// A format has been selected for download.
    FormatSelected {
        /// The download this event belongs to.
        id: DownloadId,
        /// The selected format identifier.
        format_id: String,
        /// Human-readable quality description (e.g. "1080p").
        quality: String,
    },

    /// Download progress update.
    Progress {
        /// The download this event belongs to.
        id: DownloadId,
        /// Current progress snapshot.
        progress: DownloadProgress,
    },

    /// Post-processing stage started.
    PostProcessing {
        /// The download this event belongs to.
        id: DownloadId,
        /// Name of the post-processing stage (e.g. "remux", "thumbnail").
        stage: String,
    },

    /// Post-processing progress update.
    PostProcessProgress {
        /// The download this event belongs to.
        id: DownloadId,
        /// Name of the post-processing stage (e.g. "remux", "normalize").
        stage: String,
        /// Clamped progress fraction. Use [`Progress::percent`] at display sites.
        progress: Progress,
        /// Smoothed estimated time remaining for this stage, once past warm-up.
        eta: Option<Duration>,
    },

    /// Subtitles were found for the requested languages.
    SubtitlesFound {
        /// The download this event belongs to.
        id: DownloadId,
        /// Language codes that were found.
        langs: Vec<String>,
    },

    /// Requested subtitle languages were not available.
    SubtitlesMissing {
        /// The download this event belongs to.
        id: DownloadId,
        /// Language codes that were requested but not found.
        requested: Vec<String>,
    },

    /// Non-fatal warning during download.
    Warning {
        /// The download this event belongs to.
        id: DownloadId,
        /// Warning message.
        message: String,
    },

    /// Download completed successfully.
    ///
    /// Carries only the output file paths. Consumers needing full metadata
    /// (`InfoDict`, stats) should use the return value from `download()`.
    Completed {
        /// The download this event belongs to.
        id: DownloadId,
        /// Output files produced by the download.
        output_files: Vec<std::path::PathBuf>,
    },

    /// Download failed with an error.
    Failed {
        /// The download this event belongs to.
        id: DownloadId,
        /// The error that caused the failure.
        error: RdlpApiError,
    },

    /// Download was cancelled by the user.
    Cancelled {
        /// The download this event belongs to.
        id: DownloadId,
    },

    /// A playlist was detected at the given URL.
    PlaylistDetected {
        /// The download this event belongs to.
        id: DownloadId,
        /// Total number of items in the playlist.
        total_items: usize,
    },

    /// A playlist item download has started.
    PlaylistItemStarted {
        /// The download this event belongs to.
        id: DownloadId,
        /// 1-based index of the item within the playlist.
        index: usize,
        /// Total number of items in the playlist.
        total: usize,
        /// URL of the playlist item.
        url: String,
        /// Title of the playlist item (episode name).
        title: String,
    },

    /// A download unit (episode or merge stream) has completed.
    UnitCompleted {
        /// The download this event belongs to.
        id: DownloadId,
        /// 1-based index of the completed unit.
        index: usize,
    },

    /// A download attempt is being retried.
    Retrying {
        /// The download this event belongs to.
        id: DownloadId,
        /// Current attempt number (1-based).
        attempt: u32,
        /// Maximum number of attempts allowed.
        max_attempts: u32,
        /// Reason for the retry.
        reason: String,
    },

    /// Debug-level log message emitted when verbose mode is enabled.
    ///
    /// These messages provide detailed internal state for troubleshooting
    /// (e.g. available processors, output file paths). Only emitted when
    /// `config.verbose` is `true`.
    Debug {
        /// The download this event belongs to.
        id: DownloadId,
        /// Debug message.
        message: String,
    },
}

impl Event {
    /// Returns the [`DownloadId`] associated with this event.
    #[must_use]
    pub const fn download_id(&self) -> DownloadId {
        match self {
            Self::Started { id, .. }
            | Self::MetadataReady { id, .. }
            | Self::FormatSelected { id, .. }
            | Self::Progress { id, .. }
            | Self::PostProcessing { id, .. }
            | Self::PostProcessProgress { id, .. }
            | Self::SubtitlesFound { id, .. }
            | Self::SubtitlesMissing { id, .. }
            | Self::Warning { id, .. }
            | Self::Completed { id, .. }
            | Self::Failed { id, .. }
            | Self::Cancelled { id }
            | Self::PlaylistDetected { id, .. }
            | Self::PlaylistItemStarted { id, .. }
            | Self::UnitCompleted { id, .. }
            | Self::Retrying { id, .. }
            | Self::Debug { id, .. } => *id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_id_accessor() {
        let id = DownloadId::next();
        let event = Event::Started {
            id,
            url: "https://example.com/video".into(),
        };
        assert_eq!(event.download_id(), id);
    }

    #[test]
    fn test_download_id_accessor_all_variants() {
        let id = DownloadId::next();
        let progress = DownloadProgress::new(1024, Some(4096), 512.0);

        let events: Vec<Event> = vec![
            Event::Started {
                id,
                url: "https://example.com".into(),
            },
            // MetadataReady and Completed require complex data — skipped
            Event::FormatSelected {
                id,
                format_id: "hls-720".into(),
                quality: "720p".into(),
            },
            Event::Progress { id, progress },
            Event::PostProcessing {
                id,
                stage: "remux".into(),
            },
            Event::PostProcessProgress {
                id,
                stage: "remux".into(),
                progress: Progress::new(0.45),
                eta: None,
            },
            Event::SubtitlesFound {
                id,
                langs: vec!["en".into(), "sv".into()],
            },
            Event::SubtitlesMissing {
                id,
                requested: vec!["ja".into()],
            },
            Event::Warning {
                id,
                message: "low quality fallback".into(),
            },
            Event::Failed {
                id,
                error: RdlpApiError::UserCancelled,
            },
            Event::Cancelled { id },
            Event::PlaylistDetected {
                id,
                total_items: 10,
            },
            Event::PlaylistItemStarted {
                id,
                index: 1,
                total: 10,
                url: "https://example.com/1".into(),
                title: "Episode 1".into(),
            },
            Event::UnitCompleted { id, index: 1 },
            Event::Retrying {
                id,
                attempt: 2,
                max_attempts: 5,
                reason: "connection reset".into(),
            },
            Event::Debug {
                id,
                message: "Available processors: remux, thumbnail".into(),
            },
        ];

        for event in &events {
            assert_eq!(
                event.download_id(),
                id,
                "download_id() mismatch for {event:?}"
            );
        }
    }

    #[test]
    fn test_completed_event_carries_output_files() {
        let id = DownloadId::next();
        let files = vec![std::path::PathBuf::from("/tmp/video.mp4")];
        let event = Event::Completed {
            id,
            output_files: files.clone(),
        };
        assert_eq!(event.download_id(), id);
        if let Event::Completed { output_files, .. } = &event {
            assert_eq!(output_files, &files);
        }
    }

    #[test]
    fn test_completed_event_empty_output_files() {
        let id = DownloadId::next();
        let event = Event::Completed {
            id,
            output_files: vec![],
        };
        if let Event::Completed { output_files, .. } = &event {
            assert!(output_files.is_empty());
        }
    }

    #[test]
    fn test_completed_event_clone() {
        let id = DownloadId::next();
        let event = Event::Completed {
            id,
            output_files: vec![std::path::PathBuf::from("/tmp/a.mp4")],
        };
        let cloned = event;
        assert_eq!(cloned.download_id(), id);
    }
}
