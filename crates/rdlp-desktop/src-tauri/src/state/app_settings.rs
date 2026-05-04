//! Persistent application settings.
//!
//! Stores user preferences such as the download directory, default
//! post-processing options, and subtitle configuration. Settings are
//! serializable for persistence between sessions.

use std::path::{Component, PathBuf};

use log::{info, warn};
use rdlp_types::{AudioFormat, BrowserType, ContainerFormat, SubtitleFormat};
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
#[allow(clippy::struct_excessive_bools)]
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
    /// Enable audio normalization (peak mode unless loudnorm is set).
    #[serde(default)]
    pub normalize_audio: bool,
    /// Use EBU R128 loudnorm normalization (implies `normalize_audio`).
    #[serde(default)]
    pub loudnorm: bool,
    /// Loudnorm preset name ("streaming", "broadcast", "loud").
    #[serde(default)]
    pub loudnorm_preset: Option<String>,
    /// Custom target integrated loudness in LUFS (overrides preset).
    #[serde(default)]
    pub loudnorm_target_i: Option<f64>,
    /// Custom target true peak in dBTP (overrides preset).
    #[serde(default)]
    pub loudnorm_target_tp: Option<f64>,
    /// Custom target loudness range in LU (overrides preset).
    #[serde(default)]
    pub loudnorm_target_lra: Option<f64>,
    /// Force dynamic (per-frame compression) mode in loudnorm pass 2.
    #[serde(default)]
    pub loudnorm_dynamic: bool,
    /// Prepend a mild acompressor before loudnorm to tame extreme peaks.
    #[serde(default)]
    pub loudnorm_precompress: bool,
    /// Enable limiter-boost fallback for over-compressed content.
    #[serde(default)]
    pub normalize_boost: bool,
    /// Gain in dB for limiter-boost fallback.
    #[serde(default)]
    pub normalize_boost_db: Option<f64>,
    /// Write (keep) downloaded thumbnail as a separate file alongside the output.
    #[serde(default)]
    pub write_thumbnail: bool,
    /// Gain in dB applied on top of normalization (peak or loudnorm).
    #[serde(default)]
    pub audio_gain_target: Option<f64>,
    /// Browser to extract cookies from for age-gated content.
    #[serde(default)]
    pub cookies_from_browser: Option<BrowserType>,
    /// Path to a Netscape-format cookies file.
    ///
    /// MUST NOT contain `..` path components (validated on save).
    #[serde(default)]
    pub cookies_file: Option<PathBuf>,
    /// HTTP/SOCKS proxy URL (e.g. `"http://proxy:3128"` or `"socks5://proxy:1080"`).
    ///
    /// Validated via `rdlp_security::validate_proxy_url()` on save.
    #[serde(default)]
    pub proxy: Option<String>,
    /// Download rate limit expressed as a string (e.g. `"500K"`, `"2M"`).
    #[serde(default)]
    pub rate_limit: Option<String>,
    /// Output filename template (yt-dlp `%(field)s` syntax).
    #[serde(default)]
    pub output_template: Option<String>,
    /// Embed subtitles into the output container.
    #[serde(default)]
    pub embed_subtitles: bool,
    /// Connect/handshake timeout in seconds. `None` uses default (30).
    /// Validated post-load by `Config::validate()`: must be 1..=300.
    #[serde(default)]
    pub socket_timeout: Option<u64>,
    /// Per-read idle timeout in seconds. `None` uses default.
    /// Validated post-load by `Config::validate()`: must be 1..=600.
    #[serde(default)]
    pub read_timeout: Option<u64>,
    /// Idle keep-alive socket eviction timeout in seconds. `None` uses default;
    /// `Some(0)` disables eviction (sentinel translated downstream by
    /// `HttpClientConfig::from_rdlp_config`).
    /// Validated post-load by `Config::validate()`: must be 0..=3600.
    #[serde(default)]
    pub pool_idle_timeout: Option<u64>,
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
        dirs::config_dir().map_or_else(
            || PathBuf::from("settings.json"),
            |p| p.join("rdlp").join("settings.json"),
        )
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
        // Safe: invoked from Tauri sync startup (AppState::new) before any async runtime is active.
        #[allow(clippy::disallowed_methods)]
        let contents = std::fs::read_to_string(&path);
        contents.map_or_else(
            |_| {
                info!("No settings file at {}, using defaults", path.display());
                Self::default()
            },
            |contents| match serde_json::from_str(&contents) {
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
        )
    }

    /// Persist the current settings to disk as JSON.
    ///
    /// Creates parent directories if they do not exist. Writes the
    /// settings to [`Self::settings_path()`].
    ///
    /// # Errors
    ///
    /// Returns an error if directory creation or file writing fails.
    // Safe: invoked from Tauri command handlers that bridge to the frontend; settings are small
    // (a few KB of JSON) and persistence occurs on user interaction, not from a hot async loop.
    // If this is ever called from a heavily contended async context, migrate to tokio::fs.
    #[allow(clippy::disallowed_methods)]
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
            normalize_audio: false,
            loudnorm: false,
            loudnorm_preset: None,
            loudnorm_target_i: None,
            loudnorm_target_tp: None,
            loudnorm_target_lra: None,
            loudnorm_dynamic: false,
            loudnorm_precompress: false,
            normalize_boost: false,
            normalize_boost_db: None,
            write_thumbnail: false,
            audio_gain_target: None,
            cookies_from_browser: None,
            cookies_file: None,
            proxy: None,
            rate_limit: None,
            output_template: None,
            embed_subtitles: false,
            socket_timeout: None,
            read_timeout: None,
            pool_idle_timeout: None,
        }
    }
}

