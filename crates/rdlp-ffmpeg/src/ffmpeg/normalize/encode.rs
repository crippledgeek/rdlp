//! Audio encoding pipeline for normalization.
//!
//! Contains the unified decode → filter → encode → mux pipeline shared by
//! peak normalization and loudnorm pass 2.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types (sample rate as
//!   `u32`/`i32`, timestamps as `i64`/`usize`). All casts are audited and within
//!   valid ranges for `FFmpeg`-returned values.
//! - `clippy::unwrap_used`: channel-count and sample-rate accessors on a validated
//!   open decoder context are guaranteed non-zero; panicking signals a programming
//!   error (the decoder should have been validated before this point).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::unwrap_used,
    clippy::similar_names,  // dec_ctx / enc_ctx / ofmt_ctx are standard FFmpeg naming
)]

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use log::{debug, warn};
use tokio_util::sync::CancellationToken;

use crate::error::{FfmpegResultExt as _, PostProcessError};

use super::super::FFmpegRunner;
use super::super::ffi_helpers::set_single_thread_codec;
use super::super::log_capture::LogSuppressGuard;
use super::super::salvage::open_input_resilient;
use super::super::transcode::MuxTimingState;
use super::helpers::default_bitrate_for_encoder;
use super::io_diag::validate_mux_header_state;
use crate::ffmpeg::ffi_helpers::filter_graph::{AudioSinkSpec, build_audio_filter_graph};

/// Recurring plumbing passed through the normalize encode helpers.
///
/// Bundles the progress callback and cancel token that thread unchanged
/// from `dispatch_normalize_sync` down into `encode_audio_only_sync`,
/// keeping each helper's data-param list within the clippy argument limit.
pub(super) struct EncodeCallCtx<'a> {
    /// Optional 0.0–1.0 progress callback (PTS-based + final 1.0 on completion).
    pub progress_fn: Option<&'a (dyn Fn(f64) + Send + Sync)>,
    /// Optional cooperative-cancellation token checked per input packet.
    pub cancel: Option<&'a CancellationToken>,
}

