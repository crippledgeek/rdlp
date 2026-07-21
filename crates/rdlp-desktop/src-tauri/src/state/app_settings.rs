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
    /// Total download timeout in seconds. `None` uses default (3600).
    /// Validated by `AppSettings::validate()` (range mirrors `rdlp_types`
    /// `Config::validate()`): must be 1..=86400.
    #[serde(default)]
    pub download_timeout: Option<u64>,
    /// Merge (mux/concat) timeout in seconds. `None` uses default (1800).
    /// Validated by `AppSettings::validate()` (range mirrors `rdlp_types`
    /// `Config::validate()`): must be 1..=86400.
    #[serde(default)]
    pub merge_timeout: Option<u64>,
    /// Download subtitles by default.
    #[serde(default)]
    pub write_subtitles: bool,
    /// Download auto-generated subtitles by default.
    #[serde(default)]
    pub write_auto_subtitles: bool,
    /// Fail the download if requested subtitles are unavailable.
    #[serde(default)]
    pub strict_subs: bool,
    /// Validate subtitle URLs before downloading.
    #[serde(default)]
    pub verify_sub_urls: bool,
    /// Retry failed subtitle downloads.
    #[serde(default)]
    pub retry_subs: bool,
    /// Number of concurrent fragment/chunk downloads. `None` uses default (8).
    /// Validated by [`AppSettings::validate_security`]: must be 1..=64.
    #[serde(default)]
    pub concurrent_fragments: Option<u32>,
    /// Download buffer size in **bytes**. `None` uses default (2 MiB).
    /// Validated by [`AppSettings::validate_security`]: must be 1..=1 GiB.
    ///
    /// The Settings UI presents this in MiB; bytes remain the stored truth.
    #[serde(default)]
    pub buffer_size: Option<u64>,
    /// Minimum file size in **bytes** before parallel chunked download is used.
    /// `None` uses default (10 MiB). Validated: must be 1..=1 GiB (mirrors
    /// `rdlp_types::Config::validate()`).
    #[serde(default)]
    pub parallel_threshold: Option<u64>,
    /// HLS HEAD-probe timeout in seconds. `None` uses default (5).
    /// Validated by [`AppSettings::validate_security`]: must be 1..=300.
    #[serde(default)]
    pub hls_head_probe_timeout: Option<u64>,
}