/// Errors that can occur when validating settings before saving.
#[derive(Debug)]
pub enum SettingsValidationError {
    /// `cookies_file` path contains a `..` component.
    CookiesFileTraversal,
    /// `proxy` URL failed security validation.
    InvalidProxy(String),
    /// A timeout field is outside its allowed range.
    TimeoutOutOfRange {
        /// Name of the offending field (e.g. `"socket_timeout"`).
        field: &'static str,
        /// Human-readable description of the allowed range.
        reason: &'static str,
    },
}

impl std::fmt::Display for SettingsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CookiesFileTraversal => {
                f.write_str("cookies_file path must not contain '..' components")
            }
            Self::InvalidProxy(msg) => write!(f, "invalid proxy URL: {msg}"),
            Self::TimeoutOutOfRange { field, reason } => {
                write!(f, "{field}: {reason}")
            }
        }
    }
}

impl std::error::Error for SettingsValidationError {}

impl AppSettings {
    /// Validate security-sensitive fields before persisting.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsValidationError`] if any field fails validation.
    pub fn validate_security(&self) -> Result<(), SettingsValidationError> {
        // Reject cookies_file paths with `..` components (path traversal).
        if let Some(path) = &self.cookies_file {
            let has_dotdot = path.components().any(|c| matches!(c, Component::ParentDir));
            if has_dotdot {
                return Err(SettingsValidationError::CookiesFileTraversal);
            }
        }

        // Validate proxy URL if set.
        if let Some(proxy) = &self.proxy {
            rdlp_security::validate_proxy_url(proxy)
                .map_err(|e| SettingsValidationError::InvalidProxy(e.to_string()))?;
        }

        // HTTP timeout ranges — mirror `rdlp_types::Config::validate()` so a
        // hand-edited settings.json can't bypass the frontend's zod parsing.
        if let Some(t) = self.socket_timeout
            && !(1..=300).contains(&t)
        {
            return Err(SettingsValidationError::TimeoutOutOfRange {
                field: "socket_timeout",
                reason: "must be 1..=300 seconds",
            });
        }
        if let Some(t) = self.read_timeout
            && !(1..=600).contains(&t)
        {
            return Err(SettingsValidationError::TimeoutOutOfRange {
                field: "read_timeout",
                reason: "must be 1..=600 seconds",
            });
        }
        if let Some(t) = self.pool_idle_timeout
            && t > 3600
        {
            return Err(SettingsValidationError::TimeoutOutOfRange {
                field: "pool_idle_timeout",
                reason: "must be 0..=3600 seconds (0 = disabled)",
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_security_rejects_dotdot_cookies_path() {
        let settings = AppSettings {
            cookies_file: Some(PathBuf::from("/tmp/../etc/passwd")),
            ..AppSettings::default()
        };
        assert!(settings.validate_security().is_err());
    }

    #[test]
    fn test_validate_security_accepts_clean_cookies_path() {
        let settings = AppSettings {
            cookies_file: Some(PathBuf::from("/tmp/cookies.txt")),
            ..AppSettings::default()
        };
        assert!(settings.validate_security().is_ok());
    }

    #[test]
    fn test_validate_security_rejects_socket_timeout_zero() {
        let s = AppSettings {
            socket_timeout: Some(0),
            ..AppSettings::default()
        };
        let err = s.validate_security().expect_err("must reject");
        assert!(err.to_string().contains("socket_timeout"));
    }

    #[test]
    fn test_validate_security_rejects_socket_timeout_above_max() {
        let s = AppSettings {
            socket_timeout: Some(301),
            ..AppSettings::default()
        };
        assert!(s.validate_security().is_err());
    }

    #[test]
    fn test_validate_security_rejects_read_timeout_above_max() {
        let s = AppSettings {
            read_timeout: Some(601),
            ..AppSettings::default()
        };
        let err = s.validate_security().expect_err("must reject");
        assert!(err.to_string().contains("read_timeout"));
    }

    #[test]
    fn test_validate_security_accepts_pool_idle_timeout_zero_sentinel() {
        let s = AppSettings {
            pool_idle_timeout: Some(0),
            ..AppSettings::default()
        };
        assert!(s.validate_security().is_ok(), "0 is the disable sentinel");
    }

    #[test]
    fn test_validate_security_rejects_pool_idle_timeout_above_max() {
        let s = AppSettings {
            pool_idle_timeout: Some(3601),
            ..AppSettings::default()
        };
        let err = s.validate_security().expect_err("must reject");
        assert!(err.to_string().contains("pool_idle_timeout"));
    }

    #[test]
    fn test_validate_security_rejects_private_proxy() {
        let settings = AppSettings {
            proxy: Some("http://192.168.1.1:3128".to_owned()),
            ..AppSettings::default()
        };
        assert!(settings.validate_security().is_err());
    }

    #[test]
    fn test_validate_security_rejects_invalid_proxy_scheme() {
        let settings = AppSettings {
            proxy: Some("ftp://proxy.example.com:21".to_owned()),
            ..AppSettings::default()
        };
        assert!(settings.validate_security().is_err());
    }

    #[test]
    fn test_validate_security_accepts_valid_proxy() {
        let settings = AppSettings {
            proxy: Some("socks5://proxy.example.com:1080".to_owned()),
            ..AppSettings::default()
        };
        assert!(settings.validate_security().is_ok());
    }

    #[test]
    fn test_validate_security_default_is_ok() {
        assert!(AppSettings::default().validate_security().is_ok());
    }

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
        assert!(!settings.normalize_audio);
        assert!(!settings.loudnorm);
        assert!(settings.loudnorm_preset.is_none());
        assert!(settings.loudnorm_target_i.is_none());
        assert!(settings.loudnorm_target_tp.is_none());
        assert!(settings.loudnorm_target_lra.is_none());
        assert!(!settings.loudnorm_dynamic);
        assert!(!settings.loudnorm_precompress);
        assert!(!settings.normalize_boost);
        assert!(settings.normalize_boost_db.is_none());
        assert!(!settings.write_thumbnail);
        assert!(settings.audio_gain_target.is_none());
        assert!(settings.cookies_from_browser.is_none());
        assert!(settings.cookies_file.is_none());
        assert!(settings.proxy.is_none());
        assert!(settings.rate_limit.is_none());
        assert!(settings.output_template.is_none());
        assert!(!settings.embed_subtitles);
        assert!(settings.socket_timeout.is_none());
        assert!(settings.read_timeout.is_none());
        assert!(settings.pool_idle_timeout.is_none());
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

    /// Settings JSON that predates the normalization fields (i.e. produced
    /// by an older version of the application) MUST deserialize without
    /// error, with all normalization fields falling back to their defaults.
    #[test]
    fn test_load_legacy_settings_without_normalization_fields() {
        // Simulate a settings.json from before normalization was added.
        let legacy_json = r#"{
            "output_dir": "/tmp/downloads",
            "default_remux": null,
            "default_extract_audio": null,
            "default_subtitle_format": null,
            "default_subtitle_langs": [],
            "embed_thumbnail": true,
            "embed_metadata": false,
            "verbose": false,
            "default_search_provider": null
        }"#;

        let settings: AppSettings = serde_json::from_str(legacy_json)
            .expect("legacy JSON without normalization fields should deserialize");

        // All normalization fields should default to false / None.
        assert!(!settings.normalize_audio);
        assert!(!settings.loudnorm);
        assert!(settings.loudnorm_preset.is_none());
        assert!(settings.loudnorm_target_i.is_none());
        assert!(settings.loudnorm_target_tp.is_none());
        assert!(settings.loudnorm_target_lra.is_none());
        assert!(!settings.loudnorm_dynamic);
        assert!(!settings.loudnorm_precompress);
        assert!(!settings.normalize_boost);
        assert!(settings.normalize_boost_db.is_none());

        // New fields should also default to false / None.
        assert!(!settings.write_thumbnail);
        assert!(settings.audio_gain_target.is_none());
        assert!(settings.cookies_from_browser.is_none());
        assert!(settings.cookies_file.is_none());
        assert!(settings.proxy.is_none());
        assert!(settings.rate_limit.is_none());
        assert!(settings.output_template.is_none());
        assert!(!settings.embed_subtitles);

        // Pre-existing fields should be preserved.
        assert_eq!(
            settings.output_dir,
            std::path::PathBuf::from("/tmp/downloads")
        );
        assert!(settings.embed_thumbnail);
        assert!(!settings.embed_metadata);
        assert!(!settings.verbose);
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
            normalize_audio: true,
            loudnorm: true,
            loudnorm_preset: Some("streaming".to_owned()),
            loudnorm_target_i: Some(-14.0),
            loudnorm_target_tp: Some(-1.0),
            loudnorm_target_lra: Some(11.0),
            loudnorm_dynamic: true,
            loudnorm_precompress: true,
            normalize_boost: true,
            normalize_boost_db: Some(8.0),
            write_thumbnail: true,
            audio_gain_target: Some(3.0),
            cookies_from_browser: Some(BrowserType::Firefox),
            cookies_file: Some(PathBuf::from("/tmp/cookies.txt")),
            proxy: Some("http://proxy.example.com:3128".to_owned()),
            rate_limit: Some("500K".to_owned()),
            output_template: Some("%(title)s.%(ext)s".to_owned()),
            embed_subtitles: true,
            socket_timeout: None,
            read_timeout: None,
            pool_idle_timeout: None,
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
        assert!(restored.normalize_audio);
        assert!(restored.loudnorm);
        assert_eq!(restored.loudnorm_preset.as_deref(), Some("streaming"));
        assert_eq!(restored.loudnorm_target_i, Some(-14.0));
        assert_eq!(restored.loudnorm_target_tp, Some(-1.0));
        assert_eq!(restored.loudnorm_target_lra, Some(11.0));
        assert!(restored.loudnorm_dynamic);
        assert!(restored.loudnorm_precompress);
        assert!(restored.normalize_boost);
        assert_eq!(restored.normalize_boost_db, Some(8.0));
        assert!(restored.write_thumbnail);
        assert_eq!(restored.audio_gain_target, Some(3.0));
        assert_eq!(restored.cookies_from_browser, Some(BrowserType::Firefox));
        assert_eq!(
            restored.cookies_file.as_deref(),
            Some(std::path::Path::new("/tmp/cookies.txt"))
        );
        assert_eq!(
            restored.proxy.as_deref(),
            Some("http://proxy.example.com:3128")
        );
        assert_eq!(restored.rate_limit.as_deref(), Some("500K"));
        assert_eq!(
            restored.output_template.as_deref(),
            Some("%(title)s.%(ext)s")
        );
        assert!(restored.embed_subtitles);
    }

    #[test]
    fn test_default_timeout_fields_are_none() {
        let s = AppSettings::default();
        assert!(s.socket_timeout.is_none());
        assert!(s.read_timeout.is_none());
        assert!(s.pool_idle_timeout.is_none());
    }

    #[test]
    fn test_timeout_fields_round_trip_json() {
        let s = AppSettings {
            socket_timeout: Some(45),
            read_timeout: Some(120),
            pool_idle_timeout: Some(0),
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.socket_timeout, Some(45));
        assert_eq!(back.read_timeout, Some(120));
        assert_eq!(back.pool_idle_timeout, Some(0));
    }

    #[test]
    fn test_legacy_settings_json_without_timeout_fields_loads() {
        // Older settings.json files won't have these keys; serde(default) must populate them as None.
        let json = r#"{"output_dir":".","embed_thumbnail":true,"embed_metadata":false,"verbose":false,"default_subtitle_langs":[]}"#;
        let s: AppSettings = serde_json::from_str(json).expect("must load legacy json");
        assert!(s.socket_timeout.is_none());
        assert!(s.read_timeout.is_none());
        assert!(s.pool_idle_timeout.is_none());
    }
}
