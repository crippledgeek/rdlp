//! Application state managed by Tauri.
//!
//! Holds the download queue and persistent settings, both wrapped
//! in `Arc<Mutex<_>>` for safe concurrent access from async
//! command handlers.

mod app_settings;
mod download_queue;

pub use app_settings::AppSettings;
pub use download_queue::DownloadQueue;

use std::sync::{Arc, Mutex};

/// Top-level application state registered with [`tauri::Builder::manage`].
pub struct AppState {
    /// Active and completed downloads.
    pub queue: Arc<Mutex<DownloadQueue>>,
    /// Persistent application settings.
    pub settings: Arc<Mutex<AppSettings>>,
}

impl AppState {
    /// Create a new `AppState` with default settings and an empty
    /// queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
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
