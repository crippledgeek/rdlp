//! Rich subtitle track metadata and extraction result types
//!
//! Provides [`SubtitleTrack`] (richer than the legacy [`Subtitle`](crate::info_dict::Subtitle)
//! wire format), [`SubtitleResult`] for pipeline status reporting, and the
//! [`normalize_from_info_dict`] bridge function.

use crate::info_dict::Subtitle;
use crate::subtitle_kind::SubtitleKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Rich subtitle track with full metadata.
///
/// Internal pipeline format — richer than [`Subtitle`] which remains the
/// `InfoDict` serialization contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleTrack {
    /// Language code (e.g., "en", "ja") or human-readable label
    pub language: String,
    /// URL to the subtitle file
    pub url: String,
    /// File extension (e.g., "srt", "vtt")
    pub ext: String,
    /// Whether this is an auto-generated caption
    pub is_auto: bool,
    /// Track purpose classification
    #[serde(default)]
    pub kind: SubtitleKind,
    /// Optional human-readable name (e.g., "English", "CC")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Status of subtitle availability for a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleStatus {
    /// Subtitles were found and are available
    Available,
    /// No subtitles exist for this content
    NotAvailable,
    /// Subtitles exist but are restricted (auth required, geo-blocked)
    Restricted,
    /// Could not determine subtitle availability
    Unknown,
    /// A non-fatal error occurred during extraction or validation
    ErrorSoft,
}

/// Reason why subtitles are missing or restricted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleReason {
    /// The subtitle fields in InfoDict were None
    FieldMissing,
    /// The subtitle map was present but empty
    EmptyList,
    /// Subtitles exist but none matched requested languages
    FilteredOutByLanguage,
    /// Authentication required to access subtitles
    RequiresAuth,
    /// Subtitle URL contains an expired or signed token
    UrlExpiredOrSigned,
    /// Server-side selection required (cannot pre-fetch)
    ServerSelectionRequired,
    /// Subtitles embedded in HLS/DASH manifest (not standalone files)
    ManifestEmbedded,
    /// Subtitle URL returned a non-success HTTP status
    UrlNotReachable(u16),
}

/// Diagnostic key-value pair for debugging subtitle issues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleDiagnostic {
    /// Diagnostic key (e.g., "extractor", "field_checked")
    pub key: String,
    /// Diagnostic value
    pub value: String,
}

/// Result of subtitle extraction and normalization.
///
/// Wraps extracted tracks with status information so the policy
/// layer can decide whether to warn, skip, or fail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleResult {
    /// Successfully extracted/normalized tracks
    pub tracks: Vec<SubtitleTrack>,
    /// Overall subtitle availability status
    pub status: SubtitleStatus,
    /// Reasons explaining the status (may be multiple)
    pub reasons: Vec<SubtitleReason>,
    /// Diagnostic metadata for debugging
    pub diagnostics: Vec<SubtitleDiagnostic>,
}

