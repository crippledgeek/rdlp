//! MetadataStage stub — to be implemented.
use async_trait::async_trait;
use crate::pipeline::{PipelineMessage, PipelineStage};

/// Embeds metadata (title, artist, chapters) into the output file.
pub struct MetadataStage;
#[async_trait]
impl PipelineStage for MetadataStage {
    fn name(&self) -> &str { "metadata" }
    fn should_run(&self, _: &PipelineMessage) -> bool { false }
    fn is_fatal(&self) -> bool { false }
    async fn process(&self, msg: PipelineMessage) -> anyhow::Result<PipelineMessage> { Ok(msg) }
}
