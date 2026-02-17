//! CLI-specific interactive callback using dialoguer.
//!
//! [`DialoguerCallback`] implements [`InteractiveCallback`] from `rdlp-api`
//! using dialoguer's `Select`, `MultiSelect`, and `Confirm` widgets.
//! All blocking terminal I/O is wrapped in `spawn_blocking` to avoid
//! blocking the Tokio runtime.

use async_trait::async_trait;
use dialoguer::{Confirm, MultiSelect, Select, theme::ColorfulTheme};
use log::info;
use rdlp_api::InteractiveCallback;
use rdlp_core::{Format, InfoDict};

/// CLI interactive callback backed by dialoguer.
pub struct DialoguerCallback;

#[async_trait]
impl InteractiveCallback for DialoguerCallback {
    async fn select_format(&self, formats: &[Format], info: &InfoDict) -> Option<usize> {
        if formats.is_empty() {
            return None;
        }

        // Warn about live streams
        if info.is_live.unwrap_or(false) {
            info!("LIVE stream detected — download may be incomplete");
        }

        // Compute dynamic size column width
        let size_width = formats
            .iter()
            .map(|f| f.size_text().len())
            .max()
            .unwrap_or(4)
            .max(4);

        // Build menu items with format details
        let items: Vec<String> = formats.iter().map(|f| f.table_row(size_width)).collect();

        info!("Available formats:");
        info!(
            "{:<qw$} | {:<rw$} | {:<size_width$} | {:<tw$} | Codecs",
            "Quality",
            "Resolution",
            "Size",
            "Type",
            qw = rdlp_table::QUALITY_WIDTH,
            rw = rdlp_table::RESOLUTION_WIDTH,
            tw = rdlp_table::TYPE_WIDTH,
        );
        info!("{}", "-".repeat(rdlp_table::separator_width(size_width)));

        tokio::task::spawn_blocking(move || {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select a format to download (ESC to cancel)")
                .items(&items)
                .default(0)
                .interact_opt()
        })
        .await
        .ok()?
        .ok()?
    }

    async fn select_subtitles(&self, items: &[String], defaults: &[bool]) -> Option<Vec<usize>> {
        if items.is_empty() {
            return Some(Vec::new());
        }

        let items_owned: Vec<String> = items.to_vec();
        let defaults_owned: Vec<bool> = defaults.to_vec();

        tokio::task::spawn_blocking(move || {
            MultiSelect::with_theme(&ColorfulTheme::default())
                .with_prompt(
                    "Select subtitle languages (SPACE to toggle, ENTER to confirm, ESC to cancel)",
                )
                .items(&items_owned)
                .defaults(&defaults_owned)
                .interact_opt()
        })
        .await
        .ok()?
        .ok()?
    }

    async fn select_audio_type(&self, options: &[String]) -> Option<usize> {
        if options.is_empty() {
            return None;
        }

        let options_owned: Vec<String> = options.to_vec();

        tokio::task::spawn_blocking(move || {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select audio type (ESC to skip)")
                .items(&options_owned)
                .default(0)
                .interact_opt()
        })
        .await
        .ok()?
        .ok()?
    }

    async fn confirm(&self, prompt: &str) -> bool {
        let prompt_owned = prompt.to_string();

        tokio::task::spawn_blocking(move || {
            Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(prompt_owned)
                .default(true)
                .interact()
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }
}
