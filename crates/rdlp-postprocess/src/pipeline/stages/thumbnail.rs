//! ThumbnailStage stub — to be implemented.
use async_trait::async_trait;
use crate::pipeline::{PipelineMessage, PipelineStage};

/// Embeds a thumbnail/cover image into the output file.
pub struct ThumbnailStage;
#[async_trait]
impl PipelineStage for ThumbnailStage {
    fn name(&self) -> &str { "thumbnail" }
    fn should_run(&self, _: &PipelineMessage) -> bool { false }
    fn is_fatal(&self) -> bool { false }
    async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> { Ok(msg) }
}
