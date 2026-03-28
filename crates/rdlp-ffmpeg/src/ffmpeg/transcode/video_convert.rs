//! Video conversion: remux and transcoding.
//!
//! Provides `convert_video` (async entry point) plus synchronous helpers for
//! video transcoding with filter graph pixel format conversion, and video
//! encoder packet writing.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use log::debug;

use crate::error::{PostProcessError, Result};

use super::super::salvage::prepare_input_with_salvage;
use super::super::{FFmpegRunner, RemuxOptions, VideoConvertOptions, ensure_init};
use super::mux_timing::flush_interleave_queue;

/// Callback type for forwarding FFmpeg log lines to the UI.
type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

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
        log_fn: Option<LogFn>,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("convert_video", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, true)?;

            // Capture FFmpeg C-level logs when verbose mode is enabled.
            let log_guard = if opts.verbose {
                super::super::log_capture::LogCaptureGuard::begin().ok()
            } else {
                None
            };

            let result =
                Self::convert_video_sync(&effective_input, &output, &opts, progress_fn.as_deref());

            // Drain captured logs and forward to the UI log viewer.
            if let Some(ref guard) = log_guard
                && let Ok(lines) = guard.take_captured()
                && let Some(ref log) = log_fn
            {
                for line in lines {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        log(trimmed);
                    }
                }
            }

            if let Some(ref temp) = salvage_temp {
                let _ = std::fs::remove_file(temp);
            }

            Ok(result?)
        })
        .await
    }

    /// Convert video synchronously (dispatches to remux or transcode).
    fn convert_video_sync(
        input: &Path,
        output: &Path,
        opts: &VideoConvertOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
    ) -> anyhow::Result<()> {
        if opts.remux_only {
            // Determine if output is MP4/MOV for faststart
            let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("");
            let remux_opts = RemuxOptions {
                faststart: ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov"),
                ..Default::default()
            };
            Ok(Self::remux_sync(input, output, &remux_opts, progress_fn)
                .map_err(|e| PostProcessError::ffmpeg_failed(format!("{e:#}")))?)
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
    ) -> anyhow::Result<()> {
        ensure_init()?;

        // Open input
        let mut ictx = ffmpeg_the_third::format::input(input)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open input for video transcode {}",
                    input.display()
                )
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
        let video_ist_frame_rate = {
            let stream = ictx.stream(video_ist_index).ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?;
            let avg = stream.avg_frame_rate();
            let r_rate = unsafe {
                let r = (*stream.as_ptr()).r_frame_rate;
                ffmpeg_the_third::Rational(r.num, r.den)
            };
            // Prefer avg_frame_rate, but fall back to r_frame_rate if avg looks wrong
            // (zero, negative, or unreasonably high like 100+ fps for non-HFR content)
            let avg_fps = if avg.denominator() > 0 {
                avg.numerator() as f64 / avg.denominator() as f64
            } else {
                0.0
            };
            if avg.numerator() > 0 && avg.denominator() > 0 && avg_fps <= 120.0 {
                avg
            } else if r_rate.numerator() > 0 && r_rate.denominator() > 0 {
                debug!(
                    "video_convert: avg_frame_rate={}/{} ({avg_fps:.1} fps) looks wrong, using r_frame_rate={}/{}",
                    avg.numerator(),
                    avg.denominator(),
                    r_rate.numerator(),
                    r_rate.denominator()
                );
                r_rate
            } else {
                debug!("video_convert: both frame rates invalid, defaulting to 30fps");
                ffmpeg_the_third::Rational(30, 1)
            }
        };
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
        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to create output for video transcode {}",
                    output.display()
                )
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
        {
            let ost = octx
                .add_stream(video_enc_codec)
                .map_err(PostProcessError::from)
                .context("failed to add video output stream for transcode")?;
            video_ost_index = ost.index();
        }

        // Create encoder context WITH the codec so priv_data is allocated.
        // Using from_parameters()+NULL codec leaves priv_data unset, which
        // prevents avcodec_open2 from applying dictionary options (preset/crf)
        // to encoder-private data (libx264, libx265, etc.).
        let video_enc_context =
            ffmpeg_the_third::codec::context::Context::new_with_codec(video_enc_codec);
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

        // For VP9: set bitrate to 0 for pure CRF mode
        if video_codec_name.contains("vpx") && opts.crf.is_some() {
            video_encoder.set_bit_rate(0);
        }

        // Open encoder with preset/CRF options
        let mut enc_opts = ffmpeg_the_third::Dictionary::new();
        if let Some(ref preset) = opts.preset {
            enc_opts.set("preset", preset);
        }
        if let Some(crf) = opts.crf {
            enc_opts.set("crf", &crf.to_string());
        }
        let mut video_encoder = video_encoder
            .open_as_with(video_enc_codec, enc_opts)
            .map_err(PostProcessError::from)
            .context("failed to open video encoder for transcode")?;

        // The encoder's ctx->time_base is NOT the timebase of output packets.
        // Packets inherit the timebase from the filter graph input, which is
        // the input stream's time_base (e.g., 1/1000 for MKV). Use that for
        // rescaling packets to the output stream's time_base.
        let video_enc_time_base = video_ist_time_base;

        // Copy encoder parameters + time_base to output stream as a hint.
        // The muxer may override stream->time_base during write_header;
        // we re-read it after write_header for correct packet rescaling.
        Self::copy_encoder_params_to_stream(&mut octx, video_ost_index, unsafe {
            video_encoder.as_ptr()
        });
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(video_ost_index);
            (*stream_ptr).time_base = ffmpeg_the_third::ffi::AVRational {
                num: video_enc_time_base.numerator(),
                den: video_enc_time_base.denominator(),
            };
        }

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
                        .map_err(PostProcessError::from)
                        .context("failed to add audio output stream for video transcode")?;
                    let audio_ist = ictx.stream(audio_idx).ok_or_else(|| {
                        PostProcessError::ffmpeg_failed(format!(
                            "audio input stream {audio_idx} not found"
                        ))
                    })?;
                    ost.set_parameters(audio_ist.parameters());
                    ost.set_metadata(audio_ist.metadata().to_owned());
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

        // Audio transcode: open decoder + encoder + filter when audio_codec is specified
        let audio_transcode_state: Option<(
            ffmpeg_the_third::decoder::Audio,
            ffmpeg_the_third::encoder::audio::Encoder,
            ffmpeg_the_third::Rational,      // encoder time_base
            usize,                           // audio_ost_index for transcode
            ffmpeg_the_third::filter::Graph, // audio format conversion filter
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
                    let ost = octx
                        .add_stream(audio_enc_codec)
                        .map_err(PostProcessError::from)
                        .context("failed to add audio encode output stream for video transcode")?;
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

                let audio_encoder = audio_encoder
                    .open_as(audio_enc_codec)
                    .map_err(PostProcessError::from)
                    .with_context(|| {
                        format!("failed to open audio encoder '{enc_name}' for video transcode")
                    })?;

                // Copy encoder parameters back to output stream
                unsafe {
                    Self::copy_encoder_params_to_stream(
                        &mut octx,
                        audio_enc_ost_idx,
                        audio_encoder.as_ptr(),
                    );
                }

                // Build audio filter graph for format/rate conversion + frame size buffering
                let audio_filter = Self::build_audio_transcode_filter(
                    &audio_decoder,
                    &audio_encoder,
                    audio_ist_time_base.unwrap_or(ffmpeg_the_third::Rational(1, 48000)),
                )?;

                Some((
                    audio_decoder,
                    audio_encoder,
                    enc_time_base,
                    audio_enc_ost_idx,
                    audio_filter,
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

        // Set format-level encoding_tool metadata
        {
            let audio_component = if let Some(enc_name) = audio_encode_codec {
                enc_name
            } else if opts.audio_copy {
                "copy"
            } else {
                "none"
            };
            let tool_components = format!("{video_codec_name} + {audio_component}");
            crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, &tool_components);
        }

        // Set per-stream encoder tag on video output stream
        crate::ffmpeg::encoding_tag::set_stream_encoder(&mut octx, video_ost_index, video_codec_name);

        // Set per-stream encoder tag on audio output stream (only if re-encoding)
        if let Some((_, _, _, audio_enc_ost_idx, _)) = audio_transcode_state.as_ref()
            && let Some(enc_name) = audio_encode_codec
        {
            crate::ffmpeg::encoding_tag::set_stream_encoder(&mut octx, *audio_enc_ost_idx, enc_name);
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(PostProcessError::from)
            .context("failed to write output header for video transcode")?;

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
            mut audio_transcode_filter,
        ) = match audio_transcode_state {
            Some((dec, enc, enc_tb, idx, filter)) => {
                (Some(dec), Some(enc), Some(enc_tb), Some(idx), Some(filter))
            }
            None => (None, None, None, None, None),
        };

        // Process packets: video -> decode/filter/encode, audio -> copy or transcode
        for result in ictx.packets() {
            let (stream, mut packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read packet during video transcode")?;
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
                    video_enc_time_base,
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
                    packet
                        .write_interleaved(&mut octx)
                        .map_err(PostProcessError::from)
                        .context("failed to write audio packet during video transcode")?;
                } else if let (
                    Some(ref mut audio_dec),
                    Some(ref mut audio_enc),
                    Some(enc_tb),
                    Some(audio_ost_idx),
                    Some(ref mut audio_filter),
                ) = (
                    audio_transcode_decoder.as_mut(),
                    audio_transcode_encoder.as_mut(),
                    audio_transcode_enc_tb,
                    audio_transcode_ost_idx,
                    audio_transcode_filter.as_mut(),
                ) {
                    // Audio transcode path: decode → filter (format convert + frame size) → encode → write
                    audio_dec.send_packet(&packet)?;
                    Self::drain_audio_transcode_filtered(
                        audio_dec,
                        audio_filter,
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
            video_enc_time_base,
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
            video_enc_time_base,
        )?;

        // Flush video encoder
        video_encoder.send_eof()?;
        Self::drain_video_encoder_packets(
            &mut video_encoder,
            &mut octx,
            video_ost_index,
            video_enc_time_base,
        )?;

        // Flush audio encoder (transcode path)
        if let (
            Some(ref mut audio_dec),
            Some(ref mut audio_enc),
            Some(enc_tb),
            Some(audio_ost_idx),
            Some(ref mut audio_filter),
        ) = (
            audio_transcode_decoder.as_mut(),
            audio_transcode_encoder.as_mut(),
            audio_transcode_enc_tb,
            audio_transcode_ost_idx,
            audio_transcode_filter.as_mut(),
        ) {
            // Flush decoder
            audio_dec.send_eof()?;
            Self::drain_audio_transcode_filtered(
                audio_dec,
                audio_filter,
                audio_enc,
                &mut octx,
                enc_tb,
                audio_ost_idx,
            )?;
            // Flush filter graph
            audio_filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("audio filter 'in' not found"))?
                .source()
                .flush()?;
            Self::drain_audio_filter_to_encoder(
                audio_filter,
                audio_enc,
                &mut octx,
                enc_tb,
                audio_ost_idx,
            )?;
            // Flush encoder
            audio_enc.send_eof()?;
            Self::drain_audio_encoder_packets(audio_enc, &mut octx, enc_tb, audio_ost_idx)?;
        }

        // Flush interleave queue before trailer
        flush_interleave_queue(&mut octx);

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for video transcode")?;

        Ok(())
    }

    /// Build audio filter graph for format/rate conversion in transcode path.
    ///
    /// Uses `abuffer → aformat → abuffersink` via `parse_and_validate_filter_graph`.
    /// The buffersink frame_size is set to match the encoder's required frame size.
    fn build_audio_transcode_filter(
        decoder: &ffmpeg_the_third::decoder::Audio,
        encoder: &ffmpeg_the_third::encoder::audio::Encoder,
        input_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let mut graph = ffmpeg_the_third::filter::Graph::new();

        let sample_fmt_name = unsafe {
            let name_ptr = ffmpeg_the_third::ffi::av_get_sample_fmt_name(decoder.format().into());
            if name_ptr.is_null() {
                "fltp"
            } else {
                std::ffi::CStr::from_ptr(name_ptr)
                    .to_str()
                    .unwrap_or("fltp")
            }
        };

        let enc_fmt_name = unsafe {
            let name_ptr = ffmpeg_the_third::ffi::av_get_sample_fmt_name(encoder.format().into());
            if name_ptr.is_null() {
                "s16"
            } else {
                std::ffi::CStr::from_ptr(name_ptr).to_str().unwrap_or("s16")
            }
        };

        let channels = decoder.ch_layout().channels();
        let ch_layout = if channels == 1 { "mono" } else { "stereo" };

        // Add abuffer (input) node
        let abuffer_args = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
            input_time_base.numerator(),
            input_time_base.denominator(),
            decoder.rate(),
            sample_fmt_name,
            ch_layout,
        );
        graph
            .add(
                &ffmpeg_the_third::filter::find("abuffer")
                    .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffer filter not found"))?,
                "in",
                &abuffer_args,
            )
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to create abuffer: {e}"),
            })?;

        // Add abuffersink (output) node
        graph
            .add(
                &ffmpeg_the_third::filter::find("abuffersink").ok_or_else(|| {
                    PostProcessError::ffmpeg_failed("abuffersink filter not found")
                })?,
                "out",
                "",
            )
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to create abuffersink: {e}"),
            })?;

        // Parse filter spec: aformat converts sample format, rate, and layout
        let filter_spec = format!(
            "aformat=sample_fmts={enc_fmt_name}:sample_rates={}:channel_layouts={ch_layout},aresample",
            encoder.rate(),
        );
        Self::parse_and_validate_filter_graph(&mut graph, "in", "out", &filter_spec)?;

        // Set buffersink frame_size to match encoder's required frame size
        let frame_size = encoder.frame_size();
        if frame_size > 0
            && let Some(mut sink) = graph.get("out")
        {
            unsafe {
                ffmpeg_the_third::ffi::av_buffersink_set_frame_size(sink.as_mut_ptr(), frame_size);
            }
        }

        Ok(graph)
    }

    /// Decode audio frames → push through filter → encode → write.
    fn drain_audio_transcode_filtered(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Encoder,
        octx: &mut ffmpeg_the_third::format::context::Output,
        enc_time_base: ffmpeg_the_third::Rational,
        ost_index: usize,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
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
    fn drain_audio_filter_to_encoder(
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
