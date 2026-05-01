//! Application state managed by Tauri.
//!
//! Holds the [`RdlpClient`], download queue, and persistent settings.
//! The client is shared via `Arc` (it is already thread-safe and
//! cloneable), while mutable state is wrapped in `Arc<Mutex<_>>` for
//! safe concurrent access from async command handlers.

mod app_settings;
mod download_queue;

pub use app_settings::{AppSettings, SettingsValidationError};
pub use download_queue::{DownloadJob, DownloadQueue, JobStatus, SavedDownloadOptions};

use std::sync::{Arc, Mutex};

use rdlp_api::RdlpClient;
use rdlp_core::config_io;
use rdlp_postprocess::TempRegistry;
use rdlp_types::Config;

/// Top-level application state registered with [`tauri::Builder::manage`].
///
/// Created once at startup and shared across all IPC command handlers.
/// Access in Tauri commands via `State<'_, AppState>`.
pub struct AppState {
    /// The rdlp download engine client (thread-safe, cloneable).
    pub(crate) client: Arc<RdlpClient>,
    /// Active and completed downloads.
    pub(crate) queue: Arc<Mutex<DownloadQueue>>,
    /// Persistent application settings.
    pub(crate) settings: Arc<Mutex<AppSettings>>,
    /// Shared temp-file registry for crash-safe cleanup on exit.
    pub(crate) temp_registry: Arc<TempRegistry>,
}

impl AppState {
    /// Create a new `AppState` with default config, an empty queue,
    /// and default settings.
    ///
    /// # Panics
    ///
    /// Panics if the [`RdlpClient`] cannot be created with the
    /// default [`Config`]. This indicates a fundamental initialisation
    /// failure that cannot be recovered from.
    #[must_use]
    pub fn new() -> Self {
        let settings = AppSettings::load();

        // Remove stale temp files left by a prior crash in the output directory.
        TempRegistry::cleanup_stale(&settings.output_dir);

        // Create the shared registry once; pass it into the client so all
        // pipeline instances register their temp files in the same registry.
        let temp_registry = Arc::new(TempRegistry::new());

        // Load the user config file so installed plugins are picked up by
        // the desktop the same way they are by the CLI. Without this load
        // step, `Config::default()` ships an empty `plugin_directories`
        // and `RdlpClient::builder()` runs with zero discovered plugins —
        // the GUI would then fall back to built-in extractors only, even
        // when `~/.config/rdlp/plugins/<name>` is populated. Failure to
        // read or parse the file is non-fatal: we log and continue with
        // defaults so a corrupt config doesn't lock the user out of the
        // app.
        let config = match config_io::load_config(None) {
            Ok(Some((cfg, path))) => {
                log::info!(
                    "Loaded config from {} ({} plugin dir(s))",
                    path.display(),
                    cfg.plugin_directories.len(),
                );
                cfg
            }
            Ok(None) => {
                log::debug!("No config file found; using Config::default()");
                Config::default()
            }
            Err(e) => {
                log::warn!("Config load failed ({e}); using Config::default()");
                Config::default()
            }
        };

        // SAFETY (expect): startup-time fatal — builder only fails if the plugin
        // system cannot initialise (WASM runtime unavailable), which is an
        // unrecoverable environment failure.
        #[allow(clippy::expect_used)]
        let client = RdlpClient::builder()
            .config(config)
            .temp_registry(Arc::clone(&temp_registry))
            .build()
            .expect("Failed to create RdlpClient");

        Self {
            client: Arc::new(client),
            queue: Arc::new(Mutex::new(DownloadQueue::new())),
            settings: Arc::new(Mutex::new(settings)),
            temp_registry,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
