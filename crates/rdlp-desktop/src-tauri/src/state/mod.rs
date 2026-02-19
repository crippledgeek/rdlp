//! Application state managed by Tauri.
//!
//! Holds the [`RdlpClient`], download queue, and persistent settings.
//! The client is shared via `Arc` (it is already thread-safe and
//! cloneable), while mutable state is wrapped in `Arc<Mutex<_>>` for
//! safe concurrent access from async command handlers.

mod app_settings;
mod download_queue;

pub use app_settings::AppSettings;
pub use download_queue::{DownloadJob, DownloadQueue, JobStatus};

use std::sync::{Arc, Mutex};

use rdlp_api::RdlpClient;
use rdlp_core::Config;

/// Top-level application state registered with [`tauri::Builder::manage`].
///
/// Created once at startup and shared across all IPC command handlers.
/// Access in Tauri commands via `State<'_, AppState>`.
pub struct AppState {
    /// The rdlp download engine client (thread-safe, cloneable).
    pub client: Arc<RdlpClient>,
    /// Active and completed downloads.
    pub queue: Arc<Mutex<DownloadQueue>>,
    /// Persistent application settings.
    pub settings: Arc<Mutex<AppSettings>>,
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
        let client = RdlpClient::new(Config::default()).expect("Failed to create RdlpClient");

        Self {
            client: Arc::new(client),
            queue: Arc::new(Mutex::new(DownloadQueue::new())),
            settings: Arc::new(Mutex::new(AppSettings::default())),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
