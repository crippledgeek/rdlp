//! PostProcessor registry for managing and executing post-processors.
//!
//! The registry manages a collection of post-processors and executes them
//! in priority order. It handles the full post-processing pipeline including
//! cleanup of temporary files.
//!
//! # Example
//!
//! ```no_run
//! use rdlp_postprocess::{PostProcessorRegistry, PostProcessorRegistryTrait, processors::*};
//! use rdlp_core::{InfoDict, PostProcessConfig};
//! use std::path::PathBuf;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let mut registry = PostProcessorRegistry::new()?;
//!
//! // Optionally register additional processors
//! // registry.register(Box::new(MyCustomProcessor::new()));
//!
//! let info = InfoDict::new(
//!     "test123".to_string(),
//!     "Test Video".to_string(),
//!     "Test".to_string(),
//!     "https://example.com".to_string(),
//! );
//!
//! let config = PostProcessConfig::default();
//! let files = vec![PathBuf::from("video.mp4")];
//!
//! let result = registry.process(&info, files, &config).await?;
//! println!("Output files: {:?}", result.files);
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, info, warn};
use rdlp_core::{InfoDict, PostProcessConfig, PostProcessResult, PostProcessor};

use crate::error::Result;
use crate::ffmpeg::{FFmpegRunner, RemuxOptions};
use crate::processors::*;

/// Trait for post-processor registry operations.
#[async_trait]
pub trait PostProcessorRegistryTrait: Send + Sync {
    /// Process files through all applicable post-processors.
    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
    ) -> anyhow::Result<PostProcessResult>;

    /// Remux a file with faststart enabled (stream copy, no re-encoding).
    ///
    /// Moves the moov atom to the beginning of the file for progressive
    /// playback and fixes container structure/timestamps.
    async fn remux_faststart(&self, input: &Path, output: &Path) -> anyhow::Result<()>;

    /// List all registered post-processors.
    fn list_processors(&self) -> Vec<String>;

    /// Get a post-processor by name.
    fn get_processor(&self, name: &str) -> Option<Arc<dyn PostProcessor>>;
}

/// Registry for managing post-processors.
pub struct PostProcessorRegistry {
    /// Registered post-processors
    processors: Vec<Arc<dyn PostProcessor>>,
    /// Shared FFmpeg runner
    ffmpeg: Arc<FFmpegRunner>,
}

impl PostProcessorRegistry {
    /// Create a new registry with default processors.
    ///
    /// This will auto-detect FFmpeg and register all built-in processors.
    pub fn new() -> Result<Self> {
        Self::with_ffmpeg_location(None)
    }

    /// Create a new registry with a custom FFmpeg location.
    pub fn with_ffmpeg_location(location: Option<&std::path::Path>) -> Result<Self> {
        let ffmpeg = Arc::new(FFmpegRunner::with_location(location)?);
        let mut registry = Self {
            processors: Vec::new(),
            ffmpeg,
        };

        // Register default processors in priority order
        registry.register_defaults();

        Ok(registry)
    }

    /// Create a registry without FFmpeg (for testing or non-FFmpeg operations).
    ///
    /// Note: FFmpeg library initialization will still be attempted.
    /// Operations will fail at runtime if the library is not available.
    pub fn without_ffmpeg() -> Result<Self> {
        Ok(Self {
            processors: Vec::new(),
            ffmpeg: Arc::new(FFmpegRunner::new()?),
        })
    }

    /// Register default post-processors.
    fn register_defaults(&mut self) {
        // Register in reverse priority order (highest priority first)
        // Priority 100: Merge video+audio streams
        self.register(Arc::new(FFmpegMerger::new(self.ffmpeg.clone())));

        // Priority 50: Extract audio
        self.register(Arc::new(FFmpegExtractAudio::new(self.ffmpeg.clone())));

        // Priority 45: Container remuxing (TS → MP4/MKV for better seeking)
        self.register(Arc::new(FFmpegRemuxer::new(self.ffmpeg.clone())));

        // Priority 40: Video conversion/remuxing
        self.register(Arc::new(FFmpegVideoConvertor::new(self.ffmpeg.clone())));

        // Priority 30: Metadata embedding
        self.register(Arc::new(FFmpegMetadata::new(self.ffmpeg.clone())));

        // Priority 20: Thumbnail embedding
        self.register(Arc::new(EmbedThumbnail::new(self.ffmpeg.clone())));
    }

