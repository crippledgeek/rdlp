//! Audio extraction: stream copy and transcoding.
//!
//! Provides `extract_audio` (async entry point) plus synchronous helpers for
//! copy-mode extraction and transcode-mode extraction with filter graph
//! sample format/rate conversion.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types (sample rate,
//!   timestamps). All casts are audited and within valid ranges for
//!   `FFmpeg`-returned values.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use log::{debug, info};
use tokio_util::sync::CancellationToken;

use crate::error::{PostProcessError, Result};

use super::super::ffi_helpers::cleanup_partial_output;
use super::super::log_capture::LogSuppressGuard;
use super::super::salvage::prepare_input_with_salvage;
use super::super::{AudioExtractOptions, FFmpegRunner, ensure_init};
use super::mux_timing::{MuxTimingState, flush_interleave_queue};

impl FFmpegRunner {
    /// Extract audio from a media file, either by stream copy or transcoding.
    ///
    /// Uses `opts.copy` to determine whether to copy or transcode.
    /// For transcoding, supports bitrate (CBR) and quality scale (VBR) modes.
    ///
    /// Automatically detects and salvages corrupt Matroska/WebM containers
    /// before extraction to prevent EBML-induced muxer failures.
    ///
    /// # Errors
    ///
    /// Returns an error if probing, decoding, encoding, or muxing fails —
    /// including I/O errors, unsupported codec errors, and ENOMEM during
    /// mux write.
    pub async fn extract_audio(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &AudioExtractOptions,
        progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>>,
        cancel: Option<CancellationToken>,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("extract_audio", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, true)?;

            let result = Self::extract_audio_sync(
                &effective_input,
                &output,
                &opts,
                progress_fn.as_deref(),
                cancel.as_ref(),
            );

            if let Some(ref temp) = salvage_temp {
                // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
                #[allow(clippy::disallowed_methods)]
                let _ = std::fs::remove_file(temp);
            }

            Ok(result?)
        })
        .await
    }

    /// Extract audio synchronously (dispatches to copy or transcode).
    fn extract_audio_sync(
        input: &Path,
        output: &Path,
        opts: &AudioExtractOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        if opts.copy {
            Self::extract_audio_copy_sync(input, output, progress_fn, cancel)
        } else {
            Self::extract_audio_transcode_sync(input, output, opts, progress_fn, cancel)
        }
    }

    /// Extract audio by stream copy (no re-encoding).
    ///
    /// Maps only the best audio stream from input to output without transcoding.
    fn extract_audio_copy_sync(
        input: &Path,
        output: &Path,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        ensure_init()?;

        // Suppress FFmpeg's internal muxer trace/debug spam while keeping errors visible.
        let _log_suppress = LogSuppressGuard::error_level();

        let mut ictx = ffmpeg_the_third::format::input(input)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open input for audio copy extract {}",
                    input.display()
                )
            })?;

        let input_duration_us: i64 = unsafe { (*ictx.as_mut_ptr()).duration };

        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to create output for audio copy extract {}",
                    output.display()
                )
            })?;

        // Find best audio stream
        let ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let ist_time_base = ictx
            .stream(ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
            })?
            .time_base();

        // Add output stream (stream copy mode)
        Self::add_stream_copy(
            &mut octx,
            ictx.stream(ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {ist_index} not found"
                    ))
                })?
                .parameters(),
            "for audio copy extract",
        )
        .inspect_err(|_| cleanup_partial_output(output))?;

        // Set format-level encoding_tool metadata (copy, no re-encoding)
        crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, "copy");

        octx.write_header()
            .map_err(PostProcessError::from)
            .context("failed to write output header for audio copy extract")?;

        let mut last_progress = Instant::now();
        let throttle = Duration::from_millis(100);

        // Copy only audio packets
        for result in ictx.packets() {
            crate::ffmpeg::transcode::check_cancelled(cancel)?;
            let (stream, mut packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read packet during audio copy extract")?;
            if stream.index() != ist_index {
                continue;
            }
            // PTS-based progress
            if let Some(ref progress) = progress_fn
                && input_duration_us > 0
                && last_progress.elapsed() >= throttle
                && let Some(pts) = packet.pts()
            {
                let pts_us = pts * i64::from(ist_time_base.numerator()) * 1_000_000
                    / i64::from(ist_time_base.denominator());
                let frac = (pts_us as f64 / input_duration_us as f64).clamp(0.0, 1.0);
                progress(frac);
                last_progress = Instant::now();
            }
            let ost_time_base = octx
                .stream(0)
                .ok_or_else(|| PostProcessError::ffmpeg_failed("output stream 0 not found"))?
                .time_base();
            packet.rescale_ts(ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(0);
            packet
                .write_interleaved(&mut octx)
                .map_err(PostProcessError::from)
                .context("failed to write packet during audio copy extract")?;
        }

        if let Some(ref progress) = progress_fn {
            progress(1.0);
        }

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for audio copy extract")?;

        Ok(())
    }

    /// Extract audio by transcoding to a target codec.
    ///
    /// Decodes the input audio, optionally converts sample format/rate through
    /// a filter graph, and encodes to the target codec.
    #[allow(clippy::too_many_lines)]
    fn extract_audio_transcode_sync(
        input: &Path,
        output: &Path,
        opts: &AudioExtractOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
        ensure_init()?;

        // Suppress FFmpeg's internal muxer trace/debug spam while keeping errors visible.
        let _log_suppress = LogSuppressGuard::error_level();

        // Open input and find audio stream
        let mut ictx = ffmpeg_the_third::format::input(input)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open input for audio transcode {}",
                    input.display()
                )
            })?;

        let input_duration_us: i64 = unsafe { (*ictx.as_mut_ptr()).duration };

        let ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let ist_time_base = ictx
            .stream(ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
            })?
            .time_base();

        // Create decoder (bind stream to extend its lifetime for parameters())
        let ist = ictx.stream(ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!("audio input stream {ist_index} not found"))
        })?;
        let decoder_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(ist.parameters())?;
        let mut decoder = decoder_ctx.decoder().audio()?;

        // Open output
        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to create output for audio transcode {}",
                    output.display()
                )
            })?;

        // Find encoder codec
        let enc_codec = if let Some(ref name) = opts.encoder_name {
            ffmpeg_the_third::encoder::find_by_name(name).ok_or_else(|| {
                PostProcessError::UnsupportedCodec {
                    codec: name.clone(),
                    operation: "audio extraction".into(),
                }
            })?
        } else {
            let codec_id = octx
                .format()
                .codec(output, ffmpeg_the_third::media::Type::Audio);
            ffmpeg_the_third::encoder::find(codec_id).ok_or_else(|| {
                PostProcessError::ffmpeg_failed("no default encoder for output format")
            })?
        };

        // Check global header flag BEFORE taking mutable stream borrow
        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        // Add output stream and create encoder context (scoped to release octx borrow)
        let ost_index;
        let enc_context;
        {
            let ost = octx
                .add_stream(enc_codec)
                .map_err(PostProcessError::from)
                .context("failed to add output stream for audio transcode")?;
            ost_index = ost.index();
            enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }
        // ost dropped -- octx no longer mutably borrowed

        // Configure encoder
        let mut audio_encoder = enc_context.encoder().audio()?;

        let target_format = Self::pick_audio_sample_format(&enc_codec, decoder.format());
        audio_encoder.set_format(target_format);
        let target_rate = Self::pick_audio_sample_rate(&enc_codec, decoder.rate());
        if target_rate != decoder.rate() {
            debug!(
                "Resampling {}→{} Hz (encoder does not support source rate)",
                decoder.rate(),
                target_rate,
            );
        }
        audio_encoder.set_rate(target_rate as i32);
        let enc_time_base = ffmpeg_the_third::Rational(1, target_rate as i32);
        audio_encoder.set_time_base(enc_time_base);

        // Set channel layout from decoder (default layout matching channel count)
        let channels = decoder.ch_layout().channels();
        // SAFETY: audio_encoder is a valid pre-open encoder context.
        Self::set_default_channel_layout(unsafe { audio_encoder.as_mut_ptr() }, channels as i32);

        // Set bitrate (CBR)
        if let Some(br_kbps) = opts.bitrate_kbps {
            audio_encoder.set_bit_rate((br_kbps as usize) * 1000);
        }

        // Set VBR quality
        if let Some(quality) = opts.quality_scale {
            // SAFETY: audio_encoder is a valid pre-open encoder context.
            Self::set_vbr_quality(unsafe { audio_encoder.as_mut_ptr() }, quality);
        }

        // Set global header flag if required by output format
        if needs_global_header {
            // SAFETY: audio_encoder is a valid pre-open encoder context.
            Self::set_global_header_flag(unsafe { audio_encoder.as_mut_ptr() });
        }

        // Open encoder
        let mut audio_encoder = audio_encoder
            .open_as(enc_codec)
            .map_err(PostProcessError::from)
            .context("failed to open audio encoder for transcode")?;

        // Read actual encoder time_base via FFI (may differ from configured after open)
        // SAFETY: audio_encoder is a valid opened encoder context.
        let enc_time_base = unsafe {
            let tb = (*audio_encoder.as_ptr()).time_base;
            ffmpeg_the_third::Rational(tb.num, tb.den)
        };
        debug!(
            "Encoder time_base: configured=1/{}, actual={}/{}",
            decoder.rate(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
        );

        // Copy encoder parameters back to output stream
        // SAFETY: audio_encoder is a valid opened encoder context.
        Self::copy_encoder_params_to_stream(&mut octx, ost_index, unsafe {
            audio_encoder.as_ptr()
        });

        // Set format-level encoding_tool metadata
        let enc_display_name = enc_codec.name();
        crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, enc_display_name);

        // Set per-stream encoder tag on audio output stream
        crate::ffmpeg::encoding_tag::set_stream_encoder(&mut octx, ost_index, enc_display_name);

        octx.write_header()
            .map_err(PostProcessError::from)
            .context("failed to write output header for audio transcode")?;

        // Read ost_time_base AFTER write_header (Matroska may change it)
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| PostProcessError::ffmpeg_failed("output stream not found"))?
            .time_base();

        let expected_duration = if audio_encoder.frame_size() > 0 {
            unsafe {
                ffmpeg_the_third::ffi::av_rescale_q(
                    i64::from(audio_encoder.frame_size()),
                    ffmpeg_the_third::ffi::AVRational {
                        num: enc_time_base.numerator(),
                        den: enc_time_base.denominator(),
                    },
                    ffmpeg_the_third::ffi::AVRational {
                        num: ost_time_base.numerator(),
                        den: ost_time_base.denominator(),
                    },
                )
            }
        } else {
            0
        };
        info!(
            "Expected audio packet duration: {} (frame_size={}, enc_tb={}/{}, ost_tb={}/{})",
            expected_duration,
            audio_encoder.frame_size(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
            ost_time_base.numerator(),
            ost_time_base.denominator(),
        );

        // Build filter graph for sample format/rate conversion
        let mut filter_graph = Self::build_audio_filter(&decoder, &audio_encoder, ist_time_base)?;

        let mut timing = MuxTimingState {
            encoder_frame_size: audio_encoder.frame_size(),
            expected_duration,
            ..Default::default()
        };

        let mut last_progress = Instant::now();
        let progress_throttle = Duration::from_millis(100);

        // Transcode loop: read -> decode -> filter -> encode -> write
        for result in ictx.packets() {
            crate::ffmpeg::transcode::check_cancelled(cancel)?;
            let (stream, packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read packet during audio transcode")?;
            if stream.index() != ist_index {
                continue;
            }
            // PTS-based progress
            if let Some(ref progress) = progress_fn
                && input_duration_us > 0
                && last_progress.elapsed() >= progress_throttle
                && let Some(pts) = packet.pts()
            {
                let pts_us = pts * i64::from(ist_time_base.numerator()) * 1_000_000
                    / i64::from(ist_time_base.denominator());
                let frac = (pts_us as f64 / input_duration_us as f64).clamp(0.0, 1.0);
                progress(frac);
                last_progress = Instant::now();
            }
            decoder.send_packet(&packet)?;
            Self::receive_and_process_audio(
                &mut decoder,
                &mut filter_graph,
                &mut audio_encoder,
                &mut octx,
                ost_index,
                enc_time_base,
                &mut timing,
            )?;
        }
        // Emit final 1.0 on completion
        if let Some(ref progress) = progress_fn {
            progress(1.0);
        }

        // Flush decoder
        decoder.send_eof()?;
        Self::receive_and_process_audio(
            &mut decoder,
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            ost_index,
            enc_time_base,
            &mut timing,
        )?;

        // Flush filter graph (signal EOF to source)
        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_filter_to_encoder(
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            ost_index,
            enc_time_base,
            &mut timing,
        )?;

        // Flush encoder
        audio_encoder.send_eof()?;
        Self::drain_encoder_packets(
            &mut audio_encoder,
            &mut octx,
            ost_index,
            enc_time_base,
            &mut timing,
        )?;

        // Flush interleave queue before trailer
        flush_interleave_queue(&mut octx);

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for audio transcode")?;

        Ok(())
    }

    /// Pick a sample format supported by the encoder, preferring the decoder's format.
    pub(crate) fn pick_audio_sample_format(
        codec: &ffmpeg_the_third::Codec,
        preferred: ffmpeg_the_third::format::Sample,
    ) -> ffmpeg_the_third::format::Sample {
        // Check codec's supported sample formats
        unsafe {
            let ptr = codec.as_ptr();
            let sample_fmts = (*ptr).sample_fmts;
            if sample_fmts.is_null() {
                // Codec accepts any format
                return preferred;
            }

            let mut i = 0;
            let mut first = None;
            loop {
                let fmt = *sample_fmts.offset(i);
                if fmt == ffmpeg_the_third::ffi::AVSampleFormat::AV_SAMPLE_FMT_NONE {
                    break;
                }
                let sample = ffmpeg_the_third::format::Sample::from(fmt);
                first.get_or_insert(sample);
                if sample == preferred {
                    return preferred;
                }
                i += 1;
            }

            first.unwrap_or(preferred)
        }
    }

    /// Build an audio filter graph for sample format/rate/channel conversion.
    ///
    /// Uses `abuffer` -> `aformat` -> `abuffersink` to let `FFmpeg` handle any
    /// necessary sample format, sample rate, or channel layout conversions.
    /// Delegates to the shared filter graph helper in normalize/helpers.
    fn build_audio_filter(
        decoder: &ffmpeg_the_third::decoder::Audio,
        encoder: &ffmpeg_the_third::encoder::audio::Audio,
        ist_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let enc_ch_layout_desc = encoder.ch_layout().description();
        let aformat_spec = format!(
            "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
            encoder.format().name(),
            encoder.rate(),
            enc_ch_layout_desc,
        );

        crate::ffmpeg::normalize::helpers::build_audio_filter_with_spec(
            decoder,
            ist_time_base,
            &aformat_spec,
        )
    }
}
