//! Whether the post-processing pipeline is usable — and when it is not, why.
//!
//! Absence used to be a bare `Option<Arc<Pipeline>>`, so the two ways to be
//! absent were indistinguishable downstream and both were reported as
//! "`FFmpeg` NOT found". They are not the same condition and do not have the
//! same remedy (rdlp#727):
//!
//! * `FFmpeg` is not installed — degrade gracefully, as rdlp always has.
//! * `FFmpeg` is installed and refuses to be called, because its ABI
//!   disagrees with the bindings this binary carries (rdlp#656). The operator
//!   asked for post-processing that is not going to happen, and the remedy
//!   names two versions rather than a missing package.

use std::sync::Arc;

use rdlp_ffmpeg::ffmpeg::abi::AbiMismatches;
use rdlp_postprocess::pipeline::Pipeline;

/// The post-processing pipeline, or the reason there isn't one.
#[derive(Clone)]
pub enum PipelineAvailability {
    /// `FFmpeg` initialized and the pipeline is built.
    Ready(Arc<Pipeline>),
    /// `FFmpeg` could not be initialized at all — typically not installed.
    FfmpegUnavailable,
    /// `FFmpeg` is present but its ABI disagrees with this binary's bindings,
    /// so calling into it would read struct fields at offsets that do not
    /// exist. Carries the mismatch set so its remedy reaches the user intact.
    AbiMismatch(AbiMismatches),
}

impl PipelineAvailability {
    /// The pipeline, when there is one.
    pub const fn pipeline(&self) -> Option<&Arc<Pipeline>> {
        match self {
            Self::Ready(pipeline) => Some(pipeline),
            Self::FfmpegUnavailable | Self::AbiMismatch(_) => None,
        }
    }

    /// Whether post-processing can run at all.
    ///
    /// Format selection asks this to decide whether merging separate video and
    /// audio streams is on the table.
    pub const fn is_ready(&self) -> bool {
        self.pipeline().is_some()
    }

    /// The ABI mismatch that makes the pipeline unusable, if that is why.
    pub const fn abi_mismatch(&self) -> Option<&AbiMismatches> {
        match self {
            Self::AbiMismatch(mismatches) => Some(mismatches),
            Self::Ready(_) | Self::FfmpegUnavailable => None,
        }
    }
}

impl std::fmt::Debug for PipelineAvailability {
    // Hand-written because `Pipeline` is not `Debug`; the variant is the part
    // a reader of a log line needs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready(_) => f.write_str("Ready"),
            Self::FfmpegUnavailable => f.write_str("FfmpegUnavailable"),
            Self::AbiMismatch(mismatches) => write!(f, "AbiMismatch({mismatches})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rdlp_ffmpeg::ffmpeg::abi::{
        AbiMismatch, AbiVersion, BuildPrefix, FfmpegLibrary, MismatchKind,
    };

    /// A mismatch set standing in for a partially-upgraded system.
    fn mismatches() -> AbiMismatches {
        AbiMismatches::new(
            vec![AbiMismatch {
                library: FfmpegLibrary::Avcodec,
                kind: MismatchKind::DifferentMajor,
                compiled: AbiVersion::new(62, 11),
                linked: AbiVersion::new(63, 1),
            }],
            BuildPrefix::from_env_value("/home/user/.local/mediaforge"),
        )
        .expect("a non-empty mismatch list is a mismatch")
    }

    #[test]
    fn abi_mismatch_has_no_pipeline() {
        let availability = PipelineAvailability::AbiMismatch(mismatches());

        assert!(
            availability.pipeline().is_none(),
            "an ABI-skewed FFmpeg must never hand out a pipeline"
        );
        assert!(!availability.is_ready());
    }

    #[test]
    fn abi_mismatch_is_distinguishable_from_a_missing_ffmpeg() {
        assert!(
            PipelineAvailability::AbiMismatch(mismatches())
                .abi_mismatch()
                .is_some(),
            "the mismatch must survive to the call site that reports it"
        );
        assert!(
            PipelineAvailability::FfmpegUnavailable
                .abi_mismatch()
                .is_none(),
            "a missing FFmpeg is not an ABI mismatch — that conflation is rdlp#727"
        );
    }

    #[test]
    fn unavailable_ffmpeg_has_no_pipeline_and_no_mismatch() {
        let availability = PipelineAvailability::FfmpegUnavailable;

        assert!(availability.pipeline().is_none());
        assert!(!availability.is_ready());
        assert!(availability.abi_mismatch().is_none());
    }

    #[test]
    fn debug_carries_the_remedy_for_a_mismatch() {
        let rendered = format!("{:?}", PipelineAvailability::AbiMismatch(mismatches()));

        assert!(
            rendered.contains("regenerate the bindings"),
            "the remedy must not be summarised away: {rendered}"
        );
        assert_eq!(
            format!("{:?}", PipelineAvailability::FfmpegUnavailable),
            "FfmpegUnavailable"
        );
    }
}
