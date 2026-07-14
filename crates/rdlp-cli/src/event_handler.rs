//! CLI event handler mapping [`Event`] to indicatif progress bars.
//!
//! Consumes download lifecycle events and renders appropriate
//! terminal UI using `indicatif` progress bars and spinners.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rdlp_api::DownloadProgress;
use rdlp_api::Event;
use rdlp_redact::RedactedUrl;
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
    pub const fn new(multi_progress: Arc<MultiProgress>, quiet: bool) -> Self {
        Self {
            multi_progress,
            progress_bar: None,
            quiet,
        }
    }

    /// Process a single download event.
    ///
    /// # Panics
    ///
    /// Panics are statically unreachable; all `expect` calls inside operate on
    /// static template strings and a field assigned earlier in the same branch.
    #[allow(clippy::too_many_lines)] // event dispatch match; extracting arms would be less readable
    pub fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Started { url, .. } => {
                if !self.quiet {
                    info!("Downloading: {}", RedactedUrl::new(url));
                }
            }
            Event::MetadataReady { info, .. } => {
                if !self.quiet {
                    info!(
                        "{} | {} format(s)",
                        crate::sanitize::sanitize_for_terminal(&info.title),
                        info.formats.len()
                    );
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
                        #[allow(clippy::expect_used)] // static template string — infallible
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
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // fraction() is in [0.0, 1.0]; * 1000 gives [0, 1000] — fits u64
                let position = (f64::from(progress.fraction()) * 1000.0) as u64;
                if let Some(ref pb) = self.progress_bar {
                    pb.set_position(position);
                    pb.set_message(stage.clone());
                } else if !self.quiet {
                    let pb = self.multi_progress.add(ProgressBar::new(1000));
                    pb.set_style(
                        #[allow(clippy::expect_used)] // static template string — infallible
                        ProgressStyle::with_template(
                            "{wide_bar:.yellow/blue} {percent}% | Post-processing: {msg}",
                        )
                        .expect("valid progress template"),
                    );
                    pb.set_message(stage.clone());
                    pb.set_position(position);
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
                    info!("[{}/{}] {}", index + 1, total, RedactedUrl::new(url));
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
            Event::UnitCompleted { .. } => {
                // Individual unit completions are tracked internally; no CLI output needed.
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
            #[allow(clippy::expect_used)] // field was just assigned in the line above
            self.progress_bar
                .as_ref()
                .expect("progress bar was just assigned")
        };

        if let Some(total) = progress.total_bytes {
            // The estimated total refines as fragments complete — keep the bar
            // length in sync, and surface the segment counter as secondary text.
            pb.set_length(total);
            pb.set_position(progress.bytes_downloaded);
            if progress.is_estimated
                && let (Some(done), Some(segs_total)) =
                    (progress.segments_downloaded, progress.total_segments)
            {
                pb.set_message(format!("frag {done}/{segs_total}"));
            }
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
    #[allow(clippy::option_if_let_else)] // nested if-let chain is clearer than map_or_else here
    fn create_progress_bar(&self, progress: &DownloadProgress) -> ProgressBar {
        // All three `expect` calls below are on static template string literals — infallible.
        #[allow(clippy::expect_used)]
        if let Some(total) = progress.total_bytes {
            let pb = self.multi_progress.add(ProgressBar::new(total));
            let template = if progress.is_estimated {
                // Estimated total (segmented download, no Content-Length): mark with ~,
                // and reserve {msg} for the "frag N/M" secondary counter.
                "{wide_bar:.cyan/blue} {bytes}/~{total_bytes} \
                 ({bytes_per_sec}) [{elapsed_precise}] {msg}"
            } else {
                "{wide_bar:.cyan/blue} {bytes}/{total_bytes} \
                 ({bytes_per_sec}) [{elapsed_precise}]"
            };
            pb.set_style(
                ProgressStyle::with_template(template)
                    .expect("static template string — infallible"),
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
                .expect("static template string — infallible"),
            );
            pb
        } else {
            // Unknown total: spinner
            let pb = self.multi_progress.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner} {bytes} ({bytes_per_sec}) [{elapsed_precise}]",
                )
                .expect("static template string — infallible"),
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
