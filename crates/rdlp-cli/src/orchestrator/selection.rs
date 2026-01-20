//! Format selection logic

use super::{errors::*, Orchestrator};
use dialoguer::{theme::ColorfulTheme, Select};
use rdlp_core::{Format, FormatSelector};

impl Orchestrator {
    /// Select format from available formats
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (interactive mode only)
    ///
    /// # Errors
    /// Returns an error if:
    /// - Format selector string is invalid
    /// - No suitable format is found (automatic mode)
    pub(super) fn select_format(
        &self,
        formats: &[Format],
        interactive: bool,
    ) -> Result<Option<Format>> {
        let format = if interactive {
            match self.select_format_interactive(formats)? {
                Some(format) => format,
                None => {
                    println!("\n❌ Selection cancelled by user");
                    return Ok(None);
                }
            }
        } else {
            let format_selector = FormatSelector::parse(&self.config.format)
                .map_err(|e| OrchestratorError::InvalidFormatSelector(e.into()))?;

            let selected_formats = format_selector.select(formats);
            if selected_formats.is_empty() {
                return Err(OrchestratorError::NoFormat);
            }

            selected_formats[0].clone()
        };

        println!(
            "✓ Selected format: {} ({})",
            format.format_id,
            format.format_note.as_deref().unwrap_or("unknown")
        );
        Ok(Some(format))
    }

    /// Interactive format selection menu
    ///
    /// Displays a menu of available formats and lets the user select one.
    ///
    /// Returns Ok(Some(format)) if format selected, Ok(None) if user cancelled (ESC pressed)
    pub(super) fn select_format_interactive(&self, formats: &[Format]) -> Result<Option<Format>> {
        if formats.is_empty() {
            return Err(OrchestratorError::NoFormat);
        }

        // Build menu items with format details
        let items: Vec<String> = formats
            .iter()
            .map(|f| {
                let quality = f.format_note.as_deref().unwrap_or("unknown");
                let resolution = if let (Some(w), Some(h)) = (f.width, f.height) {
                    format!("{w}x{h}")
                } else {
                    "N/A".to_string()
                };

                let size = if let Some(filesize) = f.filesize {
                    format!("{:.1} MB", filesize as f64 / (1024.0 * 1024.0))
                } else {
                    "Unknown".to_string()
                };

                // Format type (HLS, MP4, etc.)
                let format_type = f.ext.to_uppercase();

                let codecs = match (&f.vcodec, &f.acodec) {
                    (Some(v), Some(a)) => format!("{v}/{a}"),
                    (Some(v), None) => format!("{v} (video only)"),
                    (None, Some(a)) => format!("{a} (audio only)"),
                    (None, None) => "Unknown".to_string(),
                };

                format!("{quality:<12} | {resolution:<10} | {size:<12} | {format_type:<6} | {codecs}")
            })
            .collect();

        println!("\n📋 Available formats:");
        println!(
            "{:<12} | {:<10} | {:<12} | {:<6} | Codecs",
            "Quality", "Resolution", "Size", "Type"
        );
        println!("{}", "-".repeat(79));

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a format to download (ESC to cancel)")
            .items(&items)
            .default(0)
            .interact_opt()
            .map_err(|e| OrchestratorError::Io(e.into()))?;

        match selection {
            Some(index) => Ok(Some(formats[index].clone())),
            None => Ok(None),
        }
    }
}
