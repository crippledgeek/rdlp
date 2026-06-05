//! The 6 phase methods of `convert_video_transcode_sync`.
//!
//! Orchestrator lives in `video_convert.rs`. Phase-state types live in
//! `video_transcode_context.rs`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::expect_used,
    clippy::similar_names,      // dec_ctx / enc_ctx / vid_ctx are standard FFmpeg naming
    clippy::option_if_let_else, // complex closures are clearer as if let
)]

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use log::debug;

use crate::error::PostProcessError;

use super::super::{FFmpegRunner, VideoConvertOptions};
use super::mux_timing::flush_interleave_queue;
use super::video_transcode_context::{AudioTranscodeState, Phase1Outputs, VideoTranscodeContext};

impl FFmpegRunner {
    /// Phase 1: open the input container, find video/audio stream indices,
    /// capture their time bases and the video stream's frame rate, and
    /// create the video decoder.
    pub(super) fn open_input_and_decoder(input: &Path) -> anyhow::Result<Phase1Outputs> {
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
                f64::from(avg.numerator()) / f64::from(avg.denominator())
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
        let video_decoder = video_dec_ctx.decoder().video()?;

        Ok(Phase1Outputs {
            ictx,
            video_decoder,
            video_ist_index,
            audio_ist_index,
            video_ist_time_base,
            video_ist_frame_rate,
            audio_ist_time_base,
            input_duration_us,
        })
    }