impl SubtitleResult {
    /// Create a result indicating subtitles are available.
    ///
    /// # Arguments
    ///
    /// * `tracks` - Extracted subtitle tracks to include in the result
    ///
    /// # Returns
    ///
    /// A `SubtitleResult` with status [`SubtitleStatus::Available`].
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::SubtitleResult;
    /// let result = SubtitleResult::available(vec![]);
    /// assert_eq!(result.status, rdlp_types::SubtitleStatus::Available);
    /// ```
    #[must_use]
    pub fn available(tracks: Vec<SubtitleTrack>) -> Self {
        Self {
            tracks,
            status: SubtitleStatus::Available,
            reasons: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Create a result indicating no subtitles are available.
    ///
    /// # Arguments
    ///
    /// * `reasons` - Why subtitles are unavailable (e.g., field missing, empty)
    ///
    /// # Returns
    ///
    /// A `SubtitleResult` with status [`SubtitleStatus::NotAvailable`] and
    /// no tracks.
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::{SubtitleReason, SubtitleResult};
    /// let result = SubtitleResult::not_available(vec![SubtitleReason::EmptyList]);
    /// assert!(!result.has_tracks());
    /// ```
    #[must_use]
    pub fn not_available(reasons: Vec<SubtitleReason>) -> Self {
        Self {
            tracks: Vec::new(),
            status: SubtitleStatus::NotAvailable,
            reasons,
            diagnostics: Vec::new(),
        }
    }

    /// Create a result indicating a soft error.
    ///
    /// # Arguments
    ///
    /// * `reasons` - Why the error occurred
    /// * `diagnostics` - Key-value pairs for debugging
    ///
    /// # Returns
    ///
    /// A `SubtitleResult` with status [`SubtitleStatus::ErrorSoft`] and
    /// no tracks.
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::{SubtitleDiagnostic, SubtitleReason, SubtitleResult};
    /// let result = SubtitleResult::error_soft(
    ///     vec![SubtitleReason::RequiresAuth],
    ///     vec![SubtitleDiagnostic { key: "status".into(), value: "401".into() }],
    /// );
    /// assert!(!result.has_tracks());
    /// ```
    #[must_use]
    pub fn error_soft(reasons: Vec<SubtitleReason>, diagnostics: Vec<SubtitleDiagnostic>) -> Self {
        Self {
            tracks: Vec::new(),
            status: SubtitleStatus::ErrorSoft,
            reasons,
            diagnostics,
        }
    }

    /// Whether any downloadable tracks exist.
    ///
    /// # Returns
    ///
    /// `true` if `tracks` is non-empty, `false` otherwise.
    ///
    /// # Example
    ///
    /// ```
    /// use rdlp_types::SubtitleResult;
    /// assert!(!SubtitleResult::available(vec![]).has_tracks());
    /// ```
    #[must_use]
    pub fn has_tracks(&self) -> bool {
        !self.tracks.is_empty()
    }
}

/// Convert legacy `InfoDict` subtitle maps into a [`SubtitleResult`].
///
/// Bridges the existing `Subtitle` wire format to the richer pipeline types.
/// Manual subtitles get `is_auto = false`; automatic captions get `is_auto = true`.
///
/// # Arguments
///
/// * `subtitles` - Manual subtitle map from `InfoDict.subtitles`
/// * `automatic_captions` - Auto-generated caption map from
///   `InfoDict.automatic_captions`
///
/// # Returns
///
/// A [`SubtitleResult`] with status `Available` (when tracks exist) or
/// `NotAvailable` (when both maps are `None` or empty).
///
/// # Example
///
/// ```
/// use rdlp_types::subtitle_track::normalize_from_info_dict;
/// let result = normalize_from_info_dict(None, None);
/// assert!(!result.has_tracks());
/// ```
#[must_use]
pub fn normalize_from_info_dict(
    subtitles: Option<&HashMap<String, Vec<Subtitle>>>,
    automatic_captions: Option<&HashMap<String, Vec<Subtitle>>>,
) -> SubtitleResult {
    let has_manual = subtitles.is_some_and(|s| !s.is_empty());
    let has_auto = automatic_captions.is_some_and(|a| !a.is_empty());

    if !has_manual && !has_auto {
        let reason = if subtitles.is_none() && automatic_captions.is_none() {
            SubtitleReason::FieldMissing
        } else {
            SubtitleReason::EmptyList
        };
        return SubtitleResult::not_available(vec![reason]);
    }

    let mut tracks = Vec::new();

    if let Some(subs) = subtitles {
        for (lang, entries) in subs {
            for entry in entries {
                tracks.push(SubtitleTrack {
                    language: lang.clone(),
                    url: entry.url.clone(),
                    ext: entry.ext.clone(),
                    is_auto: false,
                    kind: SubtitleKind::Normal,
                    name: entry.name.clone(),
                });
            }
        }
    }

    if let Some(auto) = automatic_captions {
        for (lang, entries) in auto {
            for entry in entries {
                tracks.push(SubtitleTrack {
                    language: lang.clone(),
                    url: entry.url.clone(),
                    ext: entry.ext.clone(),
                    is_auto: true,
                    kind: SubtitleKind::Normal,
                    name: entry.name.clone(),
                });
            }
        }
    }

    SubtitleResult::available(tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(url: &str, ext: &str) -> Subtitle {
        Subtitle {
            url: url.to_string(),
            ext: ext.to_string(),
            name: None,
        }
    }

    #[test]
    fn test_both_fields_none() {
        let result = normalize_from_info_dict(None, None);
        assert_eq!(result.status, SubtitleStatus::NotAvailable);
        assert_eq!(result.reasons, vec![SubtitleReason::FieldMissing]);
        assert!(result.tracks.is_empty());
    }

    #[test]
    fn test_both_fields_empty() {
        let empty: HashMap<String, Vec<Subtitle>> = HashMap::new();
        let result = normalize_from_info_dict(Some(&empty), Some(&empty));
        assert_eq!(result.status, SubtitleStatus::NotAvailable);
        assert_eq!(result.reasons, vec![SubtitleReason::EmptyList]);
    }

    #[test]
    fn test_manual_subs_only() {
        let mut subs = HashMap::new();
        subs.insert(
            "en".to_string(),
            vec![sub("https://example.com/en.srt", "srt")],
        );
        let result = normalize_from_info_dict(Some(&subs), None);
        assert_eq!(result.status, SubtitleStatus::Available);
        assert!(result.reasons.is_empty());
        assert_eq!(result.tracks.len(), 1);
        assert!(!result.tracks[0].is_auto);
        assert_eq!(result.tracks[0].language, "en");
    }

    #[test]
    fn test_auto_captions_only() {
        let mut auto = HashMap::new();
        auto.insert(
            "ja".to_string(),
            vec![sub("https://example.com/ja.vtt", "vtt")],
        );
        let result = normalize_from_info_dict(None, Some(&auto));
        assert_eq!(result.status, SubtitleStatus::Available);
        assert_eq!(result.tracks.len(), 1);
        assert!(result.tracks[0].is_auto);
    }

    #[test]
    fn test_mixed_manual_and_auto() {
        let mut subs = HashMap::new();
        subs.insert(
            "en".to_string(),
            vec![sub("https://example.com/en.srt", "srt")],
        );
        let mut auto = HashMap::new();
        auto.insert(
            "ja".to_string(),
            vec![sub("https://example.com/ja.vtt", "vtt")],
        );
        let result = normalize_from_info_dict(Some(&subs), Some(&auto));
        assert_eq!(result.status, SubtitleStatus::Available);
        assert_eq!(result.tracks.len(), 2);

        let manual_count = result.tracks.iter().filter(|t| !t.is_auto).count();
        let auto_count = result.tracks.iter().filter(|t| t.is_auto).count();
        assert_eq!(manual_count, 1);
        assert_eq!(auto_count, 1);
    }

    #[test]
    fn test_manual_empty_auto_has_tracks() {
        let empty: HashMap<String, Vec<Subtitle>> = HashMap::new();
        let mut auto = HashMap::new();
        auto.insert(
            "en".to_string(),
            vec![sub("https://example.com/en.vtt", "vtt")],
        );
        let result = normalize_from_info_dict(Some(&empty), Some(&auto));
        assert_eq!(result.status, SubtitleStatus::Available);
        assert_eq!(result.tracks.len(), 1);
        assert!(result.tracks[0].is_auto);
    }

    #[test]
    fn test_convenience_constructors() {
        let avail = SubtitleResult::available(vec![]);
        assert_eq!(avail.status, SubtitleStatus::Available);

        let na = SubtitleResult::not_available(vec![SubtitleReason::FieldMissing]);
        assert_eq!(na.status, SubtitleStatus::NotAvailable);
        assert!(!na.has_tracks());

        let err = SubtitleResult::error_soft(
            vec![SubtitleReason::RequiresAuth],
            vec![SubtitleDiagnostic {
                key: "http_status".into(),
                value: "401".into(),
            }],
        );
        assert_eq!(err.status, SubtitleStatus::ErrorSoft);
        assert_eq!(err.diagnostics.len(), 1);
    }
}
