//! Audio encoding pipeline for normalization.
//!
//! Contains the unified audio-only encode function shared by peak
//! normalization and loudnorm pass 2, plus mode-specific wrappers.

use std::path::Path;

use log::{debug, info, warn};

use crate::error::{PostProcessError, Result};

use super::super::ffi_helpers::set_single_thread_codec;
use super::super::log_capture::LogSuppressGuard;
use super::super::transcode::MuxTimingState;
use super::super::{FFmpegRunner, LoudnormMeasurements, NormalizeOptions, PeakAnalysis};
use super::helpers::{
    TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD, audio_only_extension_for, build_alimiter_spec,
    build_audio_filter_with_spec, build_loudnorm_pass2_filter, cli_fallback_loudnorm,
    cli_fallback_peak, default_bitrate_for_encoder, select_audio_encoder_for_container,
    with_mux_retry,
};
use super::io_diag::validate_mux_header_state;

impl FFmpegRunner {
    /// Apply peak gain normalization: encode audio to temp, merge with video.
    pub(super) fn apply_peak_gain_sync(
        input: &Path,
        output: &Path,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        Self::dispatch_normalize_sync(
            input,
            output,
            opts.salvage,
            |inp, out, ext| Self::peak_encode_audio_only(inp, out, ext, analysis, opts),
            |f_in, f_out| cli_fallback_peak(f_in, f_out, analysis, opts),
        )
    }

