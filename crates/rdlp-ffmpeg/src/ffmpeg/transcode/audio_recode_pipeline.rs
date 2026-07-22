//! Audio encode/mux pipeline used DURING video recode.
//!
//! Provides 3 drain helpers (`drain_audio_transcode_filtered`,
//! `drain_audio_filter_to_encoder`, `drain_audio_encoder_packets`) that
//! participate in the audio side of `convert_video_transcode_sync` when
//! `opts.audio_codec` is set. The filter graph these drain from is built by
//! the shared `build_encoder_adapted_audio_filter` in
//! `ffi_helpers/filter_graph.rs`.
//!
//! Sibling of `video_pipeline.rs` (video-side helpers for the same
//! recode flow) and `audio_pipeline.rs` (audio helpers for the standalone
//! `extract_audio` flow). Extracted from `video_convert.rs` to fix the
//! asymmetry where audio-during-recode helpers lived inside the video
//! conversion file.
//!
//! # Lint allowances
//!
//! - `clippy::cast_possible_truncation` / `clippy::cast_possible_wrap`:
//!   `frame.samples() as i64` widens `usize` to `i64` for PTS accumulation.

#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

use crate::error::{PostProcessError, Result};

use super::super::FFmpegRunner;

impl FFmpegRunner {
    /// Decode audio frames → push through filter → encode → write.
    ///
    /// `sample_counter` tracks cumulative decoded samples to generate monotonic
    /// PTS, overriding source timestamps that may contain discontinuities
    /// (common in streaming site downloads). Without this, a source timestamp
    /// jump produces a burst of near-identical DTS values in the output,
    /// causing audible desync.
    ///
    /// The counter is in sample units (`1/sample_rate`) and rescaled to the
    /// filter graph's input `time_base` via `Rescale::rescale`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn drain_audio_transcode_filtered(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Encoder,
        octx: &mut ffmpeg_the_third::format::context::Output,
        enc_time_base: ffmpeg_the_third::Rational,
        ost_index: usize,
        sample_counter: &mut i64,
        sample_rate: i32,
        input_time_base: ffmpeg_the_third::Rational,
    ) -> Result<()> {
        use ffmpeg_the_third::Rescale as _;

        let sample_tb = ffmpeg_the_third::Rational(1, sample_rate);
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            // Override source PTS with monotonic counter to handle
            // timestamp discontinuities in the source stream.
            // Convert from sample units (1/sample_rate) to the filter
            // graph's input time_base.
            let pts = sample_counter.rescale(sample_tb, input_time_base);
            frame.set_pts(Some(pts));
            *sample_counter += frame.samples() as i64;

            // Push decoded frame into filter graph
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("audio filter 'in' not found"))?
                .source()
                .add(&frame)?;

            // Pull filtered frames and encode
            Self::drain_audio_filter_to_encoder(filter, encoder, octx, enc_time_base, ost_index)?;
        }
        Ok(())
    }

    /// Pull frames from audio filter graph, encode, and write.
    pub(super) fn drain_audio_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Encoder,
        octx: &mut ffmpeg_the_third::format::context::Output,
        enc_time_base: ffmpeg_the_third::Rational,
        ost_index: usize,
    ) -> Result<()> {
        let mut filtered_frame = ffmpeg_the_third::frame::Audio::empty();
        while filter
            .get("out")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("audio filter 'out' not found"))?
            .sink()
            .frame(&mut filtered_frame)
            .is_ok()
        {
            encoder.send_frame(&filtered_frame)?;
            Self::drain_audio_encoder_packets(encoder, octx, enc_time_base, ost_index)?;
        }
        Ok(())
    }

    /// Drain encoded audio packets to the output context (interleaved write).
    pub(super) fn drain_audio_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::audio::Encoder,
        octx: &mut ffmpeg_the_third::format::context::Output,
        enc_time_base: ffmpeg_the_third::Rational,
        ost_index: usize,
    ) -> Result<()> {
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio ost {ost_index} not found"))
            })?
            .time_base();
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.rescale_ts(enc_time_base, ost_time_base);
            packet.set_stream(ost_index);
            packet.set_position(-1);
            packet
                .write_interleaved(octx)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write encoded audio packet: {e}"),
                })?;
        }
        Ok(())
    }
}
