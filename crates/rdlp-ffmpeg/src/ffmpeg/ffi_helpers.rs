//! Unsafe FFI helper functions.
//!
//! These encapsulate all `unsafe` FFI operations that lack safe wrappers
//! in `ffmpeg-the-third`, providing safe call-site signatures. The `unsafe`
//! blocks are limited to these well-documented helpers.

use crate::error::Result;

use super::FFmpegRunner;

impl FFmpegRunner {
    /// Reset the codec tag to 0 for container compatibility.
    ///
    /// When remuxing between containers, the source codec tag may not be valid
    /// in the target container. Setting it to 0 lets FFmpeg auto-select.
    pub(crate) fn clear_codec_tag(params_ptr: *const ffmpeg_the_third::ffi::AVCodecParameters) {
        // SAFETY: `params_ptr` points to a valid AVCodecParameters allocated by FFmpeg.
        // Setting codec_tag to 0 is always valid — it tells FFmpeg to auto-select.
        unsafe {
            (*(params_ptr as *mut ffmpeg_the_third::ffi::AVCodecParameters)).codec_tag = 0;
        }
    }

    /// Copy encoder parameters back to an output stream.
    ///
    /// After opening an encoder, its parameters (codec, dimensions, sample rate,
    /// etc.) must be copied to the corresponding output stream before writing
    /// the header.
    pub(crate) fn copy_encoder_params_to_stream(
        octx: &mut ffmpeg_the_third::format::context::Output,
        stream_index: usize,
        encoder_ptr: *const ffmpeg_the_third::ffi::AVCodecContext,
    ) {
        // SAFETY: `octx` owns the output context with a valid stream array.
        // `stream_index` was obtained from a stream added to this context.
        // `encoder_ptr` points to a valid, opened encoder context.
        unsafe {
            let stream_ptr = *(*octx.as_mut_ptr()).streams.add(stream_index);
            ffmpeg_the_third::ffi::avcodec_parameters_from_context(
                (*stream_ptr).codecpar,
                encoder_ptr,
            );
        }
    }

    /// Set the default channel layout for the given number of channels.
    pub(crate) fn set_default_channel_layout(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        channels: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // `av_channel_layout_default` populates the ch_layout field in-place.
        unsafe {
            ffmpeg_the_third::ffi::av_channel_layout_default(
                &mut (*encoder_ptr).ch_layout,
                channels,
            );
        }
    }

