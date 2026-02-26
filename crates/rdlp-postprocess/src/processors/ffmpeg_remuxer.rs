//! FFmpeg container remuxer post-processor.
//!
//! Remuxes video files to different containers (MP4, MKV) using stream copy
//! (no re-encoding). Improves seeking for formats like MPEG-TS which lack
//! a centralized index.

use std::path::PathBuf;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor, Result};

use rdlp_ffmpeg::RemuxOptions;

#[cfg(test)]
use rdlp_core::ContainerFormat;

ffmpeg_processor!(
    FFmpegRemuxer,
    "FFmpegRemuxer",
    45,
    "Post-processor that remuxes video files to a different container.\n\n\
     This performs stream copy (no re-encoding) for fast container conversion.\n\
     Primary use case: remux MPEG-TS (.ts) to MP4 or MKV for better seeking.\n\n\
     # Priority\n\
     This processor has priority 45 (runs after merging, before video conversion).\n\n\
     # When it runs\n\
     When `remux_container` is specified in config."
);

impl FFmpegRemuxer {
    /// Check if the format is a supported container for remuxing.
    #[cfg(test)]
    fn is_supported_container(format: &str) -> bool {
        format.parse::<ContainerFormat>().is_ok()
    }
}

#[async_trait]
impl PostProcessor for FFmpegRemuxer {
    fn name(&self) -> &str {
        self.processor_name()
    }

    fn priority(&self) -> i32 {
        self.processor_priority()
    }

    fn should_run(&self, _info: &InfoDict, config: &PostProcessConfig) -> bool {
        // Skip remux when audio extraction is active — extract_audio produces
        // a standalone audio file that should not be remuxed into a video container.
        config.remux_container.is_some() && !config.extract_audio
    }

    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
    ) -> Result<PostProcessResult> {
        if files.is_empty() {
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        let input_file = &files[0];
        let target_container = match config.remux_container {
            Some(c) => c,
            None => {
                // should_run() gates on Some — this is unreachable
                warn!("FFmpegRemuxer invoked without remux_container; skipping");
                return Ok(PostProcessResult::new(info.clone(), files));
            }
        };

        let input_ext = input_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        // Skip if already in target container
        if input_ext.eq_ignore_ascii_case(target_container.as_ext()) {
            debug!(
                container = target_container.as_ext();
                "File already in target container, skipping remux"
            );
            return Ok(PostProcessResult::new(info.clone(), files));
        }

        info!(
            file:? = input_file.display(),
            from = input_ext,
            to = target_container.as_ext();
            "Remuxing to improve seeking"
        );

        // Build output path
        let output_path = input_file.with_extension(target_container.as_ext());

        // Configure remux options
        // MP4/MOV: enable faststart for progressive playback (moov atom at beginning)
        // MKV: cues written at end, but seeking is still excellent
        // WebM: similar to MKV (Matroska-based)
        let opts = RemuxOptions {
            faststart: target_container.supports_faststart(),
            output_format: Some(target_container.to_string()),
        };

        // Perform remux via library bindings
        self.ffmpeg.remux(input_file, &output_path, &opts).await?;

        debug!(
            output:? = output_path.display();
            "Remuxed successfully"
        );

        Ok(PostProcessResult {
            info: info.clone(),
            files: vec![output_path],
            temp_files: if config.keep_video { Vec::new() } else { files },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use rdlp_ffmpeg::FFmpegRunner;

    #[test]
    fn test_supported_containers() {
        // All supported containers
        assert!(FFmpegRemuxer::is_supported_container("mp4"));
        assert!(FFmpegRemuxer::is_supported_container("MP4"));
        assert!(FFmpegRemuxer::is_supported_container("mkv"));
        assert!(FFmpegRemuxer::is_supported_container("MKV"));
        assert!(FFmpegRemuxer::is_supported_container("webm"));
        assert!(FFmpegRemuxer::is_supported_container("mov"));
        assert!(FFmpegRemuxer::is_supported_container("avi"));
        assert!(FFmpegRemuxer::is_supported_container("ts"));
        assert!(FFmpegRemuxer::is_supported_container("flv"));

        // Aliases
        assert!(FFmpegRemuxer::is_supported_container("wmv"));
        assert!(FFmpegRemuxer::is_supported_container("3gp"));
        assert!(FFmpegRemuxer::is_supported_container("flac"));

        // Unsupported
        assert!(!FFmpegRemuxer::is_supported_container("xyz"));
    }

    #[test]
    fn test_priority() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let remuxer = FFmpegRemuxer::new(Arc::new(ffmpeg));
            // Should run after merger (100) but before video convertor (40)
            assert_eq!(remuxer.priority(), 45);
        }
    }

    #[test]
    fn test_should_run_skips_when_extract_audio() {
        if let Ok(ffmpeg) = FFmpegRunner::new() {
            let remuxer = FFmpegRemuxer::new(Arc::new(ffmpeg));
            let info = InfoDict::new("id", "title", "ext", "url");

            // Remux alone — should run
            let config = PostProcessConfig {
                remux_container: Some(ContainerFormat::Mkv),
                ..PostProcessConfig::default()
            };
            assert!(remuxer.should_run(&info, &config));

            // Remux + extract_audio — should NOT run
            let config = PostProcessConfig {
                remux_container: Some(ContainerFormat::Mkv),
                extract_audio: true,
                ..PostProcessConfig::default()
            };
            assert!(!remuxer.should_run(&info, &config));

            // extract_audio alone (no remux) — should NOT run
            let config = PostProcessConfig {
                extract_audio: true,
                ..PostProcessConfig::default()
            };
            assert!(!remuxer.should_run(&info, &config));
        }
    }
}