    /// Phase 2: open the output container, find and configure the video
    /// encoder, copy encoder parameters to the output stream, and construct
    /// the `VideoTranscodeContext` that flows through the remaining phases.
    pub(super) fn configure_video_encoder<'a>(
        phase1: Phase1Outputs,
        opts: &'a VideoConvertOptions,
        output_path: &'a Path,
    ) -> anyhow::Result<VideoTranscodeContext<'a>> {
        let Phase1Outputs {
            ictx,
            video_decoder,
            video_ist_index,
            audio_ist_index,
            video_ist_time_base,
            video_ist_frame_rate,
            audio_ist_time_base,
            input_duration_us,
        } = phase1;
        let output = output_path;

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

        // Propagate color/signal metadata decoder -> encoder so recode preserves the
        // source's color_range / primaries / transfer / matrix (else the output gets
        // the encoder's unspecified defaults: washed-out levels on full-range sources,
        // wrong matrix on SD/BT.601). Matches the FFmpeg CLI's color-preserving
        // behaviour (the CLI reaches the same outcome via filtergraph-negotiated
        // frame props rather than a direct field poke).
        //
        // The decoder context is the correct source ONLY because our filtergraph is
        // pixel-format-only (`buffer -> format -> buffersink`), which leaves color
        // metadata unchanged. INVARIANT: if a color-converting filter (scale/zscale/
        // colorspace) is ever added, this source MUST switch to the buffersink output
        // (av_buffersink_get_color_*), or the output would be mistagged.
        //
        // `field_order` is intentionally NOT copied: recode output through this
        // progressive pipeline is progressive, so copying an interlaced source's
        // field_order would mislabel the output.
        // SAFETY: both contexts are valid pre-open codec contexts.
        unsafe {
            let dec = video_decoder.as_ptr();
            let enc = video_encoder.as_mut_ptr();
            (*enc).color_range = (*dec).color_range;
            (*enc).color_primaries = (*dec).color_primaries;
            (*enc).color_trc = (*dec).color_trc;
            (*enc).colorspace = (*dec).colorspace;
            (*enc).chroma_sample_location = (*dec).chroma_sample_location;
            (*enc).sample_aspect_ratio = (*dec).sample_aspect_ratio;
        }

        // Encoder + output packets operate in the frame-rate tick base (1/fps).
        // libvvenc (H.266) and libxavs2 (AVS2) compute packet `dts` in a frame-tick
        // model (1 tick = 1 frame, driven by `framerate`, NOT `avctx->time_base`)
        // while echoing the input `cts` (= frame->pts) verbatim. The FFmpeg CLI
        // keeps these consistent because its buffersink delivers CFR frames already
        // in 1/fps (fftools sets `enc->time_base = frame->time_base`, and the frame
        // pts ARE 0,1,2,…). rdlp's buffersink emits frames in the input stream tb
        // (e.g. 1/12800 → pts 0,512,1024,…), so feeding those as `cts` makes vvenc/
        // xavs2 emit `dts` in a different scale than `pts` → `pts < dts` at the
        // muxer (EINVAL). Fix: encoder tb = 1/fps, and rescale each filtered frame's
        // pts from the buffersink tb (`video_ist_time_base`) to 1/fps before
        // `send_frame` (see `drain_video_filter_to_encoder`). `video_enc_time_base`
        // (defined here, = enc tb) is BOTH the frame-pts rescale target and the
        // output-packet rescale source; `video_ist_time_base` is the frame source tb.
        let video_enc_time_base =
            if video_ist_frame_rate.numerator() > 0 && video_ist_frame_rate.denominator() > 0 {
                ffmpeg_the_third::Rational(
                    video_ist_frame_rate.denominator(),
                    video_ist_frame_rate.numerator(),
                )
            } else {
                video_ist_time_base
            };
        video_encoder.set_time_base(video_enc_time_base);

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
        let video_encoder = video_encoder
            .open_as_with(video_enc_codec, enc_opts)
            .map_err(PostProcessError::from)
            .context("failed to open video encoder for transcode")?;

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

        Ok(VideoTranscodeContext {
            ictx,
            video_decoder,
            video_ist_index,
            audio_ist_index,
            video_ist_time_base,
            audio_ist_time_base,
            input_duration_us,
            octx,
            video_encoder,
            video_ost_index,
            video_enc_time_base,
            needs_global_header,
            audio_copy_ost_index: None,
            audio_transcode: None,
            filter_graph: ffmpeg_the_third::filter::Graph::default(),
            audio_sample_counter: 0,
            opts,
            output_path,
        })
    }

    /// Phase 3: configure audio handling — stream-copy, re-encode, or neither —
    /// based on `opts.audio_copy` / `opts.audio_codec`. Mutates `ctx` to set
    /// either `audio_copy_ost_index` or `audio_transcode`.
    pub(super) fn setup_audio_pipeline(ctx: &mut VideoTranscodeContext<'_>) -> anyhow::Result<()> {
        let opts = ctx.opts;
        let audio_ist_index = ctx.audio_ist_index;
        let audio_ist_time_base = ctx.audio_ist_time_base;
        let needs_global_header = ctx.needs_global_header;

        // Determine audio handling mode:
        // - audio_copy=true → stream copy (existing path)
        // - audio_codec=Some → re-encode with specified encoder
        // - neither → no audio output stream
        let audio_encode_codec: Option<&str> = if opts.audio_copy {
            None
        } else {
            opts.audio_codec.as_deref()
        };

        // Add audio output stream (stream copy) if audio exists and copy requested
        if opts.audio_copy
            && let Some(audio_idx) = audio_ist_index
        {
            let audio_ist = ctx.ictx.stream(audio_idx).ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio input stream {audio_idx} not found"))
            })?;
            let audio_ost_idx = Self::add_stream_copy(
                &mut ctx.octx,
                audio_ist.parameters(),
                "for video transcode audio copy",
            )?;
            ctx.octx
                .stream_mut(audio_ost_idx)
                .expect("just-added stream")
                .set_metadata(audio_ist.metadata().to_owned());
            ctx.audio_copy_ost_index = Some(audio_ost_idx);
        }

        // Audio transcode: open decoder + encoder + filter when audio_codec is specified
        if let Some(enc_name) = audio_encode_codec
            && let Some(audio_idx) = audio_ist_index
        {
            // Open audio decoder
            let audio_ist = ctx.ictx.stream(audio_idx).ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("audio input stream {audio_idx} not found"))
            })?;
            let audio_dec_ctx =
                ffmpeg_the_third::codec::context::Context::from_parameters(audio_ist.parameters())?;
            let mut audio_decoder = audio_dec_ctx.decoder().audio()?;
            // Set packet timebase for accurate audio timestamps
            let pkt_tb = audio_ist_time_base.unwrap_or(ffmpeg_the_third::Rational(1, 44100));
            unsafe {
                (*audio_decoder.as_mut_ptr()).pkt_timebase = ffmpeg_the_third::ffi::AVRational {
                    num: pkt_tb.numerator(),
                    den: pkt_tb.denominator(),
                };
            }

            // Find and open audio encoder
            let audio_enc_codec =
                ffmpeg_the_third::encoder::find_by_name(enc_name).ok_or_else(|| {
                    PostProcessError::UnsupportedCodec {
                        codec: enc_name.to_string(),
                        operation: "audio re-encode during video recode".into(),
                    }
                })?;

            // Add audio output stream with encoder
            let audio_enc_ost_idx;
            {
                let ost = ctx
                    .octx
                    .add_stream(audio_enc_codec)
                    .map_err(PostProcessError::from)
                    .context("failed to add audio encode output stream for video transcode")?;
                audio_enc_ost_idx = ost.index();
            }

            // Configure audio encoder from decoder properties
            let audio_enc_context = ffmpeg_the_third::codec::context::Context::from_parameters(
                ctx.octx
                    .stream(audio_enc_ost_idx)
                    .ok_or_else(|| PostProcessError::ffmpeg_failed("audio encode ost not found"))?
                    .parameters(),
            )?;
            let mut audio_encoder = audio_enc_context.encoder().audio()?;

            // Pick sample rate compatible with encoder (prefer decoder rate)
            let audio_input_sample_rate = audio_decoder.rate();
            let target_rate =
                Self::pick_audio_sample_rate(&audio_enc_codec, audio_input_sample_rate);
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
                    &mut ctx.octx,
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

            ctx.audio_transcode = Some(AudioTranscodeState {
                decoder: audio_decoder,
                encoder: audio_encoder,
                enc_time_base,
                ost_index: audio_enc_ost_idx,
                filter: audio_filter,
                input_sample_rate: audio_input_sample_rate as i32,
            });
        }

        Ok(())
    }

    /// Phase 4: set muxer options dictionary, write `encoding_tool` and
    /// per-stream `encoder` metadata, write the output header, and build
    /// the video filter graph for pixel format conversion.
    pub(super) fn write_header_and_build_filter(
        ctx: &mut VideoTranscodeContext<'_>,
    ) -> anyhow::Result<()> {
        let opts = ctx.opts;
        let output = ctx.output_path;
        let video_codec_name = opts.video_codec.as_deref().unwrap_or("libx264");
        let audio_encode_codec: Option<&str> = if opts.audio_copy {
            None
        } else {
            opts.audio_codec.as_deref()
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
            crate::ffmpeg::encoding_tag::set_encoding_tool(&mut ctx.octx, &tool_components);
        }

        // Set per-stream encoder tag on video output stream
        crate::ffmpeg::encoding_tag::set_stream_encoder(
            &mut ctx.octx,
            ctx.video_ost_index,
            video_codec_name,
        );

        // Set per-stream encoder tag on audio output stream (only if re-encoding)
        if let Some(ref audio_transcode) = ctx.audio_transcode
            && let Some(enc_name) = audio_encode_codec
        {
            crate::ffmpeg::encoding_tag::set_stream_encoder(
                &mut ctx.octx,
                audio_transcode.ost_index,
                enc_name,
            );
        }

        // Write header with options
        ctx.octx
            .write_header_with(dict)
            .map_err(PostProcessError::from)
            .context("failed to write output header for video transcode")?;

        // Build video filter graph for pixel format conversion
        ctx.filter_graph = Self::build_video_filter(
            &ctx.video_decoder,
            &ctx.video_encoder,
            ctx.video_ist_time_base,
        )?;

        Ok(())
    }

    /// Phase 5: main packet loop. Reads input packets, dispatches each to
    /// video decode/filter/encode, audio stream-copy, or audio transcode.
    /// Emits the final progress fraction `1.0` before returning.
    pub(super) fn run_encode_loop(
        ctx: &mut VideoTranscodeContext<'_>,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
    ) -> anyhow::Result<()> {
        let VideoTranscodeContext {
            ictx,
            octx,
            video_decoder,
            video_encoder,
            filter_graph,
            video_ist_index,
            video_ist_time_base,
            video_ost_index,
            video_enc_time_base,
            audio_ist_index,
            audio_ist_time_base,
            audio_copy_ost_index,
            audio_transcode,
            audio_sample_counter,
            input_duration_us,
            ..
        } = ctx;

        let mut last_progress = Instant::now();
        let progress_throttle = Duration::from_millis(100);

        // Process packets: video -> decode/filter/encode, audio -> copy or transcode
        for result in ictx.packets() {
            let (stream, mut packet) = result
                .map_err(PostProcessError::from)
                .context("failed to read packet during video transcode")?;
            let ist_index = stream.index();

            if ist_index == *video_ist_index {
                // PTS-based progress from video stream
                if let Some(progress) = progress_fn
                    && *input_duration_us > 0
                    && last_progress.elapsed() >= progress_throttle
                    && let Some(pts) = packet.pts()
                {
                    let tb = *video_ist_time_base;
                    let pts_us =
                        pts * i64::from(tb.numerator()) * 1_000_000 / i64::from(tb.denominator());
                    let frac = (pts_us as f64 / *input_duration_us as f64).clamp(0.0, 1.0);
                    progress(frac);
                    last_progress = Instant::now();
                }
                // Video: decode -> filter -> encode -> write
                video_decoder.send_packet(&packet)?;
                Self::receive_and_process_video(
                    video_decoder,
                    filter_graph,
                    video_encoder,
                    octx,
                    *video_ost_index,
                    *video_enc_time_base,
                    *video_ist_time_base,
                )?;
            } else if Some(ist_index) == *audio_ist_index {
                // Audio: stream copy or transcode
                if let Some(audio_ost_idx) = *audio_copy_ost_index {
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
                        .write_interleaved(octx)
                        .map_err(PostProcessError::from)
                        .context("failed to write audio packet during video transcode")?;
                } else if let Some(at) = audio_transcode.as_mut() {
                    let input_rate = at.input_sample_rate;
                    let input_tb =
                        audio_ist_time_base.unwrap_or(ffmpeg_the_third::Rational(1, input_rate));
                    // Audio transcode path: decode → filter (format convert + frame size) → encode → write
                    at.decoder.send_packet(&packet)?;
                    Self::drain_audio_transcode_filtered(
                        &mut at.decoder,
                        &mut at.filter,
                        &mut at.encoder,
                        octx,
                        at.enc_time_base,
                        at.ost_index,
                        audio_sample_counter,
                        input_rate,
                        input_tb,
                    )?;
                }
            }
        }
        // Emit final 1.0 on completion
        if let Some(progress) = progress_fn {
            progress(1.0);
        }

        Ok(())
    }

    /// Phase 6: flush video decoder, filter graph, and encoder; flush the
    /// audio transcode pipeline if present; flush the interleave queue and
    /// write the output trailer. Consumes `ctx`.
    pub(super) fn finalize_transcode(ctx: VideoTranscodeContext<'_>) -> anyhow::Result<()> {
        let VideoTranscodeContext {
            mut octx,
            mut video_decoder,
            mut video_encoder,
            mut filter_graph,
            mut audio_transcode,
            video_ost_index,
            video_enc_time_base,
            video_ist_time_base,
            audio_ist_time_base,
            mut audio_sample_counter,
            ..
        } = ctx;

        // Flush video decoder
        video_decoder.send_eof()?;
        Self::receive_and_process_video(
            &mut video_decoder,
            &mut filter_graph,
            &mut video_encoder,
            &mut octx,
            video_ost_index,
            video_enc_time_base,
            video_ist_time_base,
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
            video_ist_time_base,
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
        if let Some(at) = audio_transcode.as_mut() {
            let input_rate = at.input_sample_rate;
            let input_tb = audio_ist_time_base.unwrap_or(ffmpeg_the_third::Rational(1, input_rate));
            // Flush decoder
            at.decoder.send_eof()?;
            Self::drain_audio_transcode_filtered(
                &mut at.decoder,
                &mut at.filter,
                &mut at.encoder,
                &mut octx,
                at.enc_time_base,
                at.ost_index,
                &mut audio_sample_counter,
                input_rate,
                input_tb,
            )?;
            // Flush filter graph
            at.filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("audio filter 'in' not found"))?
                .source()
                .flush()?;
            Self::drain_audio_filter_to_encoder(
                &mut at.filter,
                &mut at.encoder,
                &mut octx,
                at.enc_time_base,
                at.ost_index,
            )?;
            // Flush encoder
            at.encoder.send_eof()?;
            Self::drain_audio_encoder_packets(
                &mut at.encoder,
                &mut octx,
                at.enc_time_base,
                at.ost_index,
            )?;
        }

        // Flush interleave queue before trailer
        flush_interleave_queue(&mut octx);

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for video transcode")?;

        Ok(())
    }
}