impl FFmpegRunner {
    /// Unified audio-only encode: decode → filter → encode → mux.
    ///
    /// Both peak normalization and loudnorm pass 2 share this pipeline.
    /// The only difference is the filter chain, built by `build_filter`
    /// which receives the encoder's sample format name, sample rate, and
    /// channel layout description.
    ///
    /// `label` appears in log and error messages (e.g. "peak encode",
    /// "loudnorm pass 2").
    ///
    /// When `resilient` is true, the input is opened with
    /// `discardcorrupt+genpts` format flags to recover from corrupt
    /// containers. This is used as Tier 3 recovery in `with_mux_retry`.
    #[allow(clippy::too_many_lines)]
    pub(super) fn encode_audio_only_sync(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        label: &str,
        resilient: bool,
        ctx: &EncodeCallCtx<'_>,
        build_filter: impl FnOnce(
            /*fmt:*/ &str,
            /*rate:*/ u32,
            /*ch_layout:*/ &str,
        ) -> String,
    ) -> anyhow::Result<()> {
        let EncodeCallCtx {
            progress_fn,
            cancel,
        } = *ctx;
        crate::ffmpeg::ensure_init()?;

        let mut ictx = if resilient {
            debug!("[{label}] opening input with resilient flags (discardcorrupt+genpts)");
            open_input_resilient(input)?
        } else {
            ffmpeg_the_third::format::input(input)
                .map_err(PostProcessError::from)
                .with_context(|| {
                    format!("failed to open input for audio encode {}", input.display())
                })?
        };

        // Read the format-level start_time and duration (in AV_TIME_BASE = µs) before
        // borrowing individual streams.  HLS downloads and some containers
        // have a non-zero start time.  We must preserve this offset in the
        // sample-clock DTS synthesis so the normalized audio aligns with
        // the original video stream when they are merged back together.
        let format_start_time_us: i64 = unsafe { (*ictx.as_mut_ptr()).start_time };
        let input_duration_us: i64 = unsafe { (*ictx.as_mut_ptr()).duration };

        let audio_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist = ictx.stream(audio_ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "audio input stream {audio_ist_index} not found"
            ))
        })?;
        let audio_ist_time_base = audio_ist.time_base();

        let mut audio_dec_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(audio_ist.parameters())?;
        set_single_thread_codec(unsafe { audio_dec_ctx.as_mut_ptr() });
        let mut audio_decoder = audio_dec_ctx.decoder().audio()?;

        let input_audio_bitrate = audio_ist.parameters().bit_rate() as usize;

        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to create output for audio encode {}",
                    output.display()
                )
            })?;

        let enc_name = super::helpers::resolve_normalize_audio_encoder(final_output_ext, label)?;
        let enc_codec = ffmpeg_the_third::encoder::find_by_name(enc_name).ok_or_else(|| {
            PostProcessError::UnsupportedCodec {
                codec: enc_name.to_string(),
                operation: format!("audio normalization ({label})"),
            }
        })?;

        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        let audio_ost_index;
        let audio_enc_context;
        {
            let ost = octx
                .add_stream(enc_codec)
                .ff_context("failed to add audio output stream for encode")?;
            audio_ost_index = ost.index();
            audio_enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }

        let mut audio_encoder = audio_enc_context.encoder().audio()?;
        let target_format = Self::pick_audio_sample_format(&enc_codec, audio_decoder.format());
        audio_encoder.set_format(target_format);
        let target_rate = Self::pick_audio_sample_rate(&enc_codec, audio_decoder.rate());
        if target_rate != audio_decoder.rate() {
            debug!(
                "[{label}] resampling {}→{} Hz (encoder does not support source rate)",
                audio_decoder.rate(),
                target_rate,
            );
        }
        audio_encoder.set_rate(target_rate as i32);
        let enc_time_base = ffmpeg_the_third::Rational(1, target_rate as i32);
        audio_encoder.set_time_base(enc_time_base);

        let channels = audio_decoder.ch_layout().channels();
        Self::set_default_channel_layout(unsafe { audio_encoder.as_mut_ptr() }, channels as i32);

        let target_bitrate = if input_audio_bitrate > 0 {
            input_audio_bitrate
        } else {
            default_bitrate_for_encoder(enc_name)
        };
        audio_encoder.set_bit_rate(target_bitrate);

        if needs_global_header {
            Self::set_global_header_flag(unsafe { audio_encoder.as_mut_ptr() });
        }

        set_single_thread_codec(unsafe { audio_encoder.as_mut_ptr() });

        // Enable afterburner for libfdk_aac (higher quality, ~10% slower)
        if enc_name == "libfdk_aac" {
            unsafe {
                let key = std::ffi::CString::new("afterburner").unwrap();
                let val = std::ffi::CString::new("1").unwrap();
                ffmpeg_the_third::ffi::av_opt_set(
                    audio_encoder.as_mut_ptr().cast(),
                    key.as_ptr(),
                    val.as_ptr(),
                    ffmpeg_the_third::ffi::AV_OPT_SEARCH_CHILDREN,
                );
            }
            debug!("[{label}] libfdk_aac: afterburner enabled");
        }

        let mut audio_encoder = audio_encoder
            .open_as(enc_codec)
            .ff_context("failed to open audio encoder")?;

        let enc_time_base = unsafe {
            let tb = (*audio_encoder.as_ptr()).time_base;
            ffmpeg_the_third::Rational(tb.num, tb.den)
        };
        debug!(
            "[{label}] encoder time_base: configured=1/{}, actual={}/{}",
            audio_decoder.rate(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
        );

        Self::copy_encoder_params_to_stream(&mut octx, audio_ost_index, unsafe {
            audio_encoder.as_ptr()
        });

        unsafe {
            let ctx = octx.as_mut_ptr();
            (*ctx).avoid_negative_ts = ffmpeg_the_third::ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
            (*ctx).flags |= ffmpeg_the_third::ffi::AVFMT_FLAG_FLUSH_PACKETS;
            (*ctx).max_interleave_delta = 0;
        }

        // Set format-level encoding_tool metadata
        crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, enc_name);

        // Set per-stream encoder tag on audio output stream
        crate::ffmpeg::encoding_tag::set_stream_encoder(&mut octx, audio_ost_index, enc_name);

        let mut muxer_opts = ffmpeg_the_third::Dictionary::new();
        muxer_opts.set("cluster_time_limit", "500");
        octx.write_header_with(muxer_opts)
            .ff_context("failed to write output header for audio encode")?;

        // Validate mux header state (threading, IO, seekability, file size)
        unsafe {
            validate_mux_header_state(&mut octx, &audio_decoder, &audio_encoder, output, label)?;
        }

        // Build filter chain via caller-supplied closure.
        let enc_ch_layout_desc = audio_encoder.ch_layout().description();
        let filter_spec = build_filter(
            audio_encoder.format().name(),
            audio_encoder.rate(),
            &enc_ch_layout_desc,
        );
        debug!("[{label}] filter_spec={filter_spec}");

        debug!(
            "[{label}] decoder: sample_rate={}, format={}, ch_layout={}",
            audio_decoder.rate(),
            audio_decoder.format().name(),
            audio_decoder.ch_layout().description(),
        );
        debug!(
            "[{label}] encoder: sample_rate={}, format={}, ch_layout={}",
            audio_encoder.rate(),
            audio_encoder.format().name(),
            enc_ch_layout_desc,
        );

        let mut filter_graph = build_audio_filter_graph(
            &audio_decoder,
            audio_ist_time_base,
            AudioSinkSpec {
                filter_spec: &filter_spec,
                frame_size: audio_encoder.frame_size(),
            },
        )?;

        Self::discard_non_audio_streams(&mut ictx, audio_ist_index);

        let ost_time_base = octx
            .stream(audio_ost_index)
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
        debug!(
            "Expected audio packet duration: {} (frame_size={}, enc_tb={}/{}, ost_tb={}/{})",
            expected_duration,
            audio_encoder.frame_size(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
            ost_time_base.numerator(),
            ost_time_base.denominator(),
        );

        let log_suppress = LogSuppressGuard::error_level();

        // Compute the starting sample offset from the input's format-level
        // start_time.  When the source has a non-zero start (e.g. HLS downloads
        // starting mid-stream), this ensures the normalized audio output
        // preserves the same temporal offset, preventing audio/video desync
        // in the subsequent merge step.
        let enc_rate = audio_encoder.rate();
        let start_samples = if format_start_time_us > 0
            && format_start_time_us != ffmpeg_the_third::ffi::AV_NOPTS_VALUE
        {
            format_start_time_us * i64::from(enc_rate) / 1_000_000
        } else {
            0
        };
        if start_samples > 0 {
            debug!(
                "[{label}] preserving input start offset: {format_start_time_us} µs = {start_samples} samples",
            );
        }

        let mut timing = MuxTimingState {
            encoder_frame_size: audio_encoder.frame_size(),
            expected_duration,
            sample_rate: enc_rate,
            use_sample_clock: true,
            samples_written: start_samples,
            ..Default::default()
        };

        // Transcode loop (audio only)
        let mut packets_processed = 0u64;
        let mut packets_skipped = 0u64;
        let mut last_progress = Instant::now();
        let progress_throttle = Duration::from_millis(100);
        for result in ictx.packets() {
            crate::ffmpeg::transcode::check_cancelled(cancel)?;
            let (stream, packet) =
                result.ff_context("failed to read packet during audio encode")?;
            if stream.index() != audio_ist_index {
                continue;
            }
            // PTS-based progress: rescale packet PTS to µs, compare against total duration
            if let Some(ref progress) = progress_fn
                && input_duration_us > 0
                && last_progress.elapsed() >= progress_throttle
                && let Some(pts) = packet.pts()
            {
                let tb = audio_ist_time_base;
                let pts_us =
                    pts * i64::from(tb.numerator()) * 1_000_000 / i64::from(tb.denominator());
                let frac = (pts_us as f64 / input_duration_us as f64).clamp(0.0, 1.0);
                progress(frac);
                last_progress = Instant::now();
            }
            if let Err(e) = audio_decoder.send_packet(&packet) {
                if packets_skipped == 0 {
                    if matches!(&e, ffmpeg_the_third::Error::Other { errno } if *errno == 12) {
                        warn!(
                            "Audio decoder allocation failure (ENOMEM) — \
                             process may be running out of memory"
                        );
                    } else {
                        warn!("Audio decoder error (skipping affected packets): {e}");
                    }
                }
                packets_skipped += 1;
                if let Err(drain_err) = Self::receive_and_process_audio_direct(
                    &mut audio_decoder,
                    &mut filter_graph,
                    &mut audio_encoder,
                    &mut octx,
                    audio_ost_index,
                    enc_time_base,
                    &mut timing,
                    Some(output),
                ) {
                    if drain_err.is_mux_write_error() {
                        return Err(drain_err.into());
                    }
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!(
                            "({label}) mux/encode pipeline failed while \
                             draining after decoder error: {drain_err}"
                        ),
                    }
                    .into());
                }
                continue;
            }
            packets_processed += 1;
            Self::receive_and_process_audio_direct(
                &mut audio_decoder,
                &mut filter_graph,
                &mut audio_encoder,
                &mut octx,
                audio_ost_index,
                enc_time_base,
                &mut timing,
                Some(output),
            )?;
        }
        // Emit final 1.0 on completion
        if let Some(ref progress) = progress_fn {
            progress(1.0);
        }

        if packets_skipped > 0 {
            warn!(
                "Skipped {packets_skipped} of {} audio packet(s) due to decoder errors",
                packets_processed + packets_skipped,
            );
        }

        if packets_processed == 0 && packets_skipped > 0 {
            return Err(PostProcessError::NormalizationFailed {
                message: format!(
                    "audio decoder failed on all {packets_skipped} packets — cannot normalize"
                ),
            }
            .into());
        }

        // Flush
        if let Err(e) = audio_decoder.send_eof() {
            warn!("Decoder send_eof failed (continuing with flush): {e}");
        }
        Self::receive_and_process_audio_direct(
            &mut audio_decoder,
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_filter_to_encoder_direct(
            &mut filter_graph,
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        audio_encoder.send_eof()?;
        Self::drain_encoder_packets_direct(
            &mut audio_encoder,
            &mut octx,
            audio_ost_index,
            enc_time_base,
            &mut timing,
            Some(output),
        )?;

        drop(log_suppress);

        // Explicit teardown with debug-level lifecycle markers.
        // Resources are RAII-managed, but logging confirms cleanup order.
        drop(audio_encoder);
        drop(audio_decoder);
        debug!("[mem] codec contexts dropped");

        drop(filter_graph);
        debug!("[mem] filter graph freed");

        octx.write_trailer()
            .ff_context("failed to write output trailer for audio encode")?;

        drop(octx);
        drop(ictx);
        debug!("[mem] format contexts closed");

        debug!("[mem] encode_audio_only_sync complete — resources released");
        Ok(())
    }
}
