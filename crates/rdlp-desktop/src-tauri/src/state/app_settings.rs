//! Persistent application settings.
//!
//! Stores user preferences such as the download directory, default
//! post-processing options, and subtitle configuration. Settings are
//! serializable for persistence between sessions.

use std::path::PathBuf;

use log::{info, warn};
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

impl AppSettings {
    /// Return the path to the settings file on disk.
    ///
    /// Uses the platform config directory (`dirs::config_dir()`) under
    /// an `rdlp` subdirectory, with `settings.json` as the filename.
    /// Falls back to `./settings.json` if the config directory cannot
    /// be determined.
    ///
    /// # Returns
    ///
    /// The [`PathBuf`] to the settings file.
    #[must_use]
    pub fn settings_path() -> PathBuf {
        dirs::config_dir()
            .map(|p| p.join("rdlp").join("settings.json"))
            .unwrap_or_else(|| PathBuf::from("settings.json"))
    }

    /// Load settings from disk, falling back to defaults.
    ///
    /// Reads the settings file at [`Self::settings_path()`]. If the
    /// file is missing or contains invalid JSON, returns
    /// [`Default::default()`] instead.
    ///
    /// # Returns
    ///
    /// The loaded [`AppSettings`], or defaults if loading fails.
    #[must_use]
    pub fn load() -> Self {
        let path = Self::settings_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(settings) => {
                    info!("Loaded settings from {}", path.display());
                    settings
                }
                Err(e) => {
                    warn!(
                        "Failed to parse settings at {}: {e}, using defaults",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(_) => {
                info!("No settings file at {}, using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Persist the current settings to disk as JSON.
    ///
    /// Creates parent directories if they do not exist. Writes the
    /// settings to [`Self::settings_path()`].
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file writing fails.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let path = Self::settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
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
    fn test_settings_path_is_not_empty() {
        let path = AppSettings::settings_path();
        assert!(
            !path.as_os_str().is_empty(),
            "settings_path should not be empty"
        );
        assert!(
            path.ends_with("settings.json"),
            "settings_path should end with settings.json"
        );
    }

    #[test]
    fn test_load_returns_defaults_when_no_file() {
        // `load()` should not panic and should return defaults when
        // the file does not exist (which it won't in a test env using
        // the global path).
        let settings = AppSettings::load();
        assert!(
            !settings.output_dir.as_os_str().is_empty(),
            "loaded settings should have a non-empty output_dir"
        );
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
