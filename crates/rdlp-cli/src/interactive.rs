//! CLI-specific interactive callback using inquire.
//!
//! [`DialoguerCallback`] implements [`InteractiveCallback`] from `rdlp-api`
//! using inquire's `Select`, `MultiSelect`, and `Confirm` widgets.
//! All blocking terminal I/O is wrapped in `spawn_blocking` to avoid
//! blocking the Tokio runtime.

use async_trait::async_trait;
use log::info;
use rdlp_api::InteractiveCallback;
use rdlp_core::{Format, InfoDict};
use rdlp_table::{ColorMode, FormatRow, TableOpts, render_format_rows, render_formats_table};

/// Build a [`FormatRow`] from a [`Format`] for table rendering.
fn format_to_row(f: &Format) -> FormatRow {
    let quality = {
        let base = f.format_note.as_deref().unwrap_or("unknown");
        match f.fps {
            Some(fps) if fps > 0.0 && (fps - 30.0).abs() > 1.0 => format!("{base}{fps:.0}"),
            _ => base.to_string(),
        }
    };

    let resolution = match (f.width, f.height) {
        (Some(w), Some(h)) => format!("{w}x{h}"),
        _ => "N/A".to_string(),
    };

    let format_type = if f.is_hls() {
        "HLS".to_string()
    } else {
        f.ext.to_ascii_uppercase()
    };

    let size = f.size_text();

    let codecs = match (&f.vcodec, &f.acodec) {
        (Some(v), Some(a)) if v != "none" && a != "none" => format!("{v}/{a}"),
        (Some(v), _) if v != "none" => format!("{v} (video only)"),
        (_, Some(a)) if a != "none" => format!("{a} (audio only)"),
        _ => "Unknown".to_string(),
    };
    let codecs = if f.has_drm.unwrap_or(false) {
        format!("{codecs} [DRM]")
    } else {
        codecs
    };

    let fps = match f.fps {
        Some(fps) if fps > 0.0 => format!("{fps:.0}"),
        _ => String::new(),
    };

    let abr = match f.abr {
        Some(abr) if abr > 0.0 => format!("{abr:.0}"),
        _ => String::new(),
    };

    let vbr = match f.vbr {
        Some(vbr) if vbr > 0.0 => format!("{vbr:.0}"),
        _ => String::new(),
    };

    let note = {
        let mut parts = Vec::new();
        if let Some(dr) = &f.dynamic_range {
            parts.push(dr.clone());
        }
        if let Some(lang) = &f.language {
            parts.push(lang.clone());
        }
        parts.join(", ")
    };

    FormatRow {
        quality,
        resolution,
        format_type,
        size,
        codecs,
        fps,
        abr,
        vbr,
        note,
    }
}

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

        let rows: Vec<FormatRow> = formats.iter().map(format_to_row).collect();

        // Print the formatted table header.
        let table = render_formats_table(&rows, &opts);
        info!("Available formats:\n{table}");

        // Build menu items matching the table's column layout.
        let items = render_format_rows(&rows, &opts);

        tokio::task::spawn_blocking(move || {
            inquire::Select::new("Select a format to download:", items)
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
