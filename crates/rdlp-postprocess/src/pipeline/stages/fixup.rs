//! FixupStage — detect and repair container/codec issues.
//!
//! Non-fatal: on failure, logs a warning and passes the file through unchanged.

use std::sync::Arc;

use async_trait::async_trait;
use log::{info, warn};

use rdlp_ffmpeg::FFmpegRunner;
use rdlp_ffmpeg::ffmpeg::fixup::{FixupIssue, detect_issues};
use rdlp_types::FixupPolicy;

use crate::pipeline::{PipelineMessage, PipelineStage};

/// Detects and repairs container/codec issues in the primary current file.
///
/// `should_run` triggers when `config.fixup != FixupPolicy::Never`.
/// Non-fatal: failures push a warning and pass through unchanged.
pub struct FixupStage {
    ffmpeg: Arc<FFmpegRunner>,
}

impl FixupStage {
    /// Create a new `FixupStage`.
    #[must_use]
    pub fn new(ffmpeg: Arc<FFmpegRunner>) -> Self {
        Self { ffmpeg }
    }
}

#[async_trait]
impl PipelineStage for FixupStage {
    fn name(&self) -> &str {
        "FixupStage"
    }

    fn should_run(&self, msg: &PipelineMessage) -> bool {
        msg.config.fixup != FixupPolicy::Never
    }

    fn is_fatal(&self) -> bool {
        false
    }

    async fn process(&self, mut msg: PipelineMessage) -> anyhow::Result<PipelineMessage> {
        if msg.tracker.current_files.is_empty() {
            return Ok(msg);
        }

        // Announce stage to UI via callback factory.
        let _stage_callback = msg.callback_factory.as_ref().map(|f| f(self.name()));

        let input_file = msg.tracker.primary();
        info!("FixupStage: probing {}", input_file.display());

        // Probe the file
        let media_info = match self.ffmpeg.probe(&input_file).await {
            Ok(info) => info,
            Err(e) => {
                warn!("FixupStage: probe failed: {e}");
                msg.warnings.push(format!("Fixup probe failed: {e}"));
                return Ok(msg);
            }
        };

        // Detect issues
        let expected_duration = msg.info.duration;
        let issues = detect_issues(&media_info, expected_duration);

        if issues.is_empty() {
            info!("FixupStage: no issues detected");
            return Ok(msg);
        }

        // Log all issues
        for issue in &issues {
            if issue.is_repairable() {
                info!("FixupStage: detected (repairable): {issue}");
            } else {
                warn!("FixupStage: detected (unrepairable): {issue}");
                msg.warnings.push(format!("Fixup: {issue}"));
            }
        }

        // Warn-only mode
        if msg.config.fixup == FixupPolicy::Warn {
            for issue in &issues {
                if issue.is_repairable() {
                    msg.warnings.push(format!("Fixup (warn only): {issue}"));
                }
            }
            return Ok(msg);
        }

        // DetectOrWarn: attempt repair
        let repairable: Vec<FixupIssue> = issues
            .iter()
            .filter(|i| i.is_repairable())
            .cloned()
            .collect();
        if repairable.is_empty() {
            return Ok(msg);
        }

        let ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4")
            .to_string();

        let temp_output = msg.tracker.temp_path(&input_file, &ext);

        match self
            .ffmpeg
            .fixup_repair(
                &input_file,
                &temp_output,
                &repairable,
                msg.encoding_tool.clone(),
            )
            .await
        {
            Ok(()) => {
                info!("FixupStage: repair successful");
                msg.tracker.replace(vec![temp_output]);
            }
            Err(e) => {
                warn!("FixupStage: repair failed: {e}");
                msg.warnings.push(format!("Fixup repair failed: {e}"));
                msg.tracker.mark_temp(temp_output);
            }
        }

        Ok(msg)
    }
}
