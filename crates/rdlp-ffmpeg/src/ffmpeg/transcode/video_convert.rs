//! Video conversion: remux and transcoding.
//!
//! Provides `convert_video` (async entry point) plus synchronous helpers for
//! video transcoding with filter graph pixel format conversion, and video
//! encoder packet writing.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::debug;

use crate::error::{PostProcessError, Result};

use super::super::salvage::prepare_input_with_salvage;
use super::super::{FFmpegRunner, RemuxOptions, VideoConvertOptions, ensure_init};
use super::mux_timing::flush_interleave_queue;

impl FFmpegRunner {
    /// Convert a video file, either by remuxing or transcoding.
    ///
    /// Uses `opts.remux_only` to determine whether to stream-copy or transcode.
    /// For transcoding, encodes video with the specified codec while optionally
    /// copying the audio stream unchanged.
    ///
    /// Automatically detects and salvages corrupt Matroska/WebM containers
    /// before conversion to prevent EBML-induced muxer failures.
    pub async fn convert_video(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &VideoConvertOptions,
        progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>>,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("convert_video", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, true)?;

            let result =
                Self::convert_video_sync(&effective_input, &output, &opts, progress_fn.as_deref());

            if let Some(ref temp) = salvage_temp {
                let _ = std::fs::remove_file(temp);
            }

            result
        })
        .await
    }

    /// Convert video synchronously (dispatches to remux or transcode).
    fn convert_video_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
    ) -> Result<()> {
        if opts.remux_only {
            // Determine if output is MP4/MOV for faststart
            let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("");
            let remux_opts = RemuxOptions {
                faststart: ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov"),
                ..Default::default()
            };
            Self::remux_sync(input, output, &remux_opts, progress_fn)
        } else {
            Self::convert_video_transcode_sync(input, output, opts, progress_fn)
        }
    }

    /// Transcode video to a target codec, optionally copying audio.
    ///
    /// Decodes video frames, converts pixel format through a filter graph,
    /// and encodes with the target video codec. Audio is stream-copied if
    /// `opts.audio_copy` is true.
    #[allow(clippy::too_many_lines)]
    fn convert_video_transcode_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
    ) -> Result<()> {
        ensure_init()?;

        // Open input
        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let input_duration_us: i64 = unsafe { (*ictx.as_mut_ptr()).duration };

        // Find video and audio stream indices
        let video_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoVideoStream)?;

        let audio_ist_index = ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index());

        // Capture stream time bases before any mutable borrows
        let video_ist_time_base = ictx
            .stream(video_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?
            .time_base();
        let video_ist_frame_rate = ictx
            .stream(video_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?
            .avg_frame_rate();
        let audio_ist_time_base = audio_ist_index
            .map(|i| {
                ictx.stream(i).map(|s| s.time_base()).ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("audio input stream {i} not found"))
                })
            })
            .transpose()?;

        // Create video decoder
        let video_ist = ictx.stream(video_ist_index).ok_or_else(|| {
            PostProcessError::ffmpeg_failed(format!(
                "video input stream {video_ist_index} not found"
            ))
        })?;
        let video_dec_ctx =
            ffmpeg_the_third::codec::context::Context::from_parameters(video_ist.parameters())?;
        let mut video_decoder = video_dec_ctx.decoder().video()?;

        // Open output
        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Find video encoder
        let video_codec_name = opts.video_codec.as_deref().unwrap_or("libx264");
        let video_enc_codec = ffmpeg_the_third::encoder::find_by_name(video_codec_name)
            .ok_or_else(|| PostProcessError::UnsupportedCodec {
                codec: video_codec_name.to_string(),
                operation: "video conversion".into(),
            })?;

        // Check global header flag before mutable stream borrows
        let needs_global_header = octx
            .format()
            .flags()
            .contains(ffmpeg_the_third::format::Flags::GLOBAL_HEADER);

        // Add video output stream (scoped to release octx borrow)
        let video_ost_index;
        let video_enc_context;
        {
            let ost = octx.add_stream(video_enc_codec).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add video output stream: {e}"),
                }
            })?;
            video_ost_index = ost.index();
            video_enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }

        // Configure video encoder
        let mut video_encoder = video_enc_context.encoder().video()?;
        video_encoder.set_width(video_decoder.width());
        video_encoder.set_height(video_decoder.height());

        let target_pix_fmt =
            Self::pick_video_pixel_format(&video_enc_codec, video_decoder.format());
        video_encoder.set_format(target_pix_fmt);

        // Set time base from frame rate (inverse of fps)
        if video_ist_frame_rate.numerator() > 0 && video_ist_frame_rate.denominator() > 0 {
            video_encoder.set_time_base(ffmpeg_the_third::Rational(
                video_ist_frame_rate.denominator(),
                video_ist_frame_rate.numerator(),
            ));
        } else {
            video_encoder.set_time_base(video_ist_time_base);
        }

        // Set frame rate
        video_encoder.set_frame_rate(Some(video_ist_frame_rate));

        if needs_global_header {
            // SAFETY: video_encoder is a valid pre-open encoder context.
            Self::set_global_header_flag(unsafe { video_encoder.as_mut_ptr() });
        }

        // Open encoder with preset/CRF options
        let mut enc_opts = ffmpeg_the_third::Dictionary::new();
        if let Some(ref preset) = opts.preset {
            enc_opts.set("preset", preset);
        }
        if let Some(crf) = opts.crf {
            enc_opts.set("crf", &crf.to_string());
        }

        // For VP9: set bitrate to 0 for pure CRF mode
        if video_codec_name.contains("vpx") && opts.crf.is_some() {
            video_encoder.set_bit_rate(0);
        }

        let mut video_encoder = video_encoder
            .open_as_with(video_enc_codec, enc_opts)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to open video encoder: {e}"),
            })?;

        // Copy encoder parameters back to output stream
        // SAFETY: video_encoder is a valid opened encoder context.
        Self::copy_encoder_params_to_stream(&mut octx, video_ost_index, unsafe {
            video_encoder.as_ptr()
        });

        // Determine audio handling mode:
        // - audio_copy=true → stream copy (existing path)
        // - audio_codec=Some → re-encode with specified encoder
        // - neither → no audio output stream
        let audio_encode_codec: Option<&str> = if !opts.audio_copy {
            opts.audio_codec.as_deref()
        } else {
            None
        };

        // Add audio output stream (stream copy) if audio exists and copy requested
        let audio_ost_index = if opts.audio_copy {
            if let Some(audio_idx) = audio_ist_index {
                let audio_ost_idx;
                {
                    let mut ost = octx
                        .add_stream(ffmpeg_the_third::encoder::find(
                            ffmpeg_the_third::codec::Id::None,
                        ))
                        .map_err(|e| PostProcessError::FFmpegLibraryError {
                            message: format!("failed to add audio output stream: {e}"),
                        })?;
                    ost.set_parameters(
                        ictx.stream(audio_idx)
                            .ok_or_else(|| {
                                PostProcessError::ffmpeg_failed(format!(
                                    "audio input stream {audio_idx} not found"
                                ))
                            })?
                            .parameters(),
                    );
                    audio_ost_idx = ost.index();
                    Self::clear_codec_tag(ost.parameters().as_ptr());
                }
                Some(audio_ost_idx)
            } else {
                None
            }
        } else {
            None
        };

        // Audio transcode: open decoder + encoder when audio_codec is specified
        let audio_transcode_state: Option<(
            ffmpeg_the_third::decoder::Audio,
            ffmpeg_the_third::encoder::audio::Encoder,
            ffmpeg_the_third::Rational, // encoder time_base
            usize,                      // audio_ost_index for transcode
        )> = if let Some(enc_name) = audio_encode_codec {
            if let Some(audio_idx) = audio_ist_index {
                // Open audio decoder
                let audio_ist = ictx.stream(audio_idx).ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {audio_idx} not found"
                    ))
                })?;
                let audio_dec_ctx = ffmpeg_the_third::codec::context::Context::from_parameters(
                    audio_ist.parameters(),
                )?;
                let mut audio_decoder = audio_dec_ctx.decoder().audio()?;
                // Set packet timebase for accurate audio timestamps
                let pkt_tb = audio_ist_time_base.unwrap_or(ffmpeg_the_third::Rational(1, 44100));
                unsafe {
                    (*audio_decoder.as_mut_ptr()).pkt_timebase =
                        ffmpeg_the_third::ffi::AVRational {
                            num: pkt_tb.numerator(),
                            den: pkt_tb.denominator(),
                        };
                }

                // Find and open audio encoder
                let audio_enc_codec = ffmpeg_the_third::encoder::find_by_name(enc_name)
                    .ok_or_else(|| PostProcessError::UnsupportedCodec {
                        codec: enc_name.to_string(),
                        operation: "audio re-encode during video recode".into(),
                    })?;

                // Add audio output stream with encoder
                let audio_enc_ost_idx;
                {
                    let ost = octx.add_stream(audio_enc_codec).map_err(|e| {
                        PostProcessError::FFmpegLibraryError {
                            message: format!("failed to add audio encode output stream: {e}"),
                        }
                    })?;
                    audio_enc_ost_idx = ost.index();
                }

                // Configure audio encoder from decoder properties
                let audio_enc_context = ffmpeg_the_third::codec::context::Context::from_parameters(
                    octx.stream(audio_enc_ost_idx)
                        .ok_or_else(|| {
                            PostProcessError::ffmpeg_failed("audio encode ost not found")
                        })?
                        .parameters(),
                )?;
                let mut audio_encoder = audio_enc_context.encoder().audio()?;

                // Pick sample rate compatible with encoder (prefer decoder rate)
                let target_rate =
                    Self::pick_audio_sample_rate(&audio_enc_codec, audio_decoder.rate());
                let enc_time_base = ffmpeg_the_third::Rational(1, target_rate as i32);
                audio_encoder.set_rate(target_rate as i32);
                audio_encoder.set_time_base(enc_time_base);

                // Set channel layout matching decoder channel count
                let channels = audio_decoder.ch_layout().channels();
                // SAFETY: audio_encoder is a valid pre-open encoder context.
                Self::set_default_channel_layout(
                    unsafe { audio_encoder.as_mut_ptr() },
                    channels as i32,
                );

                // Pick sample format compatible with encoder (prefer decoder format)
                let target_fmt =
                    Self::pick_audio_sample_format(&audio_enc_codec, audio_decoder.format());
                audio_encoder.set_format(target_fmt);

                if needs_global_header {
                    unsafe { Self::set_global_header_flag(audio_encoder.as_mut_ptr()) };
                }

                let audio_encoder = audio_encoder.open_as(audio_enc_codec).map_err(|e| {
                    PostProcessError::FFmpegLibraryError {
                        message: format!("failed to open audio encoder '{enc_name}': {e}"),
                    }
                })?;

                // Copy encoder parameters back to output stream
                unsafe {
                    Self::copy_encoder_params_to_stream(
                        &mut octx,
                        audio_enc_ost_idx,
                        audio_encoder.as_ptr(),
                    );
                }

                Some((
                    audio_decoder,
                    audio_encoder,
                    enc_time_base,
                    audio_enc_ost_idx,
                ))
            } else {
                None
            }
        } else {
            None
        };

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MKV: set cluster_time_limit for smoother playback/seeking in players like VLC
        let is_mkv = output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mkv"));
        if is_mkv {
            dict.set("cluster_time_limit", "500");
            debug!("MKV detected, setting cluster_time_limit=500ms via dictionary");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Build video filter graph for pixel format conversion
        let mut filter_graph =
            Self::build_video_filter(&video_decoder, &video_encoder, video_ist_time_base)?;

        let mut last_progress = Instant::now();
        let progress_throttle = Duration::from_millis(100);

        // Destructure transcode state to allow mutable borrows in loop
        let (
            mut audio_transcode_decoder,
            mut audio_transcode_encoder,
            audio_transcode_enc_tb,
            audio_transcode_ost_idx,
        ) = match audio_transcode_state {
            Some((dec, enc, enc_tb, idx)) => (Some(dec), Some(enc), Some(enc_tb), Some(idx)),
            None => (None, None, None, None),
        };

        // Process packets: video -> decode/filter/encode, audio -> copy or transcode
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            let ist_index = stream.index();

            if ist_index == video_ist_index {
                // PTS-based progress from video stream
                if let Some(ref progress) = progress_fn
                    && input_duration_us > 0
                    && last_progress.elapsed() >= progress_throttle
                    && let Some(pts) = packet.pts()
                {
                    let tb = video_ist_time_base;
                    let pts_us =
                        pts * i64::from(tb.numerator()) * 1_000_000 / i64::from(tb.denominator());
                    let frac = (pts_us as f64 / input_duration_us as f64).clamp(0.0, 1.0);
                    progress(frac);
                    last_progress = Instant::now();
                }
                // Video: decode -> filter -> encode -> write
                video_decoder.send_packet(&packet)?;
                Self::receive_and_process_video(
                    &mut video_decoder,
                    &mut filter_graph,
                    &mut video_encoder,
                    &mut octx,
                    video_ost_index,
                )?;
            } else if Some(ist_index) == audio_ist_index {
                // Audio: stream copy or transcode
                if let Some(audio_ost_idx) = audio_ost_index {
                    // Stream copy path
                    let ost_time_base = octx
                        .stream(audio_ost_idx)
                        .ok_or_else(|| {
                            PostProcessError::ffmpeg_failed(format!(
                                "audio output stream {audio_ost_idx} not found"
                            ))
                        })?
                        .time_base();
                    let audio_tb = audio_ist_time_base.ok_or_else(|| {
                        PostProcessError::ffmpeg_failed("audio input time base not available")
                    })?;
                    packet.rescale_ts(audio_tb, ost_time_base);
                    packet.set_position(-1);
                    packet.set_stream(audio_ost_idx);
                    packet.write_interleaved(&mut octx).map_err(|e| {
                        PostProcessError::FFmpegLibraryError {
                            message: format!("failed to write audio packet: {e}"),
                        }
                    })?;
                } else if let (
                    Some(ref mut audio_dec),
                    Some(ref mut audio_enc),
                    Some(enc_tb),
                    Some(audio_ost_idx),
                ) = (
                    audio_transcode_decoder.as_mut(),
                    audio_transcode_encoder.as_mut(),
                    audio_transcode_enc_tb,
                    audio_transcode_ost_idx,
                ) {
                    // Audio transcode path: decode → encode → write
                    audio_dec.send_packet(&packet)?;
                    Self::drain_audio_transcode(
                        audio_dec,
                        audio_enc,
                        &mut octx,
                        enc_tb,
                        audio_ost_idx,
                    )?;
                }
            }
        }
        // Emit final 1.0 on completion
        if let Some(ref progress) = progress_fn {
            progress(1.0);
        }

        // Flush video decoder
        video_decoder.send_eof()?;
        Self::receive_and_process_video(
            &mut video_decoder,
            &mut filter_graph,
            &mut video_encoder,
            &mut octx,
            video_ost_index,
        )?;

        // Flush video filter graph
        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("video filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_video_filter_to_encoder(
            &mut filter_graph,
            &mut video_encoder,
            &mut octx,
            video_ost_index,
        )?;

        // Flush video encoder
        video_encoder.send_eof()?;
        Self::drain_video_encoder_packets(&mut video_encoder, &mut octx, video_ost_index)?;

        // Flush audio encoder (transcode path)
        if let (
            Some(ref mut audio_dec),
            Some(ref mut audio_enc),
            Some(enc_tb),
            Some(audio_ost_idx),
        ) = (
            audio_transcode_decoder.as_mut(),
            audio_transcode_encoder.as_mut(),
            audio_transcode_enc_tb,
            audio_transcode_ost_idx,
        ) {
            audio_dec.send_eof()?;
            Self::drain_audio_transcode(audio_dec, audio_enc, &mut octx, enc_tb, audio_ost_idx)?;
            audio_enc.send_eof()?;
            Self::drain_audio_encoder_packets(audio_enc, &mut octx, enc_tb, audio_ost_idx)?;
        }

        // Flush interleave queue before trailer
        flush_interleave_queue(&mut octx);

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Receive decoded audio frames and encode them to the output stream.
    ///
    /// Used for the audio transcode path in `convert_video_transcode_sync`
    /// when `audio_codec` is specified in [`VideoConvertOptions`].
    fn drain_audio_transcode(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        encoder: &mut ffmpeg_the_third::encoder::audio::Encoder,
        octx: &mut ffmpeg_the_third::format::context::Output,
        enc_time_base: ffmpeg_the_third::Rational,
        ost_index: usize,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            encoder.send_frame(&frame)?;
            Self::drain_audio_encoder_packets(encoder, octx, enc_time_base, ost_index)?;
        }
        Ok(())
    }

    /// Drain encoded audio packets to the output context (interleaved write).
    fn drain_audio_encoder_packets(
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
