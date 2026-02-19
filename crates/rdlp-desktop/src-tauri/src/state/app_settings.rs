//! Persistent application settings.
//!
//! Stores user preferences such as the download directory,
//! concurrent download limits, and post-processing options.

use serde::{Deserialize, Serialize};

/// Application settings that persist between sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Directory where downloaded files are saved.
    pub download_directory: String,
    /// Maximum number of concurrent downloads.
    pub max_concurrent_downloads: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        let download_dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());

        Self {
            download_directory: download_dir,
            max_concurrent_downloads: 3,
        }
    }
}
