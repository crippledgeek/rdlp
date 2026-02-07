//! Audio extraction, video conversion, and transcoding.
//!
//! Provides audio extraction (stream copy + transcode), video conversion
//! (remux + transcode), and internal filter graph / encode helpers.

use std::path::Path;

use log::debug;

use crate::error::{PostProcessError, Result};

use super::{AudioExtractOptions, FFmpegRunner, RemuxOptions, VideoConvertOptions, ensure_init};

impl FFmpegRunner {
    /// Extract audio from a media file, either by stream copy or transcoding.
    ///
    /// Uses `opts.copy` to determine whether to copy or transcode.
    /// For transcoding, supports bitrate (CBR) and quality scale (VBR) modes.
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
            Self::extract_audio_sync(&input, &output, &opts)
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
        audio_encoder.set_time_base(ffmpeg_the_third::Rational(1, decoder.rate() as i32));

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

        // Copy encoder parameters back to output stream
        // SAFETY: audio_encoder is a valid opened encoder context.
        Self::copy_encoder_params_to_stream(&mut octx, ost_index, unsafe {
            audio_encoder.as_ptr()
        });

        octx.write_header()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Build filter graph for sample format/rate conversion
        let mut filter_graph = Self::build_audio_filter(&decoder, &audio_encoder, ist_time_base)?;

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
        )?;

        // Flush filter graph (signal EOF to source)
        filter_graph
            .get("in")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
            .source()
            .flush()?;
        Self::drain_filter_to_encoder(&mut filter_graph, &mut audio_encoder, &mut octx, ost_index)?;

        // Flush encoder
        audio_encoder.send_eof()?;
        Self::drain_encoder_packets(&mut audio_encoder, &mut octx, ost_index)?;

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Pick a sample format supported by the encoder, preferring the decoder's format.
    fn pick_audio_sample_format(
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
                if first.is_none() {
                    first = Some(sample);
                }
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

        let abuffer = ffmpeg_the_third::filter::find("abuffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffer filter not found"))?;
        let abuffersink = ffmpeg_the_third::filter::find("abuffersink")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffersink filter not found"))?;

        // Build abuffer args with decoder's output parameters
        let channels = decoder.ch_layout().channels();
        let args = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:chlayout={}c",
            ist_time_base.numerator(),
            ist_time_base.denominator(),
            decoder.rate(),
            decoder.format().name(),
            channels,
        );

        graph
            .add(&abuffer, "in", &args)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffer filter: {e}"),
            })?;
        graph
            .add(&abuffersink, "out", "")
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add abuffersink filter: {e}"),
            })?;

        // Build aformat spec to convert to encoder's expected format
        let enc_channels = encoder.ch_layout().channels();
        let aformat_spec = format!(
            "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}c",
            encoder.format().name(),
            encoder.rate(),
            enc_channels,
        );

        graph
            .output("out", 0)?
            .input("in", 0)?
            .parse(&aformat_spec)?;
        graph.validate()?;

        Ok(graph)
    }

    /// Receive decoded frames from decoder, push through filter, encode, and write.
    fn receive_and_process_audio(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            filter
                .get("in")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'in' not found"))?
                .source()
                .add(&frame)?;
            Self::drain_filter_to_encoder(filter, encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Pull filtered frames from filter graph, encode, and write.
    fn drain_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut filtered = ffmpeg_the_third::frame::Audio::empty();
        loop {
            let mut out_node = filter
                .get("out")
                .ok_or_else(|| PostProcessError::ffmpeg_failed("filter node 'out' not found"))?;
            if out_node.sink().frame(&mut filtered).is_err() {
                break;
            }
            encoder.send_frame(&filtered)?;
            Self::drain_encoder_packets(encoder, octx, ost_index)?;
        }
        Ok(())
    }

    /// Receive encoded packets from encoder and write to output.
    fn drain_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
    ) -> Result<()> {
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.write_interleaved(octx)?;
        }
        Ok(())
    }

    /// Convert a video file, either by remuxing or transcoding.
    ///
    /// Uses `opts.remux_only` to determine whether to stream-copy or transcode.
    /// For transcoding, encodes video with the specified codec while optionally
    /// copying the audio stream unchanged.
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
            Self::convert_video_sync(&input, &output, &opts)
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
                if first.is_none() {
                    first = Some(pixel);
                }
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
        let sar_num = if sar.numerator() > 0 {
            sar.numerator()
        } else {
            1
        };
        let sar_den = if sar.denominator() > 0 {
            sar.denominator()
        } else {
            1
        };

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

        graph
            .output("out", 0)?
            .input("in", 0)?
            .parse(&format_spec)?;
        graph.validate()?;

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
        let mut packet = ffmpeg_the_third::Packet::empty();
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(ost_index);
            packet.write_interleaved(octx)?;
        }
        Ok(())
    }
}
