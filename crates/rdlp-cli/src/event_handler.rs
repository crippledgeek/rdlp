//! CLI event handler mapping [`Event`] to indicatif progress bars.
//!
//! Consumes download lifecycle events and renders appropriate
//! terminal UI using `indicatif` progress bars and spinners.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rdlp_api::DownloadProgress;
use rdlp_api::Event;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Handles download lifecycle events for CLI display.
pub struct CliEventHandler {
    multi_progress: Arc<MultiProgress>,
    progress_bar: Option<ProgressBar>,
    quiet: bool,
}

impl CliEventHandler {
    /// Create a new event handler.
    ///
    /// # Arguments
    /// * `multi_progress` - Shared `MultiProgress` for managing bars
    /// * `quiet` - Suppress non-essential output
    #[must_use]
    pub fn new(multi_progress: Arc<MultiProgress>, quiet: bool) -> Self {
        Self {
            multi_progress,
            progress_bar: None,
            quiet,
        }
    }

    /// Process a single download event.
    pub fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Started { url, .. } => {
                if !self.quiet {
                    info!("Downloading: {url}");
                }
            }
            Event::MetadataReady { info, .. } => {
                if !self.quiet {
                    info!("{} | {} format(s)", info.title, info.formats.len());
                }
            }
            Event::FormatSelected {
                format_id, quality, ..
            } => {
                if !self.quiet {
                    info!("Format: {format_id} ({quality})");
                }
            }
            Event::Progress { progress, .. } => {
                self.update_progress(progress);
            }
            Event::PostProcessing { stage, .. } => {
                self.finish_progress();
                if !self.quiet {
                    let pb = self.multi_progress.add(ProgressBar::new(1000));
                    pb.set_style(
                        ProgressStyle::with_template(
                            "{wide_bar:.yellow/blue} {percent}% | Post-processing: {msg}",
                        )
                        .expect("valid progress template"),
                    );
                    pb.set_message(stage.clone());
                    pb.set_position(0);
                    self.progress_bar = Some(pb);
                }
            }
            Event::PostProcessProgress {
                stage, progress, ..
            } => {
                if let Some(ref pb) = self.progress_bar {
                    let pct = (*progress * 1000.0) as u64;
                    pb.set_position(pct);
                    pb.set_message(stage.clone());
                } else if !self.quiet {
                    let pb = self.multi_progress.add(ProgressBar::new(1000));
                    pb.set_style(
                        ProgressStyle::with_template(
                            "{wide_bar:.yellow/blue} {percent}% | Post-processing: {msg}",
                        )
                        .expect("valid progress template"),
                    );
                    pb.set_message(stage.clone());
                    pb.set_position((*progress * 1000.0) as u64);
                    self.progress_bar = Some(pb);
                }
            }
            Event::SubtitlesFound { langs, .. } => {
                if !self.quiet {
                    info!("Subtitles found: {}", langs.join(", "));
                }
            }
            Event::SubtitlesMissing { requested, .. } => {
                warn!("Subtitles not found: {}", requested.join(", "));
            }
            Event::Warning { message, .. } => {
                warn!("{message}");
            }
            Event::Completed { .. } | Event::Failed { .. } | Event::Cancelled { .. } => {
                self.finish_progress();
            }
            Event::PlaylistDetected { total_items, .. } => {
                if !self.quiet {
                    info!("Playlist detected: {total_items} items");
                }
            }
            Event::PlaylistItemStarted {
                index, total, url, ..
            } => {
                self.finish_progress();
                if !self.quiet {
                    info!("[{}/{}] {url}", index + 1, total);
                }
            }
            Event::Retrying {
                attempt,
                max_attempts,
                reason,
                ..
            } => {
                warn!("Retry {attempt}/{max_attempts}: {reason}");
            }
            Event::Debug { message, .. } => {
                debug!("{message}");
            }
        }
    }

    /// Create or update the progress bar from download progress data.
    fn update_progress(&mut self, progress: &DownloadProgress) {
        let pb = if let Some(ref pb) = self.progress_bar {
            pb
        } else {
            let pb = self.create_progress_bar(progress);
            self.progress_bar = Some(pb);
            self.progress_bar
                .as_ref()
                .expect("progress bar was just assigned")
        };

        if progress.total_bytes.is_some() {
            // HTTP: byte-based progress
            pb.set_position(progress.bytes_downloaded);
        } else if let Some(segs) = progress.segments_downloaded {
            // HLS: segment-based progress
            pb.set_position(segs);
            pb.set_message(format!(
                "{} at {}",
                progress.bytes_string(),
                progress.speed_string()
            ));
        } else {
            // Unknown total: just show bytes
            pb.set_position(progress.bytes_downloaded);
        }
    }

    /// Create a progress bar appropriate for the download type.
    fn create_progress_bar(&self, progress: &DownloadProgress) -> ProgressBar {
        if let Some(total) = progress.total_bytes {
            // HTTP download with known size
            let pb = self.multi_progress.add(ProgressBar::new(total));
            pb.set_style(
                ProgressStyle::with_template(
                    "{wide_bar:.cyan/blue} {bytes}/{total_bytes} \
                     ({bytes_per_sec}) [{elapsed_precise}]",
                )
                .expect("valid progress template"),
            );
            pb
        } else if let Some(total) = progress.total_segments {
            // HLS download with known segment count
            let pb = self.multi_progress.add(ProgressBar::new(total));
            pb.set_style(
                ProgressStyle::with_template(
                    "{wide_bar:.green/blue} {pos}/{len} segs | \
                     {msg} [{elapsed_precise}]",
                )
                .expect("valid progress template"),
            );
            pb
        } else {
            // Unknown total: spinner
            let pb = self.multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner} {bytes} ({bytes_per_sec}) [{elapsed_precise}]",
                )
                .expect("valid progress template"),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            pb
        }
    }

    /// Finish and clear the current progress bar.
    fn finish_progress(&mut self) {
        if let Some(pb) = self.progress_bar.take() {
            pb.finish_and_clear();
        }
    }
}
