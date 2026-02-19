//! Persistent application settings.
//!
//! Stores user preferences such as the download directory, default
//! post-processing options, and subtitle configuration. Settings are
//! serializable for persistence between sessions.

use std::path::PathBuf;

use rdlp_types::{AudioFormat, ContainerFormat, SubtitleFormat};
use serde::{Deserialize, Serialize};

/// Application settings that persist between sessions.
///
/// # Defaults
///
/// - `output_dir`: the platform download directory, falling back to
///   the home directory, then `"."`.
/// - `embed_thumbnail`: `true` (matches rdlp CLI default).
/// - All other fields default to `false`, `None`, or empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Directory where downloaded files are saved.
    pub output_dir: PathBuf,
    /// Default container format for remuxing (e.g. Mp4, Mkv).
    pub default_remux: Option<ContainerFormat>,
    /// Default audio extraction format (e.g. Mp3, Opus).
    pub default_extract_audio: Option<AudioFormat>,
    /// Default subtitle format (e.g. Srt, Vtt).
    pub default_subtitle_format: Option<SubtitleFormat>,
    /// Default subtitle language codes (e.g. `["en", "sv"]`).
    pub default_subtitle_langs: Vec<String>,
    /// Whether to embed thumbnails in downloaded media.
    pub embed_thumbnail: bool,
    /// Whether to embed metadata tags in downloaded media.
    pub embed_metadata: bool,
    /// Whether to enable verbose logging.
    pub verbose: bool,
    /// Default search provider site name (e.g. `"xhamster"`).
    pub default_search_provider: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        let output_dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));

        Self {
            output_dir,
            default_remux: None,
            default_extract_audio: None,
            default_subtitle_format: None,
            default_subtitle_langs: Vec::new(),
            embed_thumbnail: true,
            embed_metadata: false,
            verbose: false,
            default_search_provider: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();

        // output_dir should be a real path (not empty)
        assert!(
            !settings.output_dir.as_os_str().is_empty(),
            "output_dir should not be empty"
        );

        // Thumbnail embedding enabled by default
        assert!(settings.embed_thumbnail);

        // Everything else defaults to off / None / empty
        assert!(!settings.embed_metadata);
        assert!(!settings.verbose);
        assert!(settings.default_remux.is_none());
        assert!(settings.default_extract_audio.is_none());
        assert!(settings.default_subtitle_format.is_none());
        assert!(settings.default_subtitle_langs.is_empty());
        assert!(settings.default_search_provider.is_none());
    }

    #[test]
    fn test_settings_round_trip() {
        let settings = AppSettings {
            output_dir: PathBuf::from("/tmp/downloads"),
            default_remux: Some(ContainerFormat::Mkv),
            default_extract_audio: Some(AudioFormat::Mp3),
            default_subtitle_format: Some(SubtitleFormat::Srt),
            default_subtitle_langs: vec!["en".to_owned(), "sv".to_owned()],
            embed_thumbnail: false,
            embed_metadata: true,
            verbose: true,
            default_search_provider: Some("xhamster".to_owned()),
        };

        let json = serde_json::to_string(&settings).expect("serialization should succeed");
        let restored: AppSettings =
            serde_json::from_str(&json).expect("deserialization should succeed");

        assert_eq!(restored.output_dir, PathBuf::from("/tmp/downloads"));
        assert_eq!(restored.default_remux, Some(ContainerFormat::Mkv));
        assert_eq!(restored.default_extract_audio, Some(AudioFormat::Mp3));
        assert_eq!(restored.default_subtitle_format, Some(SubtitleFormat::Srt));
        assert_eq!(restored.default_subtitle_langs, vec!["en", "sv"]);
        assert!(!restored.embed_thumbnail);
        assert!(restored.embed_metadata);
        assert!(restored.verbose);
        assert_eq!(
            restored.default_search_provider.as_deref(),
            Some("xhamster")
        );
    }
}