    /// Register a post-processor.
    pub fn register(&mut self, processor: Arc<dyn PostProcessor>) {
        debug!(name:? = processor.name(); "Registering post-processor");
        self.processors.push(processor);
    }

    /// Get the FFmpeg runner.
    pub fn ffmpeg(&self) -> &Arc<FFmpegRunner> {
        &self.ffmpeg
    }

    /// Sort processors by priority (highest first).
    fn sorted_processors(&self) -> Vec<Arc<dyn PostProcessor>> {
        let mut sorted = self.processors.clone();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.priority()));
        sorted
    }
}

#[async_trait]
impl PostProcessorRegistryTrait for PostProcessorRegistry {
    async fn process(
        &self,
        info: &InfoDict,
        files: Vec<PathBuf>,
        config: &PostProcessConfig,
    ) -> anyhow::Result<PostProcessResult> {
        let mut current_info = info.clone();
        let mut current_files = files;
        let mut all_temp_files = Vec::new();

        let processors = self.sorted_processors();

        for processor in &processors {
            if !processor.should_run(&current_info, config) {
                debug!(name:? = processor.name(); "Skipping post-processor (should_run returned false)");
                continue;
            }

            info!(name:? = processor.name(); "Running post-processor");

            match processor
                .process(&current_info, current_files.clone(), config)
                .await
            {
                Ok(result) => {
                    current_info = result.info;
                    current_files = result.files;
                    all_temp_files.extend(result.temp_files);

                    debug!(
                        name:? = processor.name(),
                        output_files = current_files.len();
                        "Post-processor completed"
                    );
                }
                Err(e) => {
                    warn!(name:? = processor.name(); "Post-processor failed: {e}");
                    // Continue with other processors on non-fatal errors
                    // Fatal errors should be propagated
                    if is_fatal_error(&e) {
                        return Err(e.into());
                    }
                }
            }
        }

        // Cleanup temporary files
        for temp_file in &all_temp_files {
            if temp_file.exists() {
                if let Err(e) = tokio::fs::remove_file(temp_file).await {
                    warn!(path:? = temp_file.display(); "Failed to remove temp file: {e}");
                } else {
                    debug!(path:? = temp_file.display(); "Removed temp file");
                }
            }
        }

        Ok(PostProcessResult {
            info: current_info,
            files: current_files,
            temp_files: Vec::new(), // Already cleaned up
        })
    }

    async fn remux_faststart(&self, input: &Path, output: &Path) -> anyhow::Result<()> {
        let opts = RemuxOptions {
            faststart: true,
            ..Default::default()
        };
        self.ffmpeg
            .remux(input, output, &opts)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn list_processors(&self) -> Vec<String> {
        self.processors
            .iter()
            .map(|p| p.name().to_string())
            .collect()
    }

    fn get_processor(&self, name: &str) -> Option<Arc<dyn PostProcessor>> {
        self.processors
            .iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
            .cloned()
    }
}

/// Check if an error is fatal and should stop processing.
fn is_fatal_error(error: &rdlp_core::RdlpError) -> bool {
    // Check for fatal error types
    matches!(
        error,
        rdlp_core::RdlpError::FFmpeg(msg) if msg.contains("not found"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        // This test only works if FFmpeg library is available
        if let Ok(registry) = PostProcessorRegistry::new() {
            let processors = registry.list_processors();
            assert!(!processors.is_empty());
        }
    }

    #[test]
    fn test_processor_priority_order() {
        if let Ok(registry) = PostProcessorRegistry::new() {
            let sorted = registry.sorted_processors();

            // Verify processors are sorted by priority (highest first)
            for i in 1..sorted.len() {
                assert!(
                    sorted[i - 1].priority() >= sorted[i].priority(),
                    "Processors not sorted by priority"
                );
            }
        }
    }
}
