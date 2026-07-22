//! The single encoder-adapting audio filter graph (#638).
//!
//! Both audio routes — standalone `extract_audio` and the audio side of a
//! video recode — must hand the encoder frames it will actually accept. That
//! means two adaptations, and an encoder rejects the frame with a bare
//! `EINVAL` from `avcodec_send_frame` if either is missing:
//!
//! 1. **Sample format / rate / channel layout**, via `aformat`.
//! 2. **Frame size** — most audio encoders have a fixed `frame_size` (AAC
//!    1024, MP3 1152, Opus 960, FLAC 4608 on this build) and reject any
//!    non-final frame of a different length. `av_buffersink_set_frame_size`
//!    makes the sink re-chunk filtered audio to exactly that size.
//!
//! Before #638 these were three separate implementations — normalize
//! (`normalize/encode.rs`), recode (`audio_recode_pipeline.rs`), and extract
//! (`audio_extract.rs`) — and only extract omitted step 2. That is why
//! `--extract-audio` worked for AAC alone: the one target whose frame size
//! already matched the AAC decoder's 1024-sample output. All three now share
//! `set_buffersink_frame_size`, and the two transcode routes share this whole
//! builder, so the adaptation cannot go missing on one route again.

use crate::error::Result;

use super::super::FFmpegRunner;

impl FFmpegRunner {
    /// Build an `abuffer → aformat → abuffersink` graph that adapts decoded
    /// frames to everything `encoder` requires.
    ///
    /// `encoder` must be **opened** — `frame_size` is only populated by
    /// `avcodec_open2`, which is precisely why this takes
    /// `encoder::audio::Encoder` rather than the pre-open
    /// `encoder::audio::Audio`.
    ///
    /// The channel layout comes from the encoder's own `ch_layout` (set from
    /// the decoder's channel count before open at both call sites), so
    /// layouts beyond mono/stereo are described correctly.
    ///
    /// # Errors
    ///
    /// Returns an error if a filter is missing from the build or the graph
    /// fails to parse, configure, or validate.
    pub(crate) fn build_encoder_adapted_audio_filter(
        decoder: &ffmpeg_the_third::decoder::Audio,
        encoder: &ffmpeg_the_third::encoder::audio::Encoder,
        input_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let aformat_spec = format!(
            "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
            encoder.format().name(),
            encoder.rate(),
            encoder.ch_layout().description(),
        );

        let mut graph = crate::ffmpeg::normalize::helpers::build_audio_filter_with_spec(
            decoder,
            input_time_base,
            &aformat_spec,
        )?;

        Self::set_buffersink_frame_size(&mut graph, "out", encoder.frame_size());

        Ok(graph)
    }
}