    /// Enable VBR (variable bitrate) quality mode on an encoder.
    pub(crate) fn set_vbr_quality(
        encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext,
        quality: i32,
    ) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // Setting QSCALE flag + global_quality is the standard way to enable VBR.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_QSCALE as i32;
            (*encoder_ptr).global_quality = quality * ffmpeg_the_third::ffi::FF_QP2LAMBDA;
        }
    }

    /// Set the global header flag on an encoder.
    ///
    /// Required when the output format needs codec parameters in the container
    /// header rather than in each packet (e.g., MP4, MKV).
    pub(crate) fn set_global_header_flag(encoder_ptr: *mut ffmpeg_the_third::ffi::AVCodecContext) {
        // SAFETY: `encoder_ptr` is a valid, pre-open encoder context.
        // This flag is required by certain container formats.
        unsafe {
            (*encoder_ptr).flags |= ffmpeg_the_third::ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }

    /// Write thumbnail packets from the thumbnail input to the output context.
    pub(crate) fn write_thumbnail_packets(
        thumb_ictx: &mut ffmpeg_the_third::format::context::Input,
        octx: &mut ffmpeg_the_third::format::context::Output,
        thumb_ist_index: usize,
        thumb_ist_time_base: ffmpeg_the_third::Rational,
        thumb_ost_index: usize,
    ) -> Result<()> {
        use crate::error::PostProcessError;

        let thumb_ost_time_base = octx
            .stream(thumb_ost_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "thumbnail output stream {thumb_ost_index} not found"
                ))
            })?
            .time_base();
        for result in thumb_ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read thumbnail packet: {e}"),
                })?;
            if stream.index() == thumb_ist_index {
                packet.rescale_ts(thumb_ist_time_base, thumb_ost_time_base);
                packet.set_position(-1);
                packet.set_stream(thumb_ost_index);
                packet.write_interleaved(octx).map_err(|e| {
                    PostProcessError::FFmpegLibraryError {
                        message: format!("failed to write thumbnail packet: {e}"),
                    }
                })?;
            }
        }
        Ok(())
    }

    /// Add an `abuffer` audio source filter to a graph using raw FFI.
    ///
    /// Uses `avfilter_graph_alloc_filter` + `av_opt_set*` + `avfilter_init_str`
    /// instead of the args-string approach via `Graph::add()`. Required because
    /// FFmpeg 8.0's abuffer option is `"channel_layout"` (not `"chlayout"`),
    /// and the args-string parser rejects unknown option names.
    pub(crate) fn add_abuffer_to_graph(
        graph: &mut ffmpeg_the_third::filter::Graph,
        name: &str,
        time_base: ffmpeg_the_third::Rational,
        sample_rate: u32,
        sample_fmt_name: &str,
        channel_layout_desc: &str,
    ) -> Result<()> {
        use std::ffi::CString;

        use crate::error::PostProcessError;

        let abuffer = ffmpeg_the_third::filter::find("abuffer")
            .ok_or_else(|| PostProcessError::ffmpeg_failed("abuffer filter not found"))?;

        let name_c = CString::new(name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid filter name"))?;
        let ch_layout_c = CString::new(channel_layout_desc)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid channel layout"))?;
        let sample_fmt_c = CString::new(sample_fmt_name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid sample format name"))?;

        // Option key CStrings (static values, can't fail)
        let key_channel_layout = CString::new("channel_layout").unwrap();
        let key_sample_fmt = CString::new("sample_fmt").unwrap();
        let key_time_base = CString::new("time_base").unwrap();
        let key_sample_rate = CString::new("sample_rate").unwrap();

        // SAFETY: All pointers are valid for the duration of this block.
        // avfilter_graph_alloc_filter allocates within the graph's lifetime.
        // av_opt_set* write to the allocated filter context.
        // avfilter_init_str finalizes the filter initialization.
        unsafe {
            let ctx = ffmpeg_the_third::ffi::avfilter_graph_alloc_filter(
                graph.as_mut_ptr(),
                abuffer.as_ptr(),
                name_c.as_ptr(),
            );
            if ctx.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "failed to allocate abuffer filter context".into(),
                });
            }

            let search = ffmpeg_the_third::ffi::AV_OPT_SEARCH_CHILDREN;

            let ret = ffmpeg_the_third::ffi::av_opt_set(
                ctx as *mut std::ffi::c_void,
                key_channel_layout.as_ptr(),
                ch_layout_c.as_ptr(),
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "av_opt_set channel_layout={channel_layout_desc} failed: {ret}"
                    ),
                });
            }

            let ret = ffmpeg_the_third::ffi::av_opt_set(
                ctx as *mut std::ffi::c_void,
                key_sample_fmt.as_ptr(),
                sample_fmt_c.as_ptr(),
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("av_opt_set sample_fmt={sample_fmt_name} failed: {ret}"),
                });
            }

            let ret = ffmpeg_the_third::ffi::av_opt_set_q(
                ctx as *mut std::ffi::c_void,
                key_time_base.as_ptr(),
                ffmpeg_the_third::ffi::AVRational {
                    num: time_base.numerator(),
                    den: time_base.denominator(),
                },
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "av_opt_set_q time_base={}/{} failed: {ret}",
                        time_base.numerator(),
                        time_base.denominator()
                    ),
                });
            }

            let ret = ffmpeg_the_third::ffi::av_opt_set_int(
                ctx as *mut std::ffi::c_void,
                key_sample_rate.as_ptr(),
                i64::from(sample_rate),
                search,
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("av_opt_set_int sample_rate={sample_rate} failed: {ret}"),
                });
            }

            let ret = ffmpeg_the_third::ffi::avfilter_init_str(ctx, std::ptr::null());
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avfilter_init_str for abuffer '{name}' failed: {ret}"),
                });
            }
        }

        Ok(())
    }

    /// Parse a filter spec between named source/sink and validate the graph.
    ///
    /// Bypasses the `ffmpeg-the-third` wrapper's `Parser::parse()` which may
    /// swap the `inputs`/`outputs` parameters to `avfilter_graph_parse_ptr`.
    /// Instead calls FFI directly matching FFmpeg's official `filter_audio.c`
    /// example: `outputs` = source (abuffer), `inputs` = sink (abuffersink).
    pub(crate) fn parse_and_validate_filter_graph(
        graph: &mut ffmpeg_the_third::filter::Graph,
        src_name: &str,
        sink_name: &str,
        filter_spec: &str,
    ) -> Result<()> {
        use std::ffi::CString;

        use crate::error::PostProcessError;

        let src_name_c = CString::new(src_name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid source filter name"))?;
        let sink_name_c = CString::new(sink_name)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid sink filter name"))?;
        let spec_c = CString::new(filter_spec)
            .map_err(|_| PostProcessError::ffmpeg_failed("invalid filter spec"))?;

        // SAFETY: All pointers are valid for the duration of this block.
        // avfilter_graph_get_filter retrieves contexts by name from the graph.
        // avfilter_inout_alloc + av_strdup allocate memory freed by avfilter_inout_free.
        // avfilter_graph_parse_ptr parses the spec and links intermediate filters.
        // avfilter_graph_config validates format negotiation and link configuration.
        unsafe {
            let src_ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                src_name_c.as_ptr(),
            );
            if src_ctx.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("filter '{src_name}' not found in graph"),
                });
            }

            let sink_ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                sink_name_c.as_ptr(),
            );
            if sink_ctx.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("filter '{sink_name}' not found in graph"),
                });
            }

            // `outputs` = source (abuffer) with unconnected output pad.
            // The parsed chain's implicit [in] label connects FROM this pad.
            let outputs = ffmpeg_the_third::ffi::avfilter_inout_alloc();
            if outputs.is_null() {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "failed to allocate AVFilterInOut for outputs".into(),
                });
            }
            (*outputs).name = ffmpeg_the_third::ffi::av_strdup(src_name_c.as_ptr());
            (*outputs).filter_ctx = src_ctx;
            (*outputs).pad_idx = 0;
            (*outputs).next = std::ptr::null_mut();

            // `inputs` = sink (abuffersink) with unconnected input pad.
            // The parsed chain's implicit [out] label connects TO this pad.
            let inputs = ffmpeg_the_third::ffi::avfilter_inout_alloc();
            if inputs.is_null() {
                let mut out_ptr = outputs;
                ffmpeg_the_third::ffi::avfilter_inout_free(&mut out_ptr);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "failed to allocate AVFilterInOut for inputs".into(),
                });
            }
            (*inputs).name = ffmpeg_the_third::ffi::av_strdup(sink_name_c.as_ptr());
            (*inputs).filter_ctx = sink_ctx;
            (*inputs).pad_idx = 0;
            (*inputs).next = std::ptr::null_mut();

            // Parse spec with FFmpeg-standard parameter order:
            //   3rd = &inputs  (sink pads, abuffersink)
            //   4th = &outputs (source pads, abuffer)
            let mut inputs_ptr = inputs;
            let mut outputs_ptr = outputs;
            let ret = ffmpeg_the_third::ffi::avfilter_graph_parse_ptr(
                graph.as_mut_ptr(),
                spec_c.as_ptr(),
                &mut inputs_ptr,
                &mut outputs_ptr,
                std::ptr::null_mut(),
            );

            // Free InOut structures (parse_ptr may set consumed pointers to NULL)
            ffmpeg_the_third::ffi::avfilter_inout_free(&mut inputs_ptr);
            ffmpeg_the_third::ffi::avfilter_inout_free(&mut outputs_ptr);

            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "avfilter_graph_parse_ptr failed for spec '{filter_spec}': {ret}"
                    ),
                });
            }

            // Validate and configure the complete graph
            let ret = ffmpeg_the_third::ffi::avfilter_graph_config(
                graph.as_mut_ptr(),
                std::ptr::null_mut(),
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avfilter_graph_config failed: {ret}"),
                });
            }
        }

        Ok(())
    }

    /// Set `AVDISCARD_ALL` on all non-audio streams in an input context.
    ///
    /// Tells the demuxer to skip non-audio packets entirely, avoiding
    /// memory allocation for large video packets during audio-only analysis.
    pub(crate) fn discard_non_audio_streams(
        ictx: &mut ffmpeg_the_third::format::context::Input,
        audio_stream_index: usize,
    ) {
        // SAFETY: `ictx` owns a valid AVFormatContext. Setting `discard` on
        // streams is a standard FFmpeg operation that tells the demuxer to
        // skip packets for those streams.
        unsafe {
            let ctx_ptr = ictx.as_mut_ptr();
            let nb_streams = (*ctx_ptr).nb_streams as usize;
            for i in 0..nb_streams {
                if i != audio_stream_index {
                    let stream = *(*ctx_ptr).streams.add(i);
                    (*stream).discard = ffmpeg_the_third::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }
    }

    /// Set the frame size on an audio buffersink filter.
    ///
    /// Tells the buffersink to output exactly `frame_size` samples per frame.
    /// The last frame at EOF is automatically zero-padded. This is the proper
    /// way to feed fixed-frame-size encoders (AAC=1024, MP3=1152, Opus=960).
    ///
    /// No-op if `frame_size` is 0 (variable-frame-size codecs like FLAC/PCM).
    pub(crate) fn set_buffersink_frame_size(
        graph: &mut ffmpeg_the_third::filter::Graph,
        sink_name: &str,
        frame_size: u32,
    ) {
        if frame_size == 0 {
            return;
        }

        let Ok(name_c) = std::ffi::CString::new(sink_name) else {
            return;
        };

        // SAFETY: `graph` owns a valid filter graph. `avfilter_graph_get_filter`
        // retrieves the named context. `av_buffersink_set_frame_size` sets the
        // min/max sample counts on the sink's input link.
        unsafe {
            let ctx = ffmpeg_the_third::ffi::avfilter_graph_get_filter(
                graph.as_mut_ptr(),
                name_c.as_ptr(),
            );
            if !ctx.is_null() {
                ffmpeg_the_third::ffi::av_buffersink_set_frame_size(ctx, frame_size);
            }
        }
    }

    /// Configure a stream with `ATTACHED_PIC` disposition (for cover art).
    ///
    /// Sets the stream disposition and clears the codec tag. Used for MP4,
    /// FLAC, OGG, and other containers that embed cover art as a video stream
    /// with special disposition.
    pub(crate) fn set_attached_pic_disposition(stream_ptr: *mut ffmpeg_the_third::ffi::AVStream) {
        // SAFETY: `stream_ptr` is a valid output stream pointer from a live
        // output context. Setting disposition and clearing codec_tag configures
        // the stream as cover art.
        unsafe {
            (*stream_ptr).disposition = ffmpeg_the_third::ffi::AV_DISPOSITION_ATTACHED_PIC;
            (*((*stream_ptr).codecpar)).codec_tag = 0;
        }
    }
}

/// Release packet buffer references. Idempotent — safe on empty packets.
///
/// SAFETY: `Packet::as_ptr()` returns a valid, non-null AVPacket pointer
/// owned by the Rust wrapper. `av_packet_unref` only zeroes internal fields.
pub(crate) fn packet_unref(pkt: &mut ffmpeg_the_third::Packet) {
    use ffmpeg_the_third::packet::Mut;
    unsafe { ffmpeg_the_third::ffi::av_packet_unref(pkt.as_mut_ptr()) }
}

/// Release audio frame buffer references. Idempotent — safe on empty frames.
///
/// Calling this after `filter.source().add(&frame)` and after
/// `encoder.send_frame(&frame)` releases our reference immediately,
/// reducing peak memory when the filter/encoder also holds a ref.
pub(crate) fn frame_unref_audio(frame: &mut ffmpeg_the_third::frame::Audio) {
    unsafe { ffmpeg_the_third::ffi::av_frame_unref((*frame).as_mut_ptr()) }
}

/// Release video frame buffer references. Idempotent — safe on empty frames.
pub(crate) fn frame_unref_video(frame: &mut ffmpeg_the_third::frame::Video) {
    unsafe { ffmpeg_the_third::ffi::av_frame_unref((*frame).as_mut_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_unref_on_empty_packet() {
        let mut pkt = ffmpeg_the_third::Packet::empty();
        // Should not panic on an empty/zeroed packet
        packet_unref(&mut pkt);
        // Double-unref should also be safe (idempotent)
        packet_unref(&mut pkt);
    }

    #[test]
    fn frame_unref_audio_on_empty_frame() {
        let mut frame = ffmpeg_the_third::frame::Audio::empty();
        frame_unref_audio(&mut frame);
        frame_unref_audio(&mut frame);
    }

    #[test]
    fn frame_unref_video_on_empty_frame() {
        let mut frame = ffmpeg_the_third::frame::Video::empty();
        frame_unref_video(&mut frame);
        frame_unref_video(&mut frame);
    }
}
