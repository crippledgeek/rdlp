//! Format selection logic

use super::{Orchestrator, errors::*};
use dialoguer::{Select, theme::ColorfulTheme};
use log::{info, warn};
use rdlp_core::{Format, FormatSelector, InfoDict};

impl Orchestrator {
    /// Select format from available formats
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (interactive mode only)
    ///
    /// # Errors
    /// Returns an error if:
    /// - Format selector string is invalid
    /// - No suitable format is found (automatic mode)
    pub(super) async fn select_format(
        &self,
        info: &InfoDict,
        interactive: bool,
    ) -> Result<Option<Format>> {
        let formats = &info.formats;
        let format = if interactive {
            match self.select_format_interactive(info).await? {
                Some(format) => format,
                None => {
                    info!("Selection cancelled by user");
                    return Ok(None);
                }
            }
        } else {
            let format_selector = FormatSelector::parse(&self.config.format)
                .map_err(OrchestratorError::InvalidFormatSelector)?;

            let selected_formats = format_selector.select(formats);
            if selected_formats.is_empty() {
                return Err(OrchestratorError::NoFormat);
            }

            selected_formats[0].clone()
        };

        info!(format:?; "Selected format");
        Ok(Some(format))
    }

    /// Interactive format selection menu
    ///
    /// Displays a menu of available formats and lets the user select one.
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (ESC pressed)
    pub(super) async fn select_format_interactive(&self, info: &InfoDict) -> Result<Option<Format>> {
        let formats = &info.formats;
        if formats.is_empty() {
            return Err(OrchestratorError::NoFormat);
        }

        // Warn about live streams before showing format table
        if info.is_live.unwrap_or(false) {
            warn!("LIVE stream detected — download may be incomplete");
        }

        // Build menu items with format details
        let items: Vec<String> = formats.iter().map(|f| f.table_row()).collect();

        info!("Available formats:");
        info!(
            "{:<12} | {:<10} | {:<12} | {:<6} | Codecs",
            "Quality", "Resolution", "Size", "Type"
        );
        info!("{}", "-".repeat(79));

        let selection = tokio::task::spawn_blocking(move || {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Select a format to download (ESC to cancel)")
                .items(&items)
                .default(0)
                .interact_opt()
        })
        .await
        .map_err(|e| OrchestratorError::Io(std::io::Error::other(e)))?
        .map_err(|e| OrchestratorError::Io(e.into()))?;

        match selection {
            Some(index) => Ok(Some(formats[index].clone())),
            None => Ok(None),
        }
    }
}
