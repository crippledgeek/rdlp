//! CLI-specific interactive callback using inquire.
//!
//! [`DialoguerCallback`] implements [`InteractiveCallback`] from `rdlp-api`
//! using inquire's `Select`, `MultiSelect`, and `Confirm` widgets.
//! All blocking terminal I/O is wrapped in `spawn_blocking` to avoid
//! blocking the Tokio runtime.

use async_trait::async_trait;
use log::info;
use rdlp_api::InteractiveCallback;
use rdlp_api::{Format, InfoDict};
use rdlp_table::{ColorMode, TableOpts, render_table_and_rows};

/// CLI interactive callback backed by inquire.
pub struct DialoguerCallback;

#[async_trait]
impl InteractiveCallback for DialoguerCallback {
    async fn select_format(&self, formats: &[Format], info: &InfoDict) -> Option<usize> {
        if formats.is_empty() {
            return None;
        }

        if info.is_live.unwrap_or(false) {
            info!("LIVE stream detected — download may be incomplete");
        }

        let opts = TableOpts {
            width_override: None,
            color: ColorMode::Auto,
            compact: None,
        };

        let refs: Vec<&Format> = formats.iter().collect();

        // Compute budget once for both header line and menu items.
        let (header, items) = render_table_and_rows(&refs, &opts);

        tokio::task::spawn_blocking(move || {
            // Print header line above the select menu so column names are visible.
            eprintln!("{header}");
            inquire::Select::new("Select a format:", items)
                .raw_prompt_skippable()
                .ok()
                .flatten()
                .map(|opt| opt.index)
        })
        .await
        .ok()?
    }

    async fn select_subtitles(&self, items: &[String], defaults: &[bool]) -> Option<Vec<usize>> {
        if items.is_empty() {
            return Some(Vec::new());
        }

        let items_owned: Vec<String> = items.to_vec();
        let default_indices: Vec<usize> = defaults
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| if d { Some(i) } else { None })
            .collect();

        tokio::task::spawn_blocking(move || {
            inquire::MultiSelect::new(
                "Select subtitle languages (SPACE to toggle, ENTER to confirm):",
                items_owned,
            )
            .with_default(&default_indices)
            .raw_prompt_skippable()
            .ok()
            .flatten()
            .map(|selected| selected.iter().map(|opt| opt.index).collect())
        })
        .await
        .ok()?
    }

    async fn select_audio_type(&self, options: &[String]) -> Option<usize> {
        if options.is_empty() {
            return None;
        }

        let options_owned: Vec<String> = options.to_vec();

        tokio::task::spawn_blocking(move || {
            inquire::Select::new("Select audio type:", options_owned)
                .raw_prompt_skippable()
                .ok()
                .flatten()
                .map(|opt| opt.index)
        })
        .await
        .ok()?
    }

    async fn confirm(&self, prompt: &str) -> bool {
        let prompt_owned = prompt.to_string();

        tokio::task::spawn_blocking(move || {
            inquire::Confirm::new(&prompt_owned)
                .with_default(true)
                .prompt()
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    }
}
