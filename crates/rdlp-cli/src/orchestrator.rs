use anyhow::{Context, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use indicatif::{ProgressBar, ProgressStyle};
use rdlp_cookies::SimpleCookieJar;
use rdlp_core::{
    Config, DownloadProgress, DownloadStats, ExtractionContext, Format, FormatSelector, ProgressCallback,
};
use rdlp_downloader::DownloaderRegistry;
use rdlp_extractor::ExtractorRegistry;
use rdlp_jsinterp::SimpleJsEngine;
use std::path::PathBuf;
use std::sync::Arc;

/// Main orchestrator coordinating extraction, download, and post-processing
pub struct Orchestrator {
    extractor_registry: ExtractorRegistry,
    downloader_registry: DownloaderRegistry,
    extraction_context: Arc<ExtractionContext>,
    config: Config,
}

impl Orchestrator {
    /// Create a new orchestrator with default registries
    pub fn new(config: Config) -> Self {
        let http_client = Arc::new(reqwest::Client::new());
        let js_engine = Arc::new(SimpleJsEngine::new());
        let cookie_jar = Arc::new(SimpleCookieJar::new());

        let extraction_context = Arc::new(ExtractionContext::new(
            http_client,
            js_engine,
            cookie_jar,
            Arc::new(config.clone()),
        ));

        Self {
            extractor_registry: ExtractorRegistry::new(),
            downloader_registry: DownloaderRegistry::new(),
            extraction_context,
            config,
        }
    }

    /// Download a video from URL
    /// Returns Ok(Some(path)) on success, Ok(None) if cancelled by user, Err on error
    pub async fn download(&self, url: &str, interactive: bool) -> Result<Option<PathBuf>> {
        println!("🔍 Finding extractor for URL...");

        // Find suitable extractor
        let extractor = self
            .extractor_registry
            .find_extractor(url)
            .with_context(|| format!("No extractor found for URL: {url}"))?;

        println!("✓ Using {} extractor", extractor.name());

        // Extract video information
        println!("📊 Extracting video information...");
        let info = extractor
            .extract(url, &self.extraction_context)
            .await
            .context("Failed to extract video information")?;

        println!("✓ Title: {}", info.title);
        println!("✓ Found {} formats", info.formats.len());

        // Select format
        let format = if interactive {
            match self.select_format_interactive(&info.formats)? {
                Some(format) => format,
                None => {
                    println!("\n❌ Selection cancelled by user");
                    return Ok(None);
                }
            }
        } else {
            let format_selector = FormatSelector::parse(&self.config.format)
                .context("Failed to parse format selector")?;

            let selected_formats = format_selector.select(&info.formats);
            if selected_formats.is_empty() {
                anyhow::bail!("No suitable format found");
            }

            selected_formats[0].clone()
        };

        println!("✓ Selected format: {} ({})", format.format_id, format.format_note.as_deref().unwrap_or("unknown"));

        // Find downloader
        let downloader = self
            .downloader_registry
            .find_downloader(&format.url)
            .with_context(|| format!("No downloader found for URL: {}", format.url))?;

        // Generate output filename
        let output_path = self.generate_output_path(&info, &format)?;
        println!("💾 Downloading to: {}", output_path.display());

        // Check if partial download exists
        let resume_from = if output_path.exists() {
            match tokio::fs::metadata(&output_path).await {
                Ok(metadata) => {
                    let size = metadata.len();
                    if size > 0 {
                        println!("📋 Found partial download ({:.1} MB), resuming...", size as f64 / (1024.0 * 1024.0));
                        size
                    } else {
                        0
                    }
                }
                Err(_) => 0,
            }
        } else {
            0
        };

        // Create progress bar
        let progress_bar = if self.config.progress {
            let pb = ProgressBar::new(format.filesize.unwrap_or(0));
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
                    )?
                    .progress_chars("#>-"),
            );
            if resume_from > 0 {
                pb.set_position(resume_from);
            }
            Some(pb)
        } else {
            None
        };

        // Download with progress tracking and Ctrl+C handling
        let progress_callback: Option<Box<dyn ProgressCallback>> = progress_bar.as_ref().map(|pb| {
            Box::new(ProgressBarCallback::new(pb.clone())) as Box<dyn ProgressCallback>
        });

        println!("⚠️  Press Ctrl+C to pause and save progress");

        let download_future = if resume_from > 0 {
            downloader.download_with_resume(&format.url, &output_path, resume_from, progress_callback)
        } else {
            downloader.download_to_file(&format.url, &output_path, progress_callback)
        };

        // Race between download and Ctrl+C signal
        let stats = tokio::select! {
            result = download_future => {
                result.context("Download failed")?
            }
            _ = tokio::signal::ctrl_c() => {
                if let Some(pb) = &progress_bar {
                    pb.finish_with_message("⏸️  Download paused");
                }
                println!("\n⏸️  Download interrupted by user");
                println!("💾 Progress saved. Run the same command again to resume.");
                return Ok(None);
            }
        };

        if let Some(pb) = progress_bar {
            pb.finish_with_message("✓ Download complete");
        }

        println!("\n✅ Downloaded successfully!");
        println!("   File: {}", output_path.display());
        println!("   Size: {}", stats.bytes_string());
        println!("   Speed: {}", stats.speed_string());
        println!("   Time: {:.2}s", stats.duration.as_secs_f64());

        Ok(Some(output_path))
    }

    /// Generate output file path
    fn generate_output_path(&self, info: &rdlp_core::InfoDict, format: &rdlp_core::Format) -> Result<PathBuf> {
        // Simple template parsing (full implementation in Phase 5)
        let filename = format!(
            "{}.{}",
            self.sanitize_filename(&info.title),
            format.ext
        );

        let mut path = self.config.output_directory.clone();
        path.push(filename);

        Ok(path)
    }

    /// Sanitize filename by removing invalid characters
    fn sanitize_filename(&self, name: &str) -> String {
        name.chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect()
    }

    /// List all available extractors
    pub fn list_extractors(&self) -> Vec<String> {
        self.extractor_registry.list_extractors()
    }

    /// Interactive format selection menu
    /// Returns Ok(Some(format)) if user selects, Ok(None) if cancelled
    fn select_format_interactive(&self, formats: &[Format]) -> Result<Option<Format>> {
        if formats.is_empty() {
            anyhow::bail!("No formats available");
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

                let codecs = match (&f.vcodec, &f.acodec) {
                    (Some(v), Some(a)) => format!("{v}/{a}"),
                    (Some(v), None) => format!("{v} (video only)"),
                    (None, Some(a)) => format!("{a} (audio only)"),
                    (None, None) => "Unknown".to_string(),
                };

                format!(
                    "{quality:<12} | {resolution:<10} | {size:<12} | {codecs}"
                )
            })
            .collect();

        println!("\n📋 Available formats:");
        println!("{:<12} | {:<10} | {:<12} | Codecs", "Quality", "Resolution", "Size");
        println!("{}", "-".repeat(70));

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select a format to download (ESC to cancel)")
            .items(&items)
            .default(0)
            .interact_opt()
            .context("Failed to get user selection")?;

        match selection {
            Some(index) => Ok(Some(formats[index].clone())),
            None => Ok(None),
        }
    }
}

/// Progress callback that updates a progress bar
struct ProgressBarCallback {
    progress_bar: ProgressBar,
}

impl ProgressBarCallback {
    fn new(progress_bar: ProgressBar) -> Self {
        Self { progress_bar }
    }
}

impl ProgressCallback for ProgressBarCallback {
    fn on_progress(&self, progress: &DownloadProgress) {
        if let Some(total) = progress.total_bytes {
            self.progress_bar.set_length(total);
        }
        self.progress_bar.set_position(progress.bytes_downloaded);
    }

    fn on_complete(&self, _stats: &DownloadStats) {
        // Progress bar will be finished by caller
    }

    fn on_error(&self, error: &str) {
        self.progress_bar.abandon_with_message(format!("❌ Error: {error}"));
    }
}
