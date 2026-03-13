//! Tauri IPC command handlers.
//!
//! Each submodule groups related commands that are registered with
//! [`tauri::generate_handler!`] in [`crate::run`].

/// Download lifecycle commands (start, cancel, queue).
pub mod download;
/// Format listing commands.
pub mod formats;
/// URL and site search commands.
pub mod search;
/// Application settings commands.
pub mod settings;
/// Thumbnail proxy command.
pub mod thumbnail;
