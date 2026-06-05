//! Phase state types for the video transcode pipeline.
//!
//! Carries data across the 6 phases of `convert_video_transcode_sync`
//! (orchestrator in `video_convert.rs`; phase methods in
//! `video_transcode_phases.rs`).

use std::path::Path;

use super::super::VideoConvertOptions;

/// Outputs of Phase 1 (`open_input_and_decoder`).
///
/// Passed by value into Phase 2 (`configure_video_encoder`), which uses
/// these fields plus its own newly-created state to construct the full
/// `VideoTranscodeContext`.
pub(super) struct Phase1Outputs {
    pub(super) ictx: ffmpeg_the_third::format::context::Input,
    pub(super) video_decoder: ffmpeg_the_third::decoder::Video,
    pub(super) video_ist_index: usize,
    pub(super) audio_ist_index: Option<usize>,
    pub(super) video_ist_time_base: ffmpeg_the_third::Rational,
    pub(super) video_ist_frame_rate: ffmpeg_the_third::Rational,
    pub(super) audio_ist_time_base: Option<ffmpeg_the_third::Rational>,
    pub(super) input_duration_us: i64,
}

/// State carried across audio re-encode phases (Phase 3 setup, Phase 5 loop,
/// Phase 6 finalize). Replaces the prior 6-tuple
/// `Option<(decoder::Audio, encoder::audio::Encoder, Rational, usize, filter::Graph, i32)>`.
pub(super) struct AudioTranscodeState {
    pub(super) decoder: ffmpeg_the_third::decoder::Audio,
    pub(super) encoder: ffmpeg_the_third::encoder::audio::Encoder,
    pub(super) enc_time_base: ffmpeg_the_third::Rational,
    pub(super) ost_index: usize,
    pub(super) filter: ffmpeg_the_third::filter::Graph,
    pub(super) input_sample_rate: i32,
}

/// Mutable state threaded across the six phases of video transcoding.
/// Constructed at the end of Phase 2 (`configure_video_encoder`) and
/// dropped at the end of Phase 6 (`finalize_transcode`).
///
/// `filter_graph` is initialized to `filter::Graph::default()` (empty)
/// in Phase 2 and overwritten by Phase 4 (`write_header_and_build_filter`).
/// `audio_copy_ost_index` and `audio_transcode` are `Option<T>` because
/// they're genuinely conditional (set by Phase 3 only when `audio_copy`
/// or `audio_codec` is requested respectively).
pub(super) struct VideoTranscodeContext<'a> {
    // Phase 1 outputs (always present once ctx exists):
    pub(super) ictx: ffmpeg_the_third::format::context::Input,
    pub(super) video_decoder: ffmpeg_the_third::decoder::Video,
    pub(super) video_ist_index: usize,
    pub(super) audio_ist_index: Option<usize>,
    pub(super) video_ist_time_base: ffmpeg_the_third::Rational,
    pub(super) audio_ist_time_base: Option<ffmpeg_the_third::Rational>,
    pub(super) input_duration_us: i64,

    // Phase 2 outputs (always present once ctx exists):
    pub(super) octx: ffmpeg_the_third::format::context::Output,
    pub(super) video_encoder: ffmpeg_the_third::encoder::video::Encoder,
    pub(super) video_ost_index: usize,
    pub(super) video_enc_time_base: ffmpeg_the_third::Rational,
    pub(super) needs_global_header: bool,

    // Phase 3 outputs (mutually exclusive; one or both may be None):
    pub(super) audio_copy_ost_index: Option<usize>,
    pub(super) audio_transcode: Option<AudioTranscodeState>,

    // Phase 4 output (initialized empty at Phase 2, populated in Phase 4):
    pub(super) filter_graph: ffmpeg_the_third::filter::Graph,

    // Phase 5 / Phase 6 shared state: monotonic audio PTS counter (in sample units).
    // Initialized to 0 in Phase 2; incremented in Phase 5's encode loop; consumed
    // by Phase 6's audio flush.
    pub(super) audio_sample_counter: i64,

    // Borrowed config:
    pub(super) opts: &'a VideoConvertOptions,
    pub(super) output_path: &'a Path,
}