    /// Encode peak-normalized audio to an output file (video streams discarded).
    fn peak_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        analysis: &PeakAnalysis,
        opts: &NormalizeOptions,
    ) -> Result<()> {
        let gain_db = opts.target_peak_db - analysis.peak_db;
        let linear_limit = 10f64.powf(opts.target_peak_db / 20.0);
        Self::encode_audio_only_sync(
            input,
            output,
            final_output_ext,
            "peak encode",
            |fmt, rate, ch_layout| {
                let oversample_prefix = if gain_db >= TRUE_PEAK_OVERSAMPLE_GAIN_THRESHOLD {
                    let rate_4x = rate * 4;
                    format!("aresample={rate_4x},")
                } else {
                    String::new()
                };
                format!(
                    "volume={gain_db:.6}dB,{oversample_prefix}aresample,\
                     alimiter=limit={linear_limit:.6}:attack=5:release=50,\
                     aformat=sample_fmts={fmt}:sample_rates={rate}:\
                     channel_layouts={ch_layout}",
                )
            },
        )
    }

    /// Loudnorm pass 2: apply normalization with measured values.
    pub(super) fn loudnorm_pass2_sync(
        input: &Path,
        output: &Path,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
    ) -> Result<()> {
        Self::dispatch_normalize_sync(
            input,
            output,
            opts.salvage,
            |inp, out, ext| Self::loudnorm_encode_audio_only(inp, out, ext, opts, measurements),
            |f_in, f_out| cli_fallback_loudnorm(f_in, f_out, opts, measurements),
        )
    }

    /// Encode loudnorm-normalized audio to an output file (video streams discarded).
    fn loudnorm_encode_audio_only(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        opts: &NormalizeOptions,
        measurements: &LoudnormMeasurements,
    ) -> Result<()> {
        let loudnorm_core = build_loudnorm_pass2_filter(opts, measurements);
        let limiter = build_alimiter_spec(opts.target_tp);
        Self::encode_audio_only_sync(
            input,
            output,
            final_output_ext,
            "loudnorm pass 2",
            |fmt, rate, ch_layout| {
                format!(
                    "aformat=sample_fmts=dbl,{loudnorm_core},aresample,\
                     {limiter},aformat=sample_fmts={fmt}:sample_rates={rate}:\
                     channel_layouts={ch_layout}",
                )
            },
        )
    }

    /// Common dispatch for normalize encode → merge with optional salvage retry.
    ///
    /// Both peak and loudnorm share this pattern: check has_video → audio-only
    /// encode to temp → merge with original video → cleanup.  When `salvage` is
    /// true, wraps the encode with `with_mux_retry` for two-tier recovery
    /// (salvage remux → CLI fallback) on mux write failures.
    fn dispatch_normalize_sync(
        input: &Path,
        output: &Path,
        salvage: bool,
        encode_fn: impl Fn(&Path, &Path, &str) -> Result<()>,
        cli_fallback_fn: impl Fn(&Path, &Path) -> Result<()>,
    ) -> Result<()> {
        crate::ffmpeg::ensure_init()?;

        let has_video = {
            let ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open input {}: {e}", input.display()),
                }
            })?;
            ictx.streams()
                .best(ffmpeg_the_third::media::Type::Video)
                .is_some()
        };

        let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mp4");

        if has_video {
            let audio_ext = audio_only_extension_for(ext);
            let temp_audio = output.with_extension(format!("norm_audio.{audio_ext}"));

            if salvage {
                with_mux_retry(
                    input,
                    &temp_audio,
                    |effective_input| encode_fn(effective_input, &temp_audio, ext),
                    |fallback_in, fallback_out| cli_fallback_fn(fallback_in, fallback_out),
                )?;
            } else {
                encode_fn(input, &temp_audio, ext)?;
            }
            let merge_result = Self::merge_sync(
                input,
                &temp_audio,
                output,
                &super::super::RemuxOptions::default(),
            );
            let _ = std::fs::remove_file(&temp_audio);
            merge_result
        } else if salvage {
            with_mux_retry(
                input,
                output,
                |effective_input| encode_fn(effective_input, output, ext),
                |fallback_in, fallback_out| cli_fallback_fn(fallback_in, fallback_out),
            )
        } else {
            encode_fn(input, output, ext)
        }
    }

    /// Unified audio-only encode: decode → filter → encode → mux.
    ///
    /// Both peak normalization and loudnorm pass 2 share this pipeline.
    /// The only difference is the filter chain, built by `build_filter`
    /// which receives the encoder's sample format name, sample rate, and
    /// channel layout description.
    ///
    /// `label` appears in log and error messages (e.g. "peak encode",
    /// "loudnorm pass 2").
    #[allow(clippy::too_many_lines)]
    fn encode_audio_only_sync(
        input: &Path,
        output: &Path,
        final_output_ext: &str,
        label: &str,
        build_filter: impl FnOnce(
            /*fmt:*/ &str,
            /*rate:*/ u32,
            /*ch_layout:*/ &str,
        ) -> String,
    ) -> Result<()> {
        crate::ffmpeg::ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let audio_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist_time_base = ictx
            .stream(audio_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "audio input stream {audio_ist_index} not found"
                ))
            })?
            .time_base();

        let audio_ist = ictx.stream(audio_ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "audio input stream {audio_ist_index} not found"
            ))
        })?;
        let mut audio_dec_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(audio_ist.parameters())?;
        set_single_thread_codec(unsafe { audio_dec_ctx.as_mut_ptr() });
        let mut audio_decoder = audio_dec_ctx.decoder().audio()?;

        let input_audio_bitrate = audio_ist.parameters().bit_rate() as usize;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        let enc_name = select_audio_encoder_for_container(final_output_ext);
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
            let ost =
                octx.add_stream(enc_codec)
                    .map_err(|e| PostProcessError::FFmpegLibraryError {
                        message: format!("failed to add audio output stream: {e}"),
                    })?;
            audio_ost_index = ost.index();
            audio_enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }

        let mut audio_encoder = audio_enc_context.encoder().audio()?;
        let target_format = Self::pick_audio_sample_format(&enc_codec, audio_decoder.format());
        audio_encoder.set_format(target_format);
        audio_encoder.set_rate(audio_decoder.rate() as i32);
        let enc_time_base = ffmpeg_the_third::Rational(1, audio_decoder.rate() as i32);
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

        let mut audio_encoder =
            audio_encoder
                .open_as(enc_codec)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open audio encoder: {e}"),
                })?;

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
            (*octx.as_mut_ptr()).avoid_negative_ts =
                ffmpeg_the_third::ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
        }

        unsafe {
            (*octx.as_mut_ptr()).flags |= ffmpeg_the_third::ffi::AVFMT_FLAG_FLUSH_PACKETS;
        }

        unsafe {
            (*octx.as_mut_ptr()).max_interleave_delta = 0;
        }

        let mut muxer_opts = ffmpeg_the_third::Dictionary::new();
        muxer_opts.set("cluster_time_limit", "500");
        octx.write_header_with(muxer_opts)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

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
        info!("[{label}] filter_spec={filter_spec}");

        info!(
            "[{label}] decoder: sample_rate={}, format={}, ch_layout={}",
            audio_decoder.rate(),
            audio_decoder.format().name(),
            audio_decoder.ch_layout().description(),
        );
        info!(
            "[{label}] encoder: sample_rate={}, format={}, ch_layout={}",
            audio_encoder.rate(),
            audio_encoder.format().name(),
            enc_ch_layout_desc,
        );

        let mut filter_graph =
            build_audio_filter_with_spec(&audio_decoder, audio_ist_time_base, &filter_spec)?;

        Self::set_buffersink_frame_size(&mut filter_graph, "out", audio_encoder.frame_size());

        FFmpegRunner::discard_non_audio_streams(&mut ictx, audio_ist_index);

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
        info!(
            "Expected audio packet duration: {} (frame_size={}, enc_tb={}/{}, ost_tb={}/{})",
            expected_duration,
            audio_encoder.frame_size(),
            enc_time_base.numerator(),
            enc_time_base.denominator(),
            ost_time_base.numerator(),
            ost_time_base.denominator(),
        );

        let _log_suppress = LogSuppressGuard::error_level();

        let mut timing = MuxTimingState {
            encoder_frame_size: audio_encoder.frame_size(),
            expected_duration,
            sample_rate: audio_decoder.rate(),
            use_sample_clock: true,
            ..Default::default()
        };

        // Transcode loop (audio only)
        let mut packets_processed = 0u64;
        let mut packets_skipped = 0u64;
        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
            })?;
            if stream.index() != audio_ist_index {
                continue;
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
                        return Err(drain_err);
                    }
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!(
                            "({label}) mux/encode pipeline failed while \
                             draining after decoder error: {drain_err}"
                        ),
                    });
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
            });
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

        drop(_log_suppress);

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }
}