/// Upper bound on `parse_and_validate`'s reset-and-revalidate loop.
///
/// `reset_invalid_field`'s `OutOfRange` arm matches exactly 9 numeric fields
/// (`socket_timeout`, `read_timeout`, `pool_idle_timeout`, `download_timeout`,
/// `merge_timeout`, `concurrent_fragments`, `buffer_size`, `parallel_threshold`,
/// `hls_head_probe_timeout`); each loop iteration resets a distinct field to
/// `None` (which is always in-range), so a legitimately hand-edited file can
/// require at most 9 iterations to converge. This bound exists so the loop's
/// termination is structural rather than incidental on `AppSettings::default()`
/// happening to validate — see the `debug_assert!` in `parse_and_validate` and
/// Finding 5 in `task-9-report.md`.
const MAX_RESET_ITERATIONS: u32 = 9;

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
    /// Reads the settings file at [`Self::settings_path()`]. If the file is
    /// missing or contains invalid JSON, returns [`Default::default()`]. If it
    /// parses but one or more fields fail security validation (see
    /// [`Self::validate_security`]), only the offending field(s) reset to
    /// their default (`None`) — every other field is preserved. See
    /// [`Self::parse_and_validate`] for why: a whole-record fallback would
    /// clobber the user's `output_dir`, cookies, proxy, and every other
    /// setting over one out-of-range field.
    ///
    /// A hand-edited `settings.json` bypasses the frontend's zod schema, so this
    /// is the last point that can reject an out-of-range or unsafe value before
    /// it reaches the rest of the application (e.g. an allocation sized directly
    /// from `buffer_size`). This is the only enforcement point on the *load*
    /// path; the *save* path enforces the same check in `update_settings`.
    ///
    /// # Returns
    ///
    /// The loaded [`AppSettings`], with any invalid field(s) reset to their
    /// default, or full defaults if the file is missing/unparseable.
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
            |contents| Self::parse_and_validate(&contents, &path),
        )
    }

    /// Parse `contents` as JSON, then validate the result. A parse error falls back to
    /// [`Default::default()`] (there's no partial record to preserve). A validation
    /// failure resets ONLY the offending field to `None` ("inherit default" — see
    /// [`Self::reset_invalid_field`]) and re-validates, looping until the record is
    /// fully valid; every other field survives untouched.
    ///
    /// Extracted from [`Self::load()`] so the parse-then-validate logic is testable
    /// without depending on [`Self::settings_path()`], which resolves to a fixed,
    /// non-configurable per-process path.
    fn parse_and_validate(contents: &str, path: &std::path::Path) -> Self {
        // `AppSettings::default()` is the fail-safe fallback both inside this loop
        // (see `reset_invalid_field`'s `other =>` arm) and if `MAX_RESET_ITERATIONS`
        // is ever exhausted below. That fallback only terminates the loop because
        // the default record happens to be in-range; this assertion makes that
        // assumption explicit and catches a future default drifting out-of-range
        // before it can turn into an infinite loop in a release build.
        debug_assert!(
            Self::default().validate_security().is_ok(),
            "AppSettings::default() must always pass validate_security() — it is the \
             fail-safe fallback the reset loop below relies on to terminate"
        );
        match serde_json::from_str::<Self>(contents) {
            Ok(mut settings) => {
                // Loop resetting only the offending field, because a hand-edited
                // settings.json can carry more than one out-of-range value and
                // `validate_security()` reports (and this loop resets) one at a time.
                // A field reset to `None` means "inherit the default" everywhere else
                // in this design, so it is the exact remedy — never fall back to
                // `Self::default()`, which would also discard every OTHER field the
                // user had set (output_dir, cookies, proxy, format defaults, ...).
                //
                // Bounded by `MAX_RESET_ITERATIONS` so termination is structural, not
                // merely incidental on `reset_invalid_field`'s fail-safe reaching a
                // valid record: without this bound, a future out-of-range default
                // field that is ALSO missing from `reset_invalid_field`'s match would
                // spin forever, since its `other =>` arm sets `*self = Self::default()`
                // and a still-invalid default would immediately fail validation again.
                let mut iterations = 0u32;
                while let Err(e) = settings.validate_security() {
                    if iterations >= MAX_RESET_ITERATIONS {
                        warn!(
                            "Settings at {} still failing validation after {MAX_RESET_ITERATIONS} \
                             reset attempts ({e}); falling back to full defaults",
                            path.display()
                        );
                        settings = Self::default();
                        break;
                    }
                    warn!(
                        "Settings at {} failed validation ({e}); resetting that field to its default",
                        path.display()
                    );
                    settings.reset_invalid_field(&e);
                    iterations += 1;
                }
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
        }
    }

    /// Reset the single field named by `err` to its default (`None`/unset), leaving
    /// every other field untouched.
    ///
    /// Only [`SettingsValidationError::OutOfRange`] identifies a single numeric field;
    /// the other variants (`CookiesFileTraversal`, `InvalidProxy`) reset their
    /// respective non-numeric field to `None` directly, since they carry no `field`
    /// name to dispatch on.
    fn reset_invalid_field(&mut self, err: &SettingsValidationError) {
        match err {
            SettingsValidationError::CookiesFileTraversal => self.cookies_file = None,
            SettingsValidationError::InvalidProxy(_) => self.proxy = None,
            SettingsValidationError::OutOfRange { field, .. } => match *field {
                "socket_timeout" => self.socket_timeout = None,
                "read_timeout" => self.read_timeout = None,
                "pool_idle_timeout" => self.pool_idle_timeout = None,
                "download_timeout" => self.download_timeout = None,
                "merge_timeout" => self.merge_timeout = None,
                "concurrent_fragments" => self.concurrent_fragments = None,
                "buffer_size" => self.buffer_size = None,
                "parallel_threshold" => self.parallel_threshold = None,
                "hls_head_probe_timeout" => self.hls_head_probe_timeout = None,
                other => {
                    // Unreachable in practice (every `OutOfRange` field above is listed).
                    // Fail safe rather than looping forever on an unmatched field: fall
                    // back to full defaults for this one pathological case only.
                    warn!(
                        "Unknown out-of-range field '{other}' during settings reset; \
                         resetting the whole record to defaults as a fail-safe"
                    );
                    *self = Self::default();
                }
            },
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
            download_timeout: None,
            merge_timeout: None,
            write_subtitles: false,
            write_auto_subtitles: false,
            strict_subs: false,
            verify_sub_urls: false,
            retry_subs: false,
            concurrent_fragments: None,
            buffer_size: None,
            parallel_threshold: None,
            hls_head_probe_timeout: None,
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
    /// A numeric field is outside its allowed range.
    OutOfRange {
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
            Self::OutOfRange { field, reason } => {
                write!(f, "{field}: {reason}")
            }
        }
    }
}

impl std::error::Error for SettingsValidationError {}

/// Upper bound for byte-valued settings, in bytes (1 GiB).
///
/// Mirrors `rdlp_types::Config::validate()`'s `parallel_threshold` and `buffer_size`
/// ceilings rather than introducing a second magic number. `Config::validate()` is not
/// called on the desktop path (see `commands::download`), so `validate_security` is this
/// field's enforcement point on that path — run on both `AppSettings::load()` and the
/// `update_settings` save command, so neither a hand-edited `settings.json` nor a save
/// from the UI can carry an out-of-range value. No cross-crate test enforces the two
/// literals staying in sync, so a future change to either ceiling must be mirrored
/// manually in the other crate.
const MAX_BYTE_SETTING: u64 = 1024 * 1024 * 1024;

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
            return Err(SettingsValidationError::OutOfRange {
                field: "socket_timeout",
                reason: "must be 1..=300 seconds",
            });
        }
        if let Some(t) = self.read_timeout
            && !(1..=600).contains(&t)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "read_timeout",
                reason: "must be 1..=600 seconds",
            });
        }
        if let Some(t) = self.pool_idle_timeout
            && t > 3600
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "pool_idle_timeout",
                reason: "must be 0..=3600 seconds (0 = disabled)",
            });
        }
        if let Some(t) = self.download_timeout
            && !(1..=86400).contains(&t)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "download_timeout",
                reason: "must be 1..=86400 seconds",
            });
        }
        if let Some(t) = self.merge_timeout
            && !(1..=86400).contains(&t)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "merge_timeout",
                reason: "must be 1..=86400 seconds",
            });
        }
        if let Some(n) = self.concurrent_fragments
            && !(1..=64).contains(&n)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "concurrent_fragments",
                reason: "must be 1..=64 (caps peak transient memory under parallel fetch)",
            });
        }
        if let Some(n) = self.buffer_size
            && !(1..=MAX_BYTE_SETTING).contains(&n)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "buffer_size",
                reason: "must be 1..=1_073_741_824 bytes (1 GiB)",
            });
        }
        if let Some(n) = self.parallel_threshold
            && !(1..=MAX_BYTE_SETTING).contains(&n)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "parallel_threshold",
                reason: "must be 1..=1_073_741_824 bytes (1 GiB)",
            });
        }
        if let Some(t) = self.hls_head_probe_timeout
            && !(1..=300).contains(&t)
        {
            return Err(SettingsValidationError::OutOfRange {
                field: "hls_head_probe_timeout",
                reason: "must be 1..=300 seconds",
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
            download_timeout: None,
            merge_timeout: None,
            write_subtitles: true,
            write_auto_subtitles: true,
            strict_subs: true,
            verify_sub_urls: true,
            retry_subs: true,
            concurrent_fragments: None,
            buffer_size: None,
            parallel_threshold: None,
            hls_head_probe_timeout: None,
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
        assert!(restored.write_subtitles);
        assert!(restored.write_auto_subtitles);
        assert!(restored.strict_subs);
        assert!(restored.verify_sub_urls);
        assert!(restored.retry_subs);
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

    #[test]
    fn legacy_json_without_subtitle_flags_defaults_false() {
        // Minimal legacy settings.json predating the 5 subtitle flags. Mirrors the
        // fixture shape in `test_legacy_settings_json_without_timeout_fields_loads`
        // (the fields below are the only ones without `#[serde(default)]`).
        let json = r#"{"output_dir":"/tmp","embed_thumbnail":true,"embed_metadata":false,"verbose":false,"default_subtitle_langs":[]}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert!(
            !s.write_subtitles
                && !s.write_auto_subtitles
                && !s.strict_subs
                && !s.verify_sub_urls
                && !s.retry_subs
        );
    }

    #[test]
    fn test_new_throughput_fields_default_to_none() {
        let s = AppSettings::default();
        assert!(s.concurrent_fragments.is_none());
        assert!(s.buffer_size.is_none());
        assert!(s.parallel_threshold.is_none());
        assert!(s.hls_head_probe_timeout.is_none());
    }

    #[test]
    fn legacy_json_without_throughput_fields_defaults_none() {
        // Minimal legacy settings.json predating the throughput fields. `None`, not
        // `Some(0)` — a zero would be a valid-looking value that disables chunking.
        let json = r#"{"output_dir":"/tmp","embed_thumbnail":true,"embed_metadata":false,"verbose":false,"default_subtitle_langs":[]}"#;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert!(
            s.concurrent_fragments.is_none()
                && s.buffer_size.is_none()
                && s.parallel_threshold.is_none()
                && s.hls_head_probe_timeout.is_none()
        );
    }

    #[test]
    fn test_throughput_fields_round_trip_json() {
        let s = AppSettings {
            concurrent_fragments: Some(16),
            buffer_size: Some(4 * 1024 * 1024),
            parallel_threshold: Some(20 * 1024 * 1024),
            hls_head_probe_timeout: Some(10),
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back: AppSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.concurrent_fragments, Some(16));
        assert_eq!(back.buffer_size, Some(4 * 1024 * 1024));
        assert_eq!(back.parallel_threshold, Some(20 * 1024 * 1024));
        assert_eq!(back.hls_head_probe_timeout, Some(10));
    }

    // --- Boundary pairs: each pair is one a `>=`-for-`>` slip cannot both pass. ---

    #[test]
    fn test_concurrent_fragments_boundary() {
        let ok = AppSettings {
            concurrent_fragments: Some(64),
            ..AppSettings::default()
        };
        assert!(ok.validate_security().is_ok(), "64 is the documented max");
        let bad = AppSettings {
            concurrent_fragments: Some(65),
            ..AppSettings::default()
        };
        let err = bad.validate_security().expect_err("65 must be rejected");
        assert!(err.to_string().contains("concurrent_fragments"));
        let zero = AppSettings {
            concurrent_fragments: Some(0),
            ..AppSettings::default()
        };
        assert!(
            zero.validate_security().is_err(),
            "0 fragments is meaningless"
        );
    }

    #[test]
    fn test_byte_setting_upper_boundary() {
        let ok = AppSettings {
            buffer_size: Some(1024 * 1024 * 1024),
            parallel_threshold: Some(1024 * 1024 * 1024),
            ..AppSettings::default()
        };
        assert!(
            ok.validate_security().is_ok(),
            "1 GiB is the documented max"
        );

        let bad_buf = AppSettings {
            buffer_size: Some(1024 * 1024 * 1024 + 1),
            ..AppSettings::default()
        };
        let err = bad_buf
            .validate_security()
            .expect_err("over 1 GiB must be rejected");
        assert!(err.to_string().contains("buffer_size"));

        let bad_thr = AppSettings {
            parallel_threshold: Some(1024 * 1024 * 1024 + 1),
            ..AppSettings::default()
        };
        assert!(bad_thr.validate_security().is_err());
    }

    #[test]
    fn test_byte_setting_lower_boundary() {
        let ok = AppSettings {
            buffer_size: Some(1),
            ..AppSettings::default()
        };
        assert!(ok.validate_security().is_ok(), "1 byte is in range");
        let bad = AppSettings {
            buffer_size: Some(0),
            ..AppSettings::default()
        };
        assert!(
            bad.validate_security().is_err(),
            "0 is rejected by Config::validate too"
        );
    }

    #[test]
    fn test_parallel_threshold_lower_boundary() {
        let ok = AppSettings {
            parallel_threshold: Some(1),
            ..AppSettings::default()
        };
        assert!(ok.validate_security().is_ok(), "1 byte is in range");
        let bad = AppSettings {
            parallel_threshold: Some(0),
            ..AppSettings::default()
        };
        let err = bad
            .validate_security()
            .expect_err("0 is rejected by Config::validate too");
        assert!(err.to_string().contains("parallel_threshold"));
    }

    /// The GUI floors these fields at 1 MiB, but validation must NOT — a hand-edited
    /// settings.json carrying a legitimate sub-MiB value has to survive. Guards against
    /// someone hardening validation to match the `NumberField`'s `minValue`.
    #[test]
    fn test_sub_mib_byte_values_are_accepted() {
        let s = AppSettings {
            buffer_size: Some(500_000),
            parallel_threshold: Some(500_000),
            ..AppSettings::default()
        };
        assert!(s.validate_security().is_ok(), "sub-MiB must remain valid");
    }

    /// Finding A regression guard: `AppSettings::load()` used to deserialize straight
    /// into state and never call `validate_security()`, so a hand-edited
    /// `settings.json` with `{"buffer_size": 100000000000}` (100 GB, far above the
    /// 1 GiB ceiling) would flow unchecked into `build_network_options` /
    /// `BufWriter::with_capacity`. This exercises the exact logic `load()` runs
    /// (`parse_and_validate`) so it fails against the pre-fix code (which had no
    /// `validate_security()` call in that path) and passes once the fix is applied.
    ///
    /// Updated for Finding 2: `load()` now resets ONLY the offending field, not the
    /// whole record — see `test_valid_neighbouring_field_survives_invalid_sibling`
    /// for the field-preservation half of this contract.
    #[test]
    fn test_out_of_range_settings_json_resets_only_that_field() {
        let json = r#"{
            "output_dir": "/tmp",
            "embed_thumbnail": true,
            "embed_metadata": false,
            "verbose": false,
            "default_subtitle_langs": [],
            "buffer_size": 100000000000
        }"#;
        let settings = AppSettings::parse_and_validate(json, std::path::Path::new("test.json"));
        assert_eq!(
            settings.buffer_size, None,
            "out-of-range buffer_size must NOT survive load — it resets to None (inherit default)"
        );
    }

    /// Finding 2 regression guard: a bad field alongside a good field must not wipe the
    /// good one. Pre-fix, `parse_and_validate` fell back to `Self::default()` on ANY
    /// validation failure, which would also have discarded `output_dir` here.
    #[test]
    fn test_valid_neighbouring_field_survives_invalid_sibling() {
        let json = r#"{
            "output_dir": "/home/user/Videos",
            "embed_thumbnail": true,
            "embed_metadata": false,
            "verbose": false,
            "default_subtitle_langs": [],
            "buffer_size": 100000000000
        }"#;
        let settings = AppSettings::parse_and_validate(json, std::path::Path::new("test.json"));
        assert_eq!(
            settings.buffer_size, None,
            "the offending field resets to None"
        );
        assert_eq!(
            settings.output_dir,
            PathBuf::from("/home/user/Videos"),
            "a valid neighbouring field must survive the reset of a different bad field"
        );
    }

    /// Multiple simultaneously-bad fields must each reset independently — the loop in
    /// `parse_and_validate` must not stop after fixing only the first one.
    #[test]
    fn test_multiple_invalid_fields_each_reset_independently() {
        let json = r#"{
            "output_dir": "/home/user/Videos",
            "embed_thumbnail": true,
            "embed_metadata": false,
            "verbose": false,
            "default_subtitle_langs": [],
            "buffer_size": 100000000000,
            "socket_timeout": 9999,
            "concurrent_fragments": 999
        }"#;
        let settings = AppSettings::parse_and_validate(json, std::path::Path::new("test.json"));
        assert_eq!(settings.buffer_size, None);
        assert_eq!(settings.socket_timeout, None);
        assert_eq!(settings.concurrent_fragments, None);
        assert_eq!(settings.output_dir, PathBuf::from("/home/user/Videos"));
    }

    /// A settings.json with a valid `buffer_size` loads unchanged (not silently
    /// replaced by defaults) — the counterpart to the out-of-range test above.
    #[test]
    fn test_valid_settings_json_loads_unchanged() {
        let json = r#"{
            "output_dir": "/tmp",
            "embed_thumbnail": true,
            "embed_metadata": false,
            "verbose": false,
            "default_subtitle_langs": [],
            "buffer_size": 4194304
        }"#;
        let settings = AppSettings::parse_and_validate(json, std::path::Path::new("test.json"));
        assert_eq!(settings.buffer_size, Some(4_194_304));
        assert_eq!(settings.output_dir, PathBuf::from("/tmp"));
    }

    #[test]
    fn test_hls_head_probe_timeout_boundary() {
        let ok = AppSettings {
            hls_head_probe_timeout: Some(300),
            ..AppSettings::default()
        };
        assert!(ok.validate_security().is_ok());
        let bad = AppSettings {
            hls_head_probe_timeout: Some(301),
            ..AppSettings::default()
        };
        let err = bad.validate_security().expect_err("301 must be rejected");
        assert!(err.to_string().contains("hls_head_probe_timeout"));
        let zero = AppSettings {
            hls_head_probe_timeout: Some(0),
            ..AppSettings::default()
        };
        assert!(zero.validate_security().is_err());
    }

    /// Finding 5 boundary guard: exactly `MAX_RESET_ITERATIONS` (9) simultaneously
    /// out-of-range fields — one per `OutOfRange` arm `reset_invalid_field` matches —
    /// MUST resolve entirely via the per-field reset path, not the iteration-cap
    /// fail-safe. `validate_security` reports fields in the same fixed order
    /// `reset_invalid_field` matches them, so this pins the exact boundary the loop
    /// bound must accommodate: 9 legitimate iterations is normal, not exhaustion.
    #[test]
    fn test_max_simultaneous_out_of_range_fields_resolves_without_full_reset() {
        let json = r#"{
            "output_dir": "/home/user/Videos",
            "embed_thumbnail": true,
            "embed_metadata": false,
            "verbose": false,
            "default_subtitle_langs": [],
            "socket_timeout": 0,
            "read_timeout": 0,
            "pool_idle_timeout": 99999,
            "download_timeout": 0,
            "merge_timeout": 0,
            "concurrent_fragments": 0,
            "buffer_size": 0,
            "parallel_threshold": 0,
            "hls_head_probe_timeout": 0
        }"#;
        let settings = AppSettings::parse_and_validate(json, std::path::Path::new("test.json"));
        assert!(settings.validate_security().is_ok());
        assert_eq!(settings.socket_timeout, None);
        assert_eq!(settings.read_timeout, None);
        assert_eq!(settings.pool_idle_timeout, None);
        assert_eq!(settings.download_timeout, None);
        assert_eq!(settings.merge_timeout, None);
        assert_eq!(settings.concurrent_fragments, None);
        assert_eq!(settings.buffer_size, None);
        assert_eq!(settings.parallel_threshold, None);
        assert_eq!(settings.hls_head_probe_timeout, None);
        // The per-field reset path preserves untouched fields; a fallback to
        // `Self::default()` would have discarded this custom `output_dir` too.
        assert_eq!(
            settings.output_dir,
            PathBuf::from("/home/user/Videos"),
            "9 legitimate iterations must resolve per-field, not trip the \
             MAX_RESET_ITERATIONS fail-safe (which would also reset output_dir)"
        );
    }

    /// Finding 5 regression guard: `reset_invalid_field`'s `other =>` fail-safe arm
    /// (an `OutOfRange` field name absent from the match — unreachable via the
    /// public API since `validate_security` only ever reports the 9 matched names,
    /// but defensive against a future field being added to one side and not the
    /// other) must still terminate by falling back to a fully valid default record.
    #[test]
    fn test_reset_invalid_field_falls_back_to_defaults_for_unmatched_field_name() {
        let mut settings = AppSettings {
            output_dir: PathBuf::from("/custom/output"),
            verbose: true,
            ..AppSettings::default()
        };
        let err = SettingsValidationError::OutOfRange {
            field: "not_a_real_field",
            reason: "synthetic, for the fail-safe arm",
        };
        settings.reset_invalid_field(&err);
        assert!(
            settings.validate_security().is_ok(),
            "the fail-safe fallback must itself be valid, or the caller's while-loop \
             would spin until MAX_RESET_ITERATIONS"
        );
        assert_eq!(
            settings.output_dir,
            AppSettings::default().output_dir,
            "the fail-safe arm discards the WHOLE record (unlike the matched arms, \
             which reset only their own field)"
        );
    }
}
