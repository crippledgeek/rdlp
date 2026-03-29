//! Stage implementations for the post-processing pipeline.
//!
//! Stages are executed in fixed index order (not priority):
//! 0: MergeStage → 1: AudioExtractStage → 2: NormalizeStage → 3: RemuxStage →
//! 4: RecodeStage → 5: SubtitleStage → 6: MetadataStage → 7: ThumbnailStage → 8: FixupStage

pub mod audio_extract;
pub mod fixup;
pub mod merge;
pub mod metadata;
pub mod normalize;
pub mod recode;
pub mod remux;
pub mod subtitle;
pub mod thumbnail;

pub use audio_extract::AudioExtractStage;
pub use fixup::FixupStage;
pub use merge::MergeStage;
pub use metadata::MetadataStage;
pub use normalize::NormalizeStage;
pub use recode::RecodeStage;
pub use remux::RemuxStage;
pub use subtitle::SubtitleStage;
pub use thumbnail::ThumbnailStage;
