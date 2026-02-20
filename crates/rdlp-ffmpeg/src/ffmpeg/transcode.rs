//! Audio extraction, video conversion, and transcoding.
//!
//! Provides audio extraction (stream copy + transcode), video conversion
//! (remux + transcode), and internal filter graph / encode helpers.

use std::path::Path;

use log::{debug, info, warn};

use ffmpeg_the_third::packet::Ref as _;

use crate::error::{PostProcessError, Result};

use super::ffi_helpers::{frame_unref_audio, frame_unref_video, packet_unref};
use super::salvage::prepare_input_with_salvage;
use super::{AudioExtractOptions, FFmpegRunner, RemuxOptions, VideoConvertOptions, ensure_init};

impl FFmpegRunner {
    /// Extract audio from a media file, either by stream copy or transcoding.
    ///
    /// Uses `opts.copy` to determine whether to copy or transcode.
    /// For transcoding, supports bitrate (CBR) and quality scale (VBR) modes.
    ///
    /// Automatically detects and salvages corrupt Matroska/WebM containers
    /// before extraction to prevent EBML-induced muxer failures.
    pub async fn extract_audio(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &AudioExtractOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("extract_audio", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, true)?;

            let result = Self::extract_audio_sync(&effective_input, &output, &opts);

            if let Some(ref temp) = salvage_temp {
                let _ = std::fs::remove_file(temp);
            }

            result
        })
        .await
    }

    /// Extract audio synchronously (dispatches to copy or transcode).
    fn extract_audio_sync(input: &Path, output: &Path, opts: &AudioExtractOptions) -> Result<()> {
        if opts.copy {
            Self::extract_audio_copy_sync(input, output)
        } else {
            Self::extract_audio_transcode_sync(input, output, opts)
        }
    }

    /// Extract audio by stream copy (no re-encoding).
    ///
    /// Maps only the best audio stream from input to output without transcoding.
    fn extract_audio_copy_sync(input: &Path, output: &Path) -> Result<()> {
        ensure_init()?;

        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
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
        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add output stream: {e}"),
            })?;
        ost.set_parameters(
            ictx.stream(ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {ist_index} not found"
                    ))
                })?
                .parameters(),
        );
        Self::clear_codec_tag(ost.parameters().as_ptr());

        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy only audio packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            if stream.index() != ist_index {
                continue;
            }
            let ost_time_base = octx
                .stream(0)
                .ok_or_else(|| PostProcessError::ffmpeg_failed("output stream 0 not found"))?
                .time_base();
            packet.rescale_ts(ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(0);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Extract audio by transcoding to a target codec.
    ///
    /// Decodes the input audio, optionally converts sample format/rate through
    /// a filter graph, and encodes to the target codec.
    fn extract_audio_transcode_sync(
        input: &Path,
        output: &Path,
        opts: &AudioExtractOptions,
    ) -> Result<()> {
        ensure_init()?;

        // Open input and find audio stream
        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

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
        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
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
            let ost =
                octx.add_stream(enc_codec)
                    .map_err(|e| PostProcessError::FFmpegLibraryError {
                        message: format!("failed to add output stream: {e}"),
                    })?;
            ost_index = ost.index();
            enc_context =
                ffmpeg_the_third::codec::context::Context::from_parameters(ost.parameters())?;
        }
        // ost dropped — octx no longer mutably borrowed

        // Configure encoder
        let mut audio_encoder = enc_context.encoder().audio()?;

        let target_format = Self::pick_audio_sample_format(&enc_codec, decoder.format());
        audio_encoder.set_format(target_format);
        audio_encoder.set_rate(decoder.rate() as i32);
        let enc_time_base = ffmpeg_the_third::Rational(1, decoder.rate() as i32);
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
        let mut audio_encoder =
            audio_encoder
                .open_as(enc_codec)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to open audio encoder: {e}"),
                })?;

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

        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

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

        // Transcode loop: read → decode → filter → encode → write
        for result in ictx.packets() {
            let (stream, packet) = result.map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to read packet: {e}"),
            })?;
            if stream.index() != ist_index {
                continue;
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
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

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
    /// Uses `abuffer` → `anull` → `abuffersink` to let FFmpeg handle any
    /// necessary sample format, sample rate, or channel layout conversions.
    fn build_audio_filter(
        decoder: &ffmpeg_the_third::decoder::Audio,
        encoder: &ffmpeg_the_third::encoder::audio::Audio,
        ist_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let mut graph = ffmpeg_the_third::filter::Graph::new();

        let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

        Self::add_abuffer_to_graph(
            &mut graph,
            "in",
            ist_time_base,
            decoder.rate(),
            decoder.format().name(),
            &decoder.ch_layout().description(),
        )?;
        graph
            .add(&abuffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffersink filter: {e}"),
            })?;

        // Build aformat spec to convert to encoder's expected format
        let enc_ch_layout_desc = encoder.ch_layout().description();
        let aformat_spec = format!(
            "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
            encoder.format().name(),
            encoder.rate(),
            enc_ch_layout_desc,
        );

        Self::parse_and_validate_filter_graph(&mut graph, "in", "out", &aformat_spec)?;

        Ok(graph)
    }

    /// Receive decoded frames from decoder, push through filter, encode, and write.
    ///
    /// Uses interleaved writes — appropriate for multi-stream output.
    pub(crate) fn receive_and_process_audio(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("av_buffersrc_add_frame failed: {e}"),
                })?;
            frame_unref_audio(&mut frame);
            Self::drain_filter_to_encoder(filter, encoder, octx, ost_index, enc_time_base, timing)?;
        }
        Ok(())
    }

    /// Pull filtered frames from filter graph, encode, and write.
    ///
    /// Uses interleaved writes — appropriate for multi-stream output.
    pub(crate) fn drain_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        loop {
            let mut out_node = filter
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            encoder
                .send_frame(&filtered)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("avcodec_send_frame failed: {e}"),
                })?;
            frame_unref_audio(&mut filtered);
            Self::drain_encoder_packets(encoder, octx, ost_index, enc_time_base, timing)?;
        }
        Ok(())
    }

    /// Receive encoded packets from encoder and write to output (interleaved).
    ///
    /// Rescales packet timestamps from encoder timebase to output stream
    /// timebase, enforces monotonic DTS, fixes zero-duration packets, then
    /// writes via direct FFI `av_interleaved_write_frame` to capture the raw
    /// return code and I/O diagnostics on failure. Appropriate for
    /// multi-stream output.
    pub(crate) fn drain_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
    ) -> Result<()> {
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("output stream {ost_index} not found"))
            })?
            .time_base();
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.rescale_ts(enc_time_base, ost_time_base);
            packet.set_position(-1);

            if timing.pkt_count == 0 {
                debug!(
                    "First encoded packet: pts={:?}, dts={:?}, size={}, dur={}, enc_tb={}/{}, ost_tb={}/{}, exp_dur={}",
                    packet.pts(),
                    packet.dts(),
                    packet.size(),
                    packet.duration(),
                    enc_time_base.numerator(),
                    enc_time_base.denominator(),
                    ost_time_base.numerator(),
                    ost_time_base.denominator(),
                    timing.expected_duration,
                );
            }

            // Unified timestamp + duration fix
            let (new_dts, new_pts, new_dur, updated) =
                fix_audio_timestamps(packet.dts(), packet.pts(), packet.duration(), timing);
            if new_dts != packet.dts() || new_dur != packet.duration() {
                debug!(
                    "Timestamp fix: pkt#{} dts={:?}->{new_dts:?}, pts={:?}->{new_pts:?}, dur={}->{new_dur}",
                    timing.pkt_count,
                    packet.dts(),
                    packet.pts(),
                    packet.duration(),
                );
            }
            packet.set_dts(new_dts);
            packet.set_pts(new_pts);
            packet.set_duration(new_dur);
            timing.last_dts = updated;
            timing.last_duration = new_dur;

            // Capture packet metadata before write (av_interleaved_write_frame unrefs)
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;
            let dur = packet.duration();

            // Direct FFI call to capture raw return code
            // SAFETY: octx and packet are valid; av_interleaved_write_frame takes
            // ownership of the packet buffer and unrefs it on success.
            let ret = unsafe {
                ffmpeg_the_third::ffi::av_interleaved_write_frame(
                    octx.as_mut_ptr(),
                    packet.as_ptr() as *mut _,
                )
            };
            // Unref immediately: frees encoded data before next receive_packet.
            // av_interleaved_write_frame unrefs on success in FFmpeg 8.0, but NOT
            // on failure. Explicit unref is idempotent and matches
            // merge.rs / remux.rs / thumbnail.rs patterns.
            packet_unref(&mut packet);
            if ret < 0 {
                if ret == -12 {
                    warn!(
                        "FFmpeg allocation failure (ENOMEM) during mux write — \
                         likely packet/frame leak or extreme memory pressure (rss={}KB)",
                        get_process_rss_kb(),
                    );
                }
                let strerr = av_strerror_string(ret);
                // SAFETY: octx owns a valid format context
                let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                return Err(PostProcessError::MuxWriteError {
                    message: format!("ret={ret} ({strerr}), dur={dur}, {io_diag}"),
                    operation: "av_interleaved_write_frame".into(),
                    stream_index: ost_index,
                    pts,
                    dts,
                    packet_size: size,
                    time_base_num: ost_time_base.numerator(),
                    time_base_den: ost_time_base.denominator(),
                });
            }
            timing.pkt_count += 1;

            // Mux progress watchdog (interleaved path): flush AVIO then check
            // pos and file size, same dual-signal approach as the direct path.
            if timing.pkt_count % 256 == 0 {
                unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() {
                        ffmpeg_the_third::ffi::avio_flush(pb);
                    }
                }

                let current_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };
                if current_pos > timing.last_pos_check {
                    timing.last_pos_check = current_pos;
                    timing.stall_count = 0;
                } else {
                    timing.stall_count += 1;
                    if timing.stall_count >= 3 {
                        let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                        return Err(PostProcessError::MuxWriteError {
                            message: format!(
                                "mux stall detected: pb->pos={current_pos} unchanged for {} packets, {io_diag}",
                                timing.stall_count * 256
                            ),
                            operation: "av_interleaved_write_frame (watchdog)".into(),
                            stream_index: ost_index,
                            pts,
                            dts,
                            packet_size: size,
                            time_base_num: ost_time_base.numerator(),
                            time_base_den: ost_time_base.denominator(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Receive decoded frames from decoder, push through filter, encode, and write.
    ///
    /// Uses direct (non-interleaved) writes — appropriate for single-stream
    /// audio-only output where the muxer's interleaving buffer is unnecessary.
    /// Primary path for audio-only normalization to avoid interleave queue ENOMEM.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn receive_and_process_audio_direct(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
        output_path: Option<&Path>,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("av_buffersrc_add_frame failed: {e}"),
                })?;
            frame_unref_audio(&mut frame);
            Self::drain_filter_to_encoder_direct(
                filter,
                encoder,
                octx,
                ost_index,
                enc_time_base,
                timing,
                output_path,
            )?;
        }
        Ok(())
    }

    /// Pull filtered frames from filter graph, encode, and write.
    ///
    /// Uses direct (non-interleaved) writes — appropriate for single-stream
    /// audio-only output. Primary path for normalization to avoid ENOMEM.
    pub(crate) fn drain_filter_to_encoder_direct(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
        output_path: Option<&Path>,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        loop {
            let mut out_node = filter
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            encoder
                .send_frame(&filtered)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("avcodec_send_frame failed: {e}"),
                })?;
            frame_unref_audio(&mut filtered);
            Self::drain_encoder_packets_direct(
                encoder,
                octx,
                ost_index,
                enc_time_base,
                timing,
                output_path,
            )?;
        }
        Ok(())
    }

    /// Receive encoded packets from encoder and write to output (direct).
    ///
    /// Rescales packet timestamps from encoder timebase to output stream
    /// timebase, enforces monotonic DTS, fixes zero-duration packets, then
    /// writes via direct FFI `av_write_frame` (non-interleaved) to bypass
    /// the muxer's interleave queue. This is appropriate for single-stream
    /// audio-only output where interleaving is unnecessary — the queue would
    /// only buffer packets without flushing, risking ENOMEM on long files.
    /// The rescaling is critical for Matroska muxer which changes the stream
    /// timebase after `write_header()` (e.g. from `1/48000` to `1/1000`).
    ///
    /// `output_path` enables the watchdog to check actual file size on disk
    /// as a secondary progress signal alongside `pb->pos`.
    ///
    /// Primary path for audio-only normalization to avoid interleave queue ENOMEM.
    pub(crate) fn drain_encoder_packets_direct(
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
        output_path: Option<&Path>,
    ) -> Result<()> {
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("output stream {ost_index} not found"))
            })?
            .time_base();
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.set_position(-1);

            if timing.use_sample_clock {
                // Sample-clock: synthesize timestamps from cumulative sample count
                let dur_samples = i64::from(timing.encoder_frame_size).max(1);
                let sr_tb = ffmpeg_the_third::ffi::AVRational {
                    num: 1,
                    den: timing.sample_rate as i32,
                };
                let ost_tb = ffmpeg_the_third::ffi::AVRational {
                    num: ost_time_base.numerator(),
                    den: ost_time_base.denominator(),
                };
                let dts = unsafe {
                    ffmpeg_the_third::ffi::av_rescale_q(timing.samples_written, sr_tb, ost_tb)
                };
                let dur_tb =
                    unsafe { ffmpeg_the_third::ffi::av_rescale_q(dur_samples, sr_tb, ost_tb) }
                        .max(1);

                packet.set_dts(Some(dts));
                packet.set_pts(Some(dts));
                packet.set_duration(dur_tb);
                timing.samples_written += dur_samples;

                // Safety net: run through fix_audio_timestamps (should be no-op)
                let (new_dts, new_pts, new_dur, updated) =
                    fix_audio_timestamps(packet.dts(), packet.pts(), packet.duration(), timing);
                packet.set_dts(new_dts);
                packet.set_pts(new_pts);
                packet.set_duration(new_dur);
                timing.last_dts = updated;
                timing.last_duration = new_dur;
            } else {
                // Legacy path: use encoder timestamps + rescale
                packet.rescale_ts(enc_time_base, ost_time_base);

                if timing.pkt_count == 0 {
                    debug!(
                        "First encoded packet (direct): pts={:?}, dts={:?}, size={}, dur={}, enc_tb={}/{}, ost_tb={}/{}, exp_dur={}",
                        packet.pts(),
                        packet.dts(),
                        packet.size(),
                        packet.duration(),
                        enc_time_base.numerator(),
                        enc_time_base.denominator(),
                        ost_time_base.numerator(),
                        ost_time_base.denominator(),
                        timing.expected_duration,
                    );
                }

                let (new_dts, new_pts, new_dur, updated) =
                    fix_audio_timestamps(packet.dts(), packet.pts(), packet.duration(), timing);
                if new_dts != packet.dts() || new_dur != packet.duration() {
                    debug!(
                        "Timestamp fix: pkt#{} dts={:?}->{new_dts:?}, pts={:?}->{new_pts:?}, dur={}->{new_dur}",
                        timing.pkt_count,
                        packet.dts(),
                        packet.pts(),
                        packet.duration(),
                    );
                }
                packet.set_dts(new_dts);
                packet.set_pts(new_pts);
                packet.set_duration(new_dur);
                timing.last_dts = updated;
                timing.last_duration = new_dur;
            }

            // Diagnostic logging for first 5 packets (sample-clock path)
            if timing.pkt_count < 5 {
                let pb_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };
                let file_sz = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                info!(
                    "[audio_only_mux] pkt#{}: dts={:?}, pts={:?}, dur={}, \
                     samples_written={}, sr={}, ost_tb={}/{}, pb_pos={}, file={}B",
                    timing.pkt_count,
                    packet.dts(),
                    packet.pts(),
                    packet.duration(),
                    timing.samples_written,
                    timing.sample_rate,
                    ost_time_base.numerator(),
                    ost_time_base.denominator(),
                    pb_pos,
                    file_sz,
                );
            }

            // Capture packet metadata before write (av_write_frame may unref)
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;
            let dur = packet.duration();

            // Direct FFI call using av_write_frame (non-interleaved) to bypass
            // the muxer's interleave queue. For single-stream audio output,
            // interleaving is unnecessary and the queue can grow unbounded.
            // SAFETY: octx and packet are valid; av_write_frame writes the
            // packet directly without buffering in an interleave queue.
            let ret = unsafe {
                ffmpeg_the_third::ffi::av_write_frame(octx.as_mut_ptr(), packet.as_ptr() as *mut _)
            };
            // Unref immediately: frees encoded data before next receive_packet.
            // av_write_frame unrefs on success in FFmpeg 8.0, but NOT on failure.
            // Explicit unref is idempotent and matches merge.rs / remux.rs /
            // thumbnail.rs patterns.
            packet_unref(&mut packet);
            if ret < 0 {
                if ret == -12 {
                    warn!(
                        "FFmpeg allocation failure (ENOMEM) during mux write — \
                         likely packet/frame leak or extreme memory pressure (rss={}KB)",
                        get_process_rss_kb(),
                    );
                }
                let strerr = av_strerror_string(ret);
                // SAFETY: octx owns a valid format context
                let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                return Err(PostProcessError::MuxWriteError {
                    message: format!("ret={ret} ({strerr}), dur={dur}, {io_diag}"),
                    operation: "av_write_frame".into(),
                    stream_index: ost_index,
                    pts,
                    dts,
                    packet_size: size,
                    time_base_num: ost_time_base.numerator(),
                    time_base_den: ost_time_base.denominator(),
                });
            }
            timing.pkt_count += 1;

            // Mux-pressure instrumentation: log progress every 10,000 packets
            if timing.pkt_count % 10_000 == 0 {
                let pb_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };
                let file_sz = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                let rss_kb = get_process_rss_kb();
                info!(
                    "[mux progress] pkt={}, dts={:?}, dur={}, samples={}, pos={}, file={}KB, rss={}KB",
                    timing.pkt_count,
                    dts,
                    dur,
                    timing.samples_written,
                    pb_pos,
                    file_sz / 1024,
                    rss_kb,
                );
            }

            // Mux progress watchdog: every 256 packets, flush AVIO so pb->pos
            // reflects actual writes, then check both pos and file size.
            // If neither advances for 3 consecutive checks (768 packets), the
            // muxer is stuck. Abort with a retryable error so salvage/CLI
            // fallback can recover.
            if timing.pkt_count % 256 == 0 {
                // Flush AVIO buffer so pb->pos reflects actual write progress
                unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() {
                        ffmpeg_the_third::ffi::avio_flush(pb);
                    }
                }

                let current_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };

                let file_size = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len() as i64)
                    .unwrap_or(-1);

                let progressing = current_pos > timing.last_pos_check
                    || (file_size > 0 && file_size > timing.last_file_size);

                if progressing {
                    timing.last_pos_check = current_pos;
                    if file_size > 0 {
                        timing.last_file_size = file_size;
                    }
                    timing.stall_count = 0;
                } else {
                    timing.stall_count += 1;
                    if timing.stall_count >= 3 {
                        // Full IO dump before returning error
                        if let Some(path) = output_path {
                            unsafe {
                                super::normalize::dump_io_state(
                                    octx.as_mut_ptr(),
                                    path,
                                    "watchdog_stall",
                                );
                            }
                        }
                        let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                        return Err(PostProcessError::MuxWriteError {
                            message: format!(
                                "mux stall detected: pb->pos={current_pos}, file_size={file_size} unchanged for {} packets, {io_diag}",
                                timing.stall_count * 256
                            ),
                            operation: "av_write_frame (watchdog)".into(),
                            stream_index: ost_index,
                            pts,
                            dts,
                            packet_size: size,
                            time_base_num: ost_time_base.numerator(),
                            time_base_den: ost_time_base.denominator(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Receive decoded frames from decoder, push through filter, encode, and write.
    ///
    /// Uses interleaved writes with `AVFMT_FLAG_FLUSH_PACKETS`.
    /// Kept as fallback; direct-write path is now primary for normalization.
    #[allow(dead_code, clippy::too_many_arguments)]
    pub(crate) fn receive_and_process_audio_interleaved(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
        output_path: Option<&Path>,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("av_buffersrc_add_frame failed: {e}"),
                })?;
            frame_unref_audio(&mut frame);
            Self::drain_filter_to_encoder_interleaved(
                filter,
                encoder,
                octx,
                ost_index,
                enc_time_base,
                timing,
                output_path,
            )?;
        }
        Ok(())
    }

    /// Pull filtered frames from filter graph, encode, and write (interleaved).
    /// Kept as fallback; direct-write path is now primary for normalization.
    #[allow(dead_code)]
    pub(crate) fn drain_filter_to_encoder_interleaved(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
        output_path: Option<&Path>,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        loop {
            let mut out_node = filter
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            encoder
                .send_frame(&filtered)
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("avcodec_send_frame failed: {e}"),
                })?;
            frame_unref_audio(&mut filtered);
            Self::drain_encoder_packets_interleaved(
                encoder,
                octx,
                ost_index,
                enc_time_base,
                timing,
                output_path,
            )?;
        }
        Ok(())
    }

    /// Receive encoded packets from encoder and write to output (interleaved).
    ///
    /// Mirrors `drain_encoder_packets_direct` but uses `av_interleaved_write_frame`.
    /// Kept as fallback; direct-write path with sample-clock is now primary
    /// for normalization to avoid interleave queue ENOMEM.
    #[allow(dead_code)]
    pub(crate) fn drain_encoder_packets_interleaved(
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
        output_path: Option<&Path>,
    ) -> Result<()> {
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("output stream {ost_index} not found"))
            })?
            .time_base();
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.rescale_ts(enc_time_base, ost_time_base);
            packet.set_position(-1);

            if timing.pkt_count == 0 {
                debug!(
                    "First encoded packet (interleaved): pts={:?}, dts={:?}, size={}, dur={}, enc_tb={}/{}, ost_tb={}/{}, exp_dur={}",
                    packet.pts(),
                    packet.dts(),
                    packet.size(),
                    packet.duration(),
                    enc_time_base.numerator(),
                    enc_time_base.denominator(),
                    ost_time_base.numerator(),
                    ost_time_base.denominator(),
                    timing.expected_duration,
                );
            }

            // Unified timestamp + duration fix
            let (new_dts, new_pts, new_dur, updated) =
                fix_audio_timestamps(packet.dts(), packet.pts(), packet.duration(), timing);
            if new_dts != packet.dts() || new_dur != packet.duration() {
                debug!(
                    "Timestamp fix: pkt#{} dts={:?}->{new_dts:?}, pts={:?}->{new_pts:?}, dur={}->{new_dur}",
                    timing.pkt_count,
                    packet.dts(),
                    packet.pts(),
                    packet.duration(),
                );
            }
            packet.set_dts(new_dts);
            packet.set_pts(new_pts);
            packet.set_duration(new_dur);
            timing.last_dts = updated;
            timing.last_duration = new_dur;

            // Capture packet metadata before write
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;
            let dur = packet.duration();

            let ret = unsafe {
                ffmpeg_the_third::ffi::av_interleaved_write_frame(
                    octx.as_mut_ptr(),
                    packet.as_ptr() as *mut _,
                )
            };
            // Unref immediately: frees encoded data before next receive_packet.
            // av_interleaved_write_frame unrefs on success in FFmpeg 8.0, but NOT
            // on failure. Explicit unref is idempotent and matches
            // merge.rs / remux.rs / thumbnail.rs patterns.
            packet_unref(&mut packet);
            if ret < 0 {
                if ret == -12 {
                    warn!(
                        "FFmpeg allocation failure (ENOMEM) during mux write — \
                         likely packet/frame leak or extreme memory pressure (rss={}KB)",
                        get_process_rss_kb(),
                    );
                }
                let strerr = av_strerror_string(ret);
                let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                return Err(PostProcessError::MuxWriteError {
                    message: format!("ret={ret} ({strerr}), dur={dur}, {io_diag}"),
                    operation: "av_interleaved_write_frame (normalize)".into(),
                    stream_index: ost_index,
                    pts,
                    dts,
                    packet_size: size,
                    time_base_num: ost_time_base.numerator(),
                    time_base_den: ost_time_base.denominator(),
                });
            }
            timing.pkt_count += 1;

            // Mux-pressure instrumentation: log progress every 10,000 packets
            if timing.pkt_count % 10_000 == 0 {
                let current_pos_for_log = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };
                let file_size = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                let rss_kb = get_process_rss_kb();
                info!(
                    "[mux progress] pkt={}, dts={:?}, dur={}, pos={}, file={}KB, rss={}KB, exp_dur={}",
                    timing.pkt_count,
                    dts,
                    dur,
                    current_pos_for_log,
                    file_size / 1024,
                    rss_kb,
                    timing.expected_duration,
                );
            }

            // Mux progress watchdog: flush AVIO then check pos + file size
            if timing.pkt_count % 256 == 0 {
                unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() {
                        ffmpeg_the_third::ffi::avio_flush(pb);
                    }
                }

                let current_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };

                let file_size = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len() as i64)
                    .unwrap_or(-1);

                let progressing = current_pos > timing.last_pos_check
                    || (file_size > 0 && file_size > timing.last_file_size);

                if progressing {
                    timing.last_pos_check = current_pos;
                    if file_size > 0 {
                        timing.last_file_size = file_size;
                    }
                    timing.stall_count = 0;
                } else {
                    timing.stall_count += 1;
                    if timing.stall_count >= 3 {
                        if let Some(path) = output_path {
                            unsafe {
                                super::normalize::dump_io_state(
                                    octx.as_mut_ptr(),
                                    path,
                                    "watchdog_stall_interleaved",
                                );
                            }
                        }
                        let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                        return Err(PostProcessError::MuxWriteError {
                            message: format!(
                                "mux stall detected: pb->pos={current_pos}, file_size={file_size} unchanged for {} packets, {io_diag}",
                                timing.stall_count * 256
                            ),
                            operation: "av_interleaved_write_frame (watchdog)".into(),
                            stream_index: ost_index,
                            pts,
                            dts,
                            packet_size: size,
                            time_base_num: ost_time_base.numerator(),
                            time_base_den: ost_time_base.denominator(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

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
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("convert_video", move || {
            let (effective_input, salvage_temp) = prepare_input_with_salvage(&input, true)?;

            let result = Self::convert_video_sync(&effective_input, &output, &opts);

            if let Some(ref temp) = salvage_temp {
                let _ = std::fs::remove_file(temp);
            }

            result
        })
        .await
    }

    /// Convert video synchronously (dispatches to remux or transcode).
    fn convert_video_sync(input: &Path, output: &Path, opts: &VideoConvertOptions) -> Result<()> {
        if opts.remux_only {
            // Determine if output is MP4/MOV for faststart
            let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("");
            let remux_opts = RemuxOptions {
                faststart: ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("mov"),
                ..Default::default()
            };
            Self::remux_sync(input, output, &remux_opts)
        } else {
            Self::convert_video_transcode_sync(input, output, opts)
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
    ) -> Result<()> {
        ensure_init()?;

        // Open input
        let mut ictx = ffmpeg_the_third::format::input(input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open input {}: {e}", input.display()),
            }
        })?;

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

        // Process packets: video → decode/filter/encode, audio → copy
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            let ist_index = stream.index();

            if ist_index == video_ist_index {
                // Video: decode → filter → encode → write
                video_decoder.send_packet(&packet)?;
                Self::receive_and_process_video(
                    &mut video_decoder,
                    &mut filter_graph,
                    &mut video_encoder,
                    &mut octx,
                    video_ost_index,
                )?;
            } else if Some(ist_index) == audio_ist_index {
                // Audio: stream copy
                if let Some(audio_ost_idx) = audio_ost_index {
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
                }
            }
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

        // Flush interleave queue before trailer
        flush_interleave_queue(&mut octx);

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Pick a pixel format supported by the video encoder, preferring the decoder's format.
    fn pick_video_pixel_format(
        codec: &ffmpeg_the_third::Codec,
        preferred: ffmpeg_the_third::format::Pixel,
    ) -> ffmpeg_the_third::format::Pixel {
        unsafe {
            let ptr = codec.as_ptr();
            let pix_fmts = (*ptr).pix_fmts;
            if pix_fmts.is_null() {
                return preferred;
            }

            let mut i = 0;
            let mut first = None;
            loop {
                let fmt = *pix_fmts.offset(i);
                if fmt == ffmpeg_the_third::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                    break;
                }
                let pixel = ffmpeg_the_third::format::Pixel::from(fmt);
                first.get_or_insert(pixel);
                if pixel == preferred {
                    return preferred;
                }
                i += 1;
            }

            first.unwrap_or(preferred)
        }
    }

    /// Build a video filter graph for pixel format conversion.
    ///
    /// Uses `buffer` → `format` → `buffersink` to convert pixel format
    /// from decoder output to encoder input format.
    fn build_video_filter(
        decoder: &ffmpeg_the_third::decoder::Video,
        encoder: &ffmpeg_the_third::encoder::video::Video,
        ist_time_base: ffmpeg_the_third::Rational,
    ) -> Result<ffmpeg_the_third::filter::Graph> {
        let mut graph = ffmpeg_the_third::filter::Graph::new();

        let buffer = ffmpeg_the_third::filter::find("buffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("buffer filter not found"))?;
        let buffersink = ffmpeg_the_third::filter::find("buffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("buffersink filter not found"))?;

        // Pixel aspect ratio (default 1:1 if unknown)
        let sar = decoder.aspect_ratio();
        let sar_num = sar.numerator().max(1);
        let sar_den = sar.denominator().max(1);

        let args = format!(
            "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
            decoder.width(),
            decoder.height(),
            decoder.format() as i32,
            ist_time_base.numerator(),
            ist_time_base.denominator(),
            sar_num,
            sar_den,
        );

        graph
            .add(&buffer, "in", &args)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffer filter: {e}"),
            })?;
        graph
            .add(&buffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add buffersink filter: {e}"),
            })?;

        // Convert pixel format to match encoder's requirement
        let enc_pix_fmt_name = encoder
            .format()
            .descriptor()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|| "yuv420p".to_string());

        let format_spec = format!("format=pix_fmts={enc_pix_fmt_name}");

        Self::parse_and_validate_filter_graph(&mut graph, "in", "out", &format_spec)?;

        Ok(graph)
    }

    /// Receive decoded video frames, push through filter, encode, and write.
    fn receive_and_process_video(
        decoder: &mut ffmpeg_the_third::decoder::Video,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("video filter node 'in' not found"))?
                .source()
                .add(&frame)?;
            frame_unref_video(&mut frame);
            Self::drain_video_filter_to_encoder(filter, encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Pull filtered video frames from filter graph, encode, and write.
    fn drain_video_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Video::empty();
        loop {
            let mut out_node = filter.get("out").ok_or_else(|| {
                PostProcessError::ffmpeg_failed("video filter node 'out' not found")
            })?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            encoder.send_frame(&filtered)?;
            frame_unref_video(&mut filtered);
            Self::drain_video_encoder_packets(encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Receive encoded video packets from encoder and write to output.
    fn drain_video_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::video::Video,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let ost_time_base = octx
            .stream(ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!("output stream {ost_index} not found"))
            })?
            .time_base();
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);

            // Capture packet metadata before write
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;

            // Direct FFI call to capture raw return code
            // SAFETY: octx and packet are valid; av_interleaved_write_frame takes
            // ownership of the packet buffer and unrefs it on success.
            let ret = unsafe {
                ffmpeg_the_third::ffi::av_interleaved_write_frame(
                    octx.as_mut_ptr(),
                    packet.as_ptr() as *mut _,
                )
            };
            // Unref immediately: frees encoded data before next receive_packet.
            // av_interleaved_write_frame unrefs on success in FFmpeg 8.0, but NOT
            // on failure. Explicit unref is idempotent and matches
            // merge.rs / remux.rs / thumbnail.rs patterns.
            packet_unref(&mut packet);
            if ret < 0 {
                if ret == -12 {
                    warn!(
                        "FFmpeg allocation failure (ENOMEM) during video mux write — \
                         likely packet/frame leak or extreme memory pressure (rss={}KB)",
                        get_process_rss_kb(),
                    );
                }
                let strerr = av_strerror_string(ret);
                let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                return Err(PostProcessError::MuxWriteError {
                    message: format!("ret={ret} ({strerr}), {io_diag}"),
                    operation: "av_interleaved_write_frame (video)".into(),
                    stream_index: ost_index,
                    pts,
                    dts,
                    packet_size: size,
                    time_base_num: ost_time_base.numerator(),
                    time_base_den: ost_time_base.denominator(),
                });
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Persistent muxing state for monotonic DTS enforcement across drain calls.
///
/// Created once per encode session and threaded through all pipeline functions
/// so that DTS monotonicity is enforced across call boundaries, not just within
/// a single drain call.
#[derive(Default, Debug)]
pub(crate) struct MuxTimingState {
    /// Last DTS written to the muxer (in output stream timebase).
    pub last_dts: Option<i64>,
    /// Duration of the last packet written (output stream timebase).
    /// Used for duration-aware DTS correction to maintain proper packet spacing.
    pub last_duration: i64,
    /// Precomputed expected packet duration in output stream timebase.
    /// Derived from `encoder_frame_size` rescaled from encoder timebase to
    /// output stream timebase. Used as the primary step size when correcting
    /// DTS regressions, ensuring the Matroska muxer's cluster boundaries
    /// trigger correctly (preventing unbounded cache growth / ENOMEM).
    pub expected_duration: i64,
    /// Total packets written in this encode session.
    pub pkt_count: u64,
    /// Encoder frame size in samples. Used for sample-clock DTS synthesis
    /// (`dur_samples = encoder_frame_size`) and retained for diagnostics.
    pub encoder_frame_size: u32,
    /// `pb->pos` at last watchdog check (mux progress tracking).
    pub last_pos_check: i64,
    /// Consecutive watchdog checks without `pb->pos` advancement.
    /// When this reaches the stall threshold, the encode is aborted
    /// with a retryable `MuxWriteError`.
    pub stall_count: u32,
    /// File size on disk at last watchdog check (secondary progress signal).
    pub last_file_size: i64,
    /// Cumulative samples written. Primary clock for audio-only outputs.
    /// DTS = rescale_q(samples_written, 1/sample_rate, ost_tb).
    pub samples_written: i64,
    /// Audio sample rate (Hz). Set once during init.
    pub sample_rate: u32,
    /// When true, synthesize DTS/PTS from `samples_written`
    /// instead of using encoder-produced timestamps.
    pub use_sample_clock: bool,
}

/// Fix audio packet timestamps for muxer compatibility.
///
/// Ensures:
/// 1. Duration is set (uses `expected_duration` if packet duration is 0/negative)
/// 2. DTS is strictly increasing with proper packet spacing
/// 3. PTS >= DTS (muxer invariant)
///
/// When correcting DTS regressions, steps by `expected_duration` (not 1) to
/// maintain correct packet spacing. This prevents the Matroska muxer from
/// accumulating an unbounded cluster cache due to artificially dense timestamps.
///
/// Returns `(corrected_dts, corrected_pts, duration, updated_last_dts)`.
fn fix_audio_timestamps(
    dts: Option<i64>,
    pts: Option<i64>,
    duration: i64,
    timing: &MuxTimingState,
) -> (Option<i64>, Option<i64>, i64, Option<i64>) {
    // 1. Fix duration first
    let dur = if duration > 0 {
        duration
    } else if timing.expected_duration > 0 {
        timing.expected_duration
    } else {
        timing.last_duration.max(1)
    };

    // 2. Fix DTS
    let Some(d) = dts else {
        return (dts, pts, dur, timing.last_dts);
    };

    if let Some(prev) = timing.last_dts {
        if d <= prev {
            let step = if timing.expected_duration > 0 {
                timing.expected_duration
            } else {
                dur.max(1)
            };
            let corrected = prev + step;
            let p = pts.map(|p| p.max(corrected)).or(Some(corrected));
            return (Some(corrected), p, dur, Some(corrected));
        }
    }

    // 3. Ensure pts >= dts even without correction (pts must be Some when dts is Some)
    let p = pts.map(|p| p.max(d)).or(Some(d));
    (Some(d), p, dur, Some(d))
}

/// Convert an FFmpeg error code to a human-readable string via `av_strerror`.
fn av_strerror_string(errnum: i32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is a stack-allocated array with known size; av_strerror
    // writes at most errbuf_size bytes including the NUL terminator.
    unsafe {
        ffmpeg_the_third::ffi::av_strerror(errnum, buf.as_mut_ptr() as *mut _, buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Flush the muxer's interleave queue before writing the trailer.
///
/// Sends a NULL packet to `av_interleaved_write_frame` which signals
/// the muxer to flush any buffered packets in the interleave queue.
/// Only needed for the interleaved write path; the direct `av_write_frame`
/// path bypasses the queue entirely.
pub(crate) fn flush_interleave_queue(octx: &mut ffmpeg_the_third::format::context::Output) {
    // SAFETY: octx is a valid output format context; passing a null packet
    // signals the muxer to flush its internal interleave queue.
    let ret = unsafe {
        ffmpeg_the_third::ffi::av_interleaved_write_frame(octx.as_mut_ptr(), std::ptr::null_mut())
    };
    if ret < 0 {
        let strerr = av_strerror_string(ret);
        warn!("Interleave queue flush returned {ret} ({strerr})");
    }
}

/// Diagnose the I/O context of an output format context at a write failure.
///
/// # Safety
///
/// `octx_ptr` must point to a valid `AVFormatContext`.
unsafe fn diagnose_mux_io(octx_ptr: *mut ffmpeg_the_third::ffi::AVFormatContext) -> String {
    // SAFETY: caller guarantees octx_ptr is valid; all field reads are from
    // FFmpeg-allocated structs whose layout is defined by the C ABI.
    unsafe {
        let pb = (*octx_ptr).pb;
        if pb.is_null() {
            return "pb=NULL".to_string();
        }
        let error = (*pb).error;
        let write_flag = (*pb).write_flag;
        let has_write_cb = (*pb).write_packet.is_some();
        let pos = (*pb).pos;
        let eof = (*pb).eof_reached;
        format!(
            "pb={{write_flag={write_flag}, error={error}, pos={pos}, eof_reached={eof}, write_cb={}}}",
            if has_write_cb { "present" } else { "NULL" }
        )
    }
}

/// Get current process RSS in KB. Returns 0 if unavailable.
fn get_process_rss_kb() -> u64 {
    #[cfg(target_os = "windows")]
    {
        use std::mem;

        #[repr(C)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }

        unsafe extern "system" {
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                ppsmemcounters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
        }

        unsafe {
            let mut pmc: ProcessMemoryCounters = mem::zeroed();
            pmc.cb = mem::size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
                return (pmc.working_set_size / 1024) as u64;
            }
        }
        0
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(0)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_audio_timestamps_normal() {
        // Increasing sequence passes through unchanged
        let timing = MuxTimingState::default();
        let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), 21, &timing);
        assert_eq!(d, Some(10));
        assert_eq!(p, Some(10));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(10));

        let timing = MuxTimingState {
            last_dts: Some(10),
            last_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(20), Some(20), 21, &timing);
        assert_eq!(d, Some(20));
        assert_eq!(p, Some(20));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(20));
    }

    #[test]
    fn test_fix_audio_timestamps_duplicate() {
        // Same DTS clamped to prev + expected_duration (or dur.max(1) if no expected)
        let timing = MuxTimingState {
            last_dts: Some(10),
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), 21, &timing);
        assert_eq!(d, Some(31)); // 10 + dur=21
        assert_eq!(p, Some(31));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(31));
    }

    #[test]
    fn test_fix_audio_timestamps_regression() {
        // Backwards DTS clamped to prev + expected_duration
        let timing = MuxTimingState {
            last_dts: Some(10),
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(5), Some(5), 21, &timing);
        assert_eq!(d, Some(31)); // 10 + 21
        assert_eq!(p, Some(31));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(31));
    }

    #[test]
    fn test_fix_audio_timestamps_pts_correction() {
        // PTS < corrected DTS gets bumped
        let timing = MuxTimingState {
            last_dts: Some(10),
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(8), 21, &timing);
        assert_eq!(d, Some(31));
        assert_eq!(p, Some(31)); // PTS bumped from 8 to 31
        assert_eq!(dur, 21);
        assert_eq!(last, Some(31));
    }

    #[test]
    fn test_fix_audio_timestamps_none() {
        // None DTS passes through, last_dts unchanged
        let timing = MuxTimingState {
            last_dts: Some(10),
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(None, Some(42), 21, &timing);
        assert_eq!(d, None);
        assert_eq!(p, Some(42));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(10));

        let timing = MuxTimingState::default();
        let (d, p, dur, last) = fix_audio_timestamps(None, None, 21, &timing);
        assert_eq!(d, None);
        assert_eq!(p, None);
        assert_eq!(dur, 21);
        assert_eq!(last, None);
    }

    #[test]
    fn test_fix_audio_timestamps_pts_ge_dts_no_correction() {
        // Even without DTS correction, ensure pts >= dts
        let timing = MuxTimingState {
            last_dts: Some(10),
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(20), Some(15), 21, &timing);
        assert_eq!(d, Some(20));
        assert_eq!(p, Some(20)); // PTS bumped from 15 to 20
        assert_eq!(dur, 21);
        assert_eq!(last, Some(20));
    }

    #[test]
    fn test_fix_audio_timestamps_cross_call_persistence() {
        // Simulates the cross-call scenario: call 1 ends with last_dts=100,
        // call 2 starts with dts=100 → must correct.
        let timing = MuxTimingState {
            last_dts: Some(100),
            expected_duration: 21,
            pkt_count: 50,
            ..Default::default()
        };
        let (d, p, dur, updated) = fix_audio_timestamps(Some(100), Some(100), 21, &timing);
        assert_eq!(d, Some(121)); // 100 + 21
        assert_eq!(p, Some(121));
        assert_eq!(dur, 21);
        assert_eq!(updated, Some(121));
    }

    #[test]
    fn test_fix_audio_timestamps_pts_none_gets_set() {
        // dts=Some(50), pts=None → pts=Some(50)
        let timing = MuxTimingState::default();
        let (d, p, dur, last) = fix_audio_timestamps(Some(50), None, 21, &timing);
        assert_eq!(d, Some(50));
        assert_eq!(p, Some(50));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(50));
    }

    #[test]
    fn test_fix_audio_timestamps_pts_none_with_correction() {
        // dts=Some(5), pts=None, last_dts=Some(10) → corrected
        let timing = MuxTimingState {
            last_dts: Some(10),
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(5), None, 21, &timing);
        assert_eq!(d, Some(31)); // 10 + 21
        assert_eq!(p, Some(31));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(31));
    }

    #[test]
    fn test_fix_audio_timestamps_expected_duration_step() {
        // AAC at 48kHz in 1/1000 tb: expected_duration=21
        let timing = MuxTimingState {
            last_dts: Some(105),
            last_duration: 21,
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(100), Some(100), 21, &timing);
        assert_eq!(d, Some(126)); // 105 + 21
        assert_eq!(p, Some(126));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(126));
    }

    #[test]
    fn test_fix_audio_timestamps_zero_duration_fixed() {
        // Zero duration gets fixed from expected_duration
        let timing = MuxTimingState {
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(0), Some(0), 0, &timing);
        assert_eq!(d, Some(0));
        assert_eq!(p, Some(0));
        assert_eq!(dur, 21); // Fixed from 0 to expected
        assert_eq!(last, Some(0));
    }

    #[test]
    fn test_fix_audio_timestamps_normal_progression() {
        // Normal progression with expected_duration: no correction needed
        let timing = MuxTimingState {
            last_dts: Some(21),
            last_duration: 21,
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(42), Some(42), 21, &timing);
        assert_eq!(d, Some(42)); // No correction needed
        assert_eq!(p, Some(42));
        assert_eq!(dur, 21);
        assert_eq!(last, Some(42));
    }

    #[test]
    fn test_fix_audio_timestamps_zero_duration_no_expected() {
        // Zero duration, no expected_duration → fallback to last_duration or 1
        let timing = MuxTimingState {
            last_dts: Some(10),
            last_duration: 0,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), 0, &timing);
        // duration fixed to max(last_duration=0, 1) = 1
        assert_eq!(dur, 1);
        // DTS corrected: 10 + dur.max(1)=1 = 11 (no expected_duration)
        assert_eq!(d, Some(11));
        assert_eq!(p, Some(11));
        assert_eq!(last, Some(11));
    }

    #[test]
    fn test_fix_audio_timestamps_negative_duration_fallback() {
        // Negative duration → fixed via expected_duration
        let timing = MuxTimingState {
            expected_duration: 21,
            ..Default::default()
        };
        let (d, p, dur, last) = fix_audio_timestamps(Some(10), Some(10), -5, &timing);
        assert_eq!(dur, 21); // Fixed from -5 to expected=21
        assert_eq!(d, Some(10));
        assert_eq!(p, Some(10));
        assert_eq!(last, Some(10));
    }

    #[test]
    fn test_mux_timing_state_default() {
        let timing = MuxTimingState::default();
        assert_eq!(timing.last_dts, None);
        assert_eq!(timing.last_duration, 0);
        assert_eq!(timing.expected_duration, 0);
        assert_eq!(timing.pkt_count, 0);
        assert_eq!(timing.encoder_frame_size, 0);
        assert_eq!(timing.last_pos_check, 0);
        assert_eq!(timing.stall_count, 0);
        assert_eq!(timing.last_file_size, 0);
        assert_eq!(timing.samples_written, 0);
        assert_eq!(timing.sample_rate, 0);
        assert!(!timing.use_sample_clock);
    }

    #[test]
    fn test_mux_timing_state_stall_tracking() {
        let mut timing = MuxTimingState {
            last_pos_check: 1000,
            stall_count: 2,
            ..Default::default()
        };
        // pos advanced
        let new_pos: i64 = 2000;
        if new_pos > timing.last_pos_check {
            timing.last_pos_check = new_pos;
            timing.stall_count = 0;
        }
        assert_eq!(timing.last_pos_check, 2000);
        assert_eq!(timing.stall_count, 0);

        // Simulate pos NOT advancing — stall_count increments
        let same_pos: i64 = 2000;
        if same_pos > timing.last_pos_check {
            timing.last_pos_check = same_pos;
            timing.stall_count = 0;
        } else {
            timing.stall_count += 1;
        }
        assert_eq!(timing.stall_count, 1);

        // Two more stalls reach threshold
        timing.stall_count += 1;
        timing.stall_count += 1;
        assert_eq!(timing.stall_count, 3);
        assert!(timing.stall_count >= 3, "stall threshold reached");
    }

    #[test]
    fn test_fix_audio_timestamps_with_sample_clock_noop() {
        // When timestamps are already monotonic from sample-clock,
        // fix_audio_timestamps should pass them through unchanged.
        let mut timing = MuxTimingState {
            expected_duration: 21,
            use_sample_clock: true,
            sample_rate: 48000,
            encoder_frame_size: 1024,
            ..Default::default()
        };

        // Simulate 3 packets with perfectly monotonic timestamps
        for i in 0..3 {
            let dts = i * 21;
            let (d, p, dur, last) = fix_audio_timestamps(Some(dts), Some(dts), 21, &timing);
            assert_eq!(d, Some(dts), "pkt {i}: dts passthrough");
            assert_eq!(p, Some(dts), "pkt {i}: pts passthrough");
            assert_eq!(dur, 21, "pkt {i}: dur passthrough");
            assert_eq!(last, Some(dts), "pkt {i}: last_dts updated");
            timing.last_dts = last;
            timing.last_duration = dur;
        }
    }

    /// Helper: compute expected DTS/duration for sample-clock synthesis
    /// using `av_rescale_q(samples, 1/sample_rate, ost_tb)`.
    fn sample_clock_rescale(
        samples: i64,
        sample_rate: i32,
        ost_tb_num: i32,
        ost_tb_den: i32,
    ) -> i64 {
        // av_rescale_q(a, bq, cq) = a * bq.num * cq.den / (bq.den * cq.num)
        // bq = {1, sample_rate}, cq = {ost_tb_num, ost_tb_den}
        // = samples * 1 * ost_tb_den / (sample_rate * ost_tb_num)
        let num = samples as i128 * ost_tb_den as i128;
        let den = sample_rate as i128 * ost_tb_num as i128;
        // Round to nearest (matching av_rescale_q behavior)
        ((num + den / 2) / den) as i64
    }

    #[test]
    fn test_sample_clock_aac_48000() {
        // AAC: 1024 samples at 48000 Hz, ost_tb=1/1000
        let sr = 48000;
        let frame = 1024i64;

        let dur = sample_clock_rescale(frame, sr, 1, 1000);
        assert_eq!(dur, 21, "AAC 48kHz dur in 1/1000 tb");

        let dts0 = sample_clock_rescale(0, sr, 1, 1000);
        assert_eq!(dts0, 0);
        let dts1 = sample_clock_rescale(frame, sr, 1, 1000);
        assert_eq!(dts1, 21);
        let dts2 = sample_clock_rescale(frame * 2, sr, 1, 1000);
        assert_eq!(dts2, 43); // 2048/48000*1000 = 42.666... → 43
        let dts3 = sample_clock_rescale(frame * 3, sr, 1, 1000);
        assert_eq!(dts3, 64); // 3072/48000*1000 = 64.0

        // Monotonic
        assert!(dts1 > dts0);
        assert!(dts2 > dts1);
        assert!(dts3 > dts2);
    }

    #[test]
    fn test_sample_clock_opus_48000() {
        // Opus: 960 samples at 48000 Hz, ost_tb=1/1000
        let sr = 48000;
        let frame = 960i64;

        let dur = sample_clock_rescale(frame, sr, 1, 1000);
        assert_eq!(dur, 20, "Opus 48kHz dur in 1/1000 tb");

        let dts0 = sample_clock_rescale(0, sr, 1, 1000);
        assert_eq!(dts0, 0);
        let dts1 = sample_clock_rescale(frame, sr, 1, 1000);
        assert_eq!(dts1, 20);
        let dts2 = sample_clock_rescale(frame * 2, sr, 1, 1000);
        assert_eq!(dts2, 40);
        let dts3 = sample_clock_rescale(frame * 3, sr, 1, 1000);
        assert_eq!(dts3, 60);

        assert!(dts1 > dts0);
        assert!(dts2 > dts1);
        assert!(dts3 > dts2);
    }

    #[test]
    fn test_sample_clock_aac_44100() {
        // AAC: 1024 samples at 44100 Hz, ost_tb=1/1000
        let sr = 44100;
        let frame = 1024i64;

        let dur = sample_clock_rescale(frame, sr, 1, 1000);
        assert_eq!(dur, 23, "AAC 44100Hz dur in 1/1000 tb"); // 1024/44100*1000 = 23.21... → 23

        let dts0 = sample_clock_rescale(0, sr, 1, 1000);
        assert_eq!(dts0, 0);
        let dts1 = sample_clock_rescale(frame, sr, 1, 1000);
        assert_eq!(dts1, 23);
        let dts2 = sample_clock_rescale(frame * 2, sr, 1, 1000);
        assert_eq!(dts2, 46);
        let dts3 = sample_clock_rescale(frame * 3, sr, 1, 1000);
        assert_eq!(dts3, 70); // 3072/44100*1000 = 69.65... → 70

        assert!(dts1 > dts0);
        assert!(dts2 > dts1);
        assert!(dts3 > dts2);
    }
}
