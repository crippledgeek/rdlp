//! Callback trait for interactive user input.
//!
//! CLI provides a `dialoguer`-based implementation. Tauri/Leptos
//! provide their own. The orchestrator calls these methods when
//! interactive mode is enabled.

use async_trait::async_trait;
use rdlp_core::{Format, InfoDict};

/// Callback for interactive user input (format selection, subtitle selection).
///
/// Frontends implement this trait to provide their own UI for user choices.
/// The orchestrator calls these methods when interactive mode is enabled.
///
/// Return `None` from any method to signal cancellation.
#[async_trait]
pub trait InteractiveCallback: Send + Sync {
    /// Select a format from the available options.
    ///
    /// # Arguments
    /// * `formats` - Available format options (already grouped/filtered)
    /// * `info` - Video metadata for display
    ///
    /// # Returns
    /// Index into `formats` or `None` to cancel
    async fn select_format(&self, formats: &[Format], info: &InfoDict) -> Option<usize>;

    /// Select subtitle languages from available options.
    ///
    /// # Arguments
    /// * `items` - Available subtitle display strings
    /// * `defaults` - Pre-selected state for each item
    ///
    /// # Returns
    /// Indices of selected items, or `None` to cancel
    async fn select_subtitles(&self, items: &[String], defaults: &[bool]) -> Option<Vec<usize>>;

    /// Select audio type for playlist (SUB/DUB).
    ///
    /// # Arguments
    /// * `options` - Available audio type labels
    ///
    /// # Returns
    /// Index into `options`, or `None` for "keep all"
    async fn select_audio_type(&self, options: &[String]) -> Option<usize>;

    /// Confirm a playlist download.
    ///
    /// # Arguments
    /// * `prompt` - Confirmation message to display
    ///
    /// # Returns
    /// `true` to proceed, `false` to cancel
    async fn confirm(&self, prompt: &str) -> bool;
}
