//! Video + audio stream merging (stream copy).

use std::path::Path;

use log::debug;

use crate::error::{PostProcessError, Result};

use super::{FFmpegRunner, RemuxOptions, ensure_init};

impl FFmpegRunner {
    /// Merge separate video and audio files into a single container (stream copy).
    ///
    /// Takes two input files (one containing video, one containing audio) and
    /// combines them into a single output file without re-encoding.
    /// The MP4 muxer automatically handles AAC ADTS→ASC conversion when needed.
    pub async fn merge(
        &self,
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
    ) -> Result<()> {
        let video_input = video_input.as_ref().to_path_buf();
        let audio_input = audio_input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("merge", move || {
            Self::merge_sync(&video_input, &audio_input, &output, &opts)
        })
        .await
    }

    /// Merge separate video and audio files synchronously (stream copy).
    fn merge_sync(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
        opts: &RemuxOptions,
    ) -> Result<()> {
        ensure_init()?;

        // MKV: use raw FFI with proper stream property copying for VLC compatibility.
        // The key is copying avg_frame_rate which sets Matroska's "Default duration" element.
        let is_mkv = output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mkv"));
        if is_mkv {
            return Self::merge_mkv_raw_ffi(video_input, audio_input, output);
        }

        let mut ictx_video = ffmpeg_the_third::format::input(video_input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open video input {}: {e}", video_input.display()),
            }
        })?;

        let mut ictx_audio = ffmpeg_the_third::format::input(audio_input).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open audio input {}: {e}", audio_input.display()),
            }
        })?;

        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        // Find best video stream from video input
        let video_ist_index = ictx_video
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoVideoStream)?;

        let video_ist_time_base = ictx_video
            .stream(video_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "video input stream {video_ist_index} not found"
                ))
            })?
            .time_base();

        let mut ost_video = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add video output stream: {e}"),
            })?;
        ost_video.set_parameters(
            ictx_video
                .stream(video_ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "video input stream {video_ist_index} not found"
                    ))
                })?
                .parameters(),
        );
        Self::clear_codec_tag(ost_video.parameters().as_ptr());
        let video_ost_index = ost_video.index();

        // Find best audio stream from audio input
        let audio_ist_index = ictx_audio
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ist_time_base = ictx_audio
            .stream(audio_ist_index)
            .ok_or_else(|| {
                PostProcessError::ffmpeg_failed(format!(
                    "audio input stream {audio_ist_index} not found"
                ))
            })?
            .time_base();

        let mut ost_audio = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add audio output stream: {e}"),
            })?;
        ost_audio.set_parameters(
            ictx_audio
                .stream(audio_ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {audio_ist_index} not found"
                    ))
                })?
                .parameters(),
        );
        Self::clear_codec_tag(ost_audio.parameters().as_ptr());
        let audio_ost_index = ost_audio.index();

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MP4/MOV: enable faststart (moov atom at beginning) for streaming
        if opts.faststart {
            dict.set("movflags", "+faststart");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy video packets
        for result in ictx_video.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read video packet: {e}"),
                })?;
            if stream.index() != video_ist_index {
                continue;
            }
            let ost_time_base = octx
                .stream(video_ost_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "video output stream {video_ost_index} not found"
                    ))
                })?
                .time_base();
            packet.rescale_ts(video_ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(video_ost_index);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write video packet: {e}"),
                }
            })?;
        }

        // Copy audio packets
        for result in ictx_audio.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read audio packet: {e}"),
                })?;
            if stream.index() != audio_ist_index {
                continue;
            }
            let ost_time_base = octx
                .stream(audio_ost_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio output stream {audio_ost_index} not found"
                    ))
                })?
                .time_base();
            packet.rescale_ts(audio_ist_time_base, ost_time_base);
            packet.set_position(-1);
            packet.set_stream(audio_ost_index);
            packet.write_interleaved(&mut octx).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("failed to write audio packet: {e}"),
                }
            })?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Merge video + audio into MKV using raw FFI with full stream property copying.
    ///
    /// This copies stream properties that are essential for proper Matroska playback:
    /// - `avg_frame_rate` — critical for "Default duration" element (VLC needs this)
    /// - `r_frame_rate` — real base frame rate
    /// - `time_base` — preserves source timing
    /// - `sample_aspect_ratio` — pixel aspect ratio
    /// - `cluster_time_limit=500` — 500ms clusters for smooth seeking
    /// - `avoid_negative_ts` — timestamp normalization
    /// - `max_interleave_delta=0` — immediate output
    #[allow(clippy::too_many_lines)]
    fn merge_mkv_raw_ffi(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
    ) -> Result<()> {
        use ffmpeg_the_third::ffi;
        use std::ffi::CString;
        use std::ptr;

        debug!("MKV merge via raw FFI with avg_frame_rate + cluster_time_limit=500");

        let video_cstr =
            CString::new(video_input.to_string_lossy().as_ref()).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("Invalid video input path: {e}"),
                }
            })?;
        let audio_cstr =
            CString::new(audio_input.to_string_lossy().as_ref()).map_err(|e| {
                PostProcessError::FFmpegLibraryError {
                    message: format!("Invalid audio input path: {e}"),
                }
            })?;
        let output_cstr = CString::new(output.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid output path: {e}"),
            }
        })?;

        unsafe {
            // 1. Open video input
            let mut ifmt_video: *mut ffi::AVFormatContext = ptr::null_mut();
            let ret = ffi::avformat_open_input(
                &mut ifmt_video,
                video_cstr.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to open video input: error code {ret}"),
                });
            }

            let ret = ffi::avformat_find_stream_info(ifmt_video, ptr::null_mut());
            if ret < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "Failed to find video stream info: error code {ret}"
                    ),
                });
            }

            // 2. Open audio input
            let mut ifmt_audio: *mut ffi::AVFormatContext = ptr::null_mut();
            let ret = ffi::avformat_open_input(
                &mut ifmt_audio,
                audio_cstr.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
            if ret < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to open audio input: error code {ret}"),
                });
            }

            let ret = ffi::avformat_find_stream_info(ifmt_audio, ptr::null_mut());
            if ret < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "Failed to find audio stream info: error code {ret}"
                    ),
                });
            }

            // 3. Find best video stream index
            let video_stream_idx = ffi::av_find_best_stream(
                ifmt_video,
                ffi::AVMediaType::AVMEDIA_TYPE_VIDEO,
                -1,
                -1,
                ptr::null_mut(),
                0,
            );
            if video_stream_idx < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                return Err(PostProcessError::NoVideoStream);
            }
            let video_stream_idx = video_stream_idx as usize;

            // 4. Find best audio stream index
            let audio_stream_idx = ffi::av_find_best_stream(
                ifmt_audio,
                ffi::AVMediaType::AVMEDIA_TYPE_AUDIO,
                -1,
                -1,
                ptr::null_mut(),
                0,
            );
            if audio_stream_idx < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                return Err(PostProcessError::NoAudioStream);
            }
            let audio_stream_idx = audio_stream_idx as usize;

            // 5. Create output context - explicitly request Matroska muxer
            let mut ofmt_ctx: *mut ffi::AVFormatContext = ptr::null_mut();
            let matroska_name = CString::new("matroska").unwrap();
            let ret = ffi::avformat_alloc_output_context2(
                &mut ofmt_ctx,
                ptr::null(),
                matroska_name.as_ptr(),
                output_cstr.as_ptr(),
            );
            if ret < 0 || ofmt_ctx.is_null() {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "Failed to create output context: error code {ret}"
                    ),
                });
            }

            // 6. Add video output stream with full property copying
            let in_video_stream =
                *(*ifmt_video).streams.add(video_stream_idx);

            let out_video_stream =
                ffi::avformat_new_stream(ofmt_ctx, ptr::null());
            if out_video_stream.is_null() {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "Failed to create video output stream".into(),
                });
            }

            let ret = ffi::avcodec_parameters_copy(
                (*out_video_stream).codecpar,
                (*in_video_stream).codecpar,
            );
            if ret < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "Failed to copy video codec params: error code {ret}"
                    ),
                });
            }
            (*(*out_video_stream).codecpar).codec_tag = 0;

            // CRITICAL: Copy stream timing properties for Matroska
            (*out_video_stream).time_base = (*in_video_stream).time_base;
            (*out_video_stream).avg_frame_rate =
                (*in_video_stream).avg_frame_rate;
            (*out_video_stream).r_frame_rate =
                (*in_video_stream).r_frame_rate;
            (*out_video_stream).sample_aspect_ratio =
                (*in_video_stream).sample_aspect_ratio;

            let video_out_idx = (*out_video_stream).index;

            debug!(
                "Video stream: time_base={}/{}, avg_frame_rate={}/{}, r_frame_rate={}/{}",
                (*out_video_stream).time_base.num,
                (*out_video_stream).time_base.den,
                (*out_video_stream).avg_frame_rate.num,
                (*out_video_stream).avg_frame_rate.den,
                (*out_video_stream).r_frame_rate.num,
                (*out_video_stream).r_frame_rate.den,
            );

            // 7. Add audio output stream with full property copying
            let in_audio_stream =
                *(*ifmt_audio).streams.add(audio_stream_idx);

            let out_audio_stream =
                ffi::avformat_new_stream(ofmt_ctx, ptr::null());
            if out_audio_stream.is_null() {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "Failed to create audio output stream".into(),
                });
            }

            let ret = ffi::avcodec_parameters_copy(
                (*out_audio_stream).codecpar,
                (*in_audio_stream).codecpar,
            );
            if ret < 0 {
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "Failed to copy audio codec params: error code {ret}"
                    ),
                });
            }
            (*(*out_audio_stream).codecpar).codec_tag = 0;

            // Copy audio stream timing properties
            (*out_audio_stream).time_base = (*in_audio_stream).time_base;
            (*out_audio_stream).avg_frame_rate =
                (*in_audio_stream).avg_frame_rate;
            (*out_audio_stream).r_frame_rate =
                (*in_audio_stream).r_frame_rate;
            (*out_audio_stream).sample_aspect_ratio =
                (*in_audio_stream).sample_aspect_ratio;

            let audio_out_idx = (*out_audio_stream).index;

            debug!(
                "Audio stream: time_base={}/{}, avg_frame_rate={}/{}",
                (*out_audio_stream).time_base.num,
                (*out_audio_stream).time_base.den,
                (*out_audio_stream).avg_frame_rate.num,
                (*out_audio_stream).avg_frame_rate.den,
            );

            // 8. Set format context options
            (*ofmt_ctx).avoid_negative_ts =
                ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
            (*ofmt_ctx).max_interleave_delta = 0;
            (*ofmt_ctx).flags |= ffi::AVFMT_FLAG_AUTO_BSF;

            // 9. Open output file (AVIO)
            if ((*(*ofmt_ctx).oformat).flags & ffi::AVFMT_NOFILE) == 0 {
                let ret = ffi::avio_open(
                    &mut (*ofmt_ctx).pb,
                    output_cstr.as_ptr(),
                    ffi::AVIO_FLAG_WRITE,
                );
                if ret < 0 {
                    ffi::avformat_close_input(&mut ifmt_video);
                    ffi::avformat_close_input(&mut ifmt_audio);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!(
                            "Failed to open output file: error code {ret}"
                        ),
                    });
                }
            }

            // 10. Build options dictionary with cluster_time_limit
            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            let key = CString::new("cluster_time_limit").unwrap();
            let value = CString::new("500").unwrap();
            ffi::av_dict_set(&mut opts, key.as_ptr(), value.as_ptr(), 0);

            // 11. Initialize muxer with options
            let ret = ffi::avformat_init_output(ofmt_ctx, &mut opts);

            // Check for unconsumed options
            let mut e: *mut ffi::AVDictionaryEntry = ptr::null_mut();
            loop {
                e = ffi::av_dict_get(
                    opts,
                    c"".as_ptr(),
                    e,
                    ffi::AV_DICT_IGNORE_SUFFIX,
                );
                if e.is_null() {
                    break;
                }
                let k =
                    std::ffi::CStr::from_ptr((*e).key).to_string_lossy();
                let v =
                    std::ffi::CStr::from_ptr((*e).value).to_string_lossy();
                log::warn!("Unconsumed FFI option: {k}={v}");
            }
            ffi::av_dict_free(&mut opts);

            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "avformat_init_output failed: error code {ret}"
                    ),
                });
            }

            // 12. Write header
            let ret =
                ffi::avformat_write_header(ofmt_ctx, ptr::null_mut());
            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "avformat_write_header failed: error code {ret}"
                    ),
                });
            }

            // 13. Copy video packets
            let mut pkt = ffi::AVPacket {
                buf: ptr::null_mut(),
                pts: ffi::AV_NOPTS_VALUE,
                dts: ffi::AV_NOPTS_VALUE,
                data: ptr::null_mut(),
                size: 0,
                stream_index: 0,
                flags: 0,
                side_data: ptr::null_mut(),
                side_data_elems: 0,
                duration: 0,
                pos: -1,
                opaque: ptr::null_mut(),
                opaque_ref: ptr::null_mut(),
                time_base: ffi::AVRational { num: 0, den: 1 },
            };

            loop {
                let ret = ffi::av_read_frame(ifmt_video, &mut pkt);
                if ret < 0 {
                    break; // EOF or error
                }

                if pkt.stream_index as usize != video_stream_idx {
                    ffi::av_packet_unref(&mut pkt);
                    continue;
                }

                pkt.stream_index = video_out_idx;

                // Rescale timestamps (guard against AV_NOPTS_VALUE)
                let out_stream =
                    *(*ofmt_ctx).streams.add(video_out_idx as usize);
                if pkt.pts != ffi::AV_NOPTS_VALUE {
                    pkt.pts = ffi::av_rescale_q_rnd(
                        pkt.pts,
                        (*in_video_stream).time_base,
                        (*out_stream).time_base,
                        ffi::AVRounding::AV_ROUND_NEAR_INF,
                    );
                }
                if pkt.dts != ffi::AV_NOPTS_VALUE {
                    pkt.dts = ffi::av_rescale_q_rnd(
                        pkt.dts,
                        (*in_video_stream).time_base,
                        (*out_stream).time_base,
                        ffi::AVRounding::AV_ROUND_NEAR_INF,
                    );
                }
                if pkt.duration > 0 {
                    pkt.duration = ffi::av_rescale_q(
                        pkt.duration,
                        (*in_video_stream).time_base,
                        (*out_stream).time_base,
                    );
                }
                pkt.pos = -1;

                let ret =
                    ffi::av_interleaved_write_frame(ofmt_ctx, &mut pkt);
                ffi::av_packet_unref(&mut pkt);

                if ret < 0 {
                    log::error!("Error writing video packet: {ret}");
                    break;
                }
            }

            // 14. Copy audio packets
            loop {
                let ret = ffi::av_read_frame(ifmt_audio, &mut pkt);
                if ret < 0 {
                    break; // EOF or error
                }

                if pkt.stream_index as usize != audio_stream_idx {
                    ffi::av_packet_unref(&mut pkt);
                    continue;
                }

                pkt.stream_index = audio_out_idx;

                // Rescale timestamps (guard against AV_NOPTS_VALUE)
                let out_stream =
                    *(*ofmt_ctx).streams.add(audio_out_idx as usize);
                if pkt.pts != ffi::AV_NOPTS_VALUE {
                    pkt.pts = ffi::av_rescale_q_rnd(
                        pkt.pts,
                        (*in_audio_stream).time_base,
                        (*out_stream).time_base,
                        ffi::AVRounding::AV_ROUND_NEAR_INF,
                    );
                }
                if pkt.dts != ffi::AV_NOPTS_VALUE {
                    pkt.dts = ffi::av_rescale_q_rnd(
                        pkt.dts,
                        (*in_audio_stream).time_base,
                        (*out_stream).time_base,
                        ffi::AVRounding::AV_ROUND_NEAR_INF,
                    );
                }
                if pkt.duration > 0 {
                    pkt.duration = ffi::av_rescale_q(
                        pkt.duration,
                        (*in_audio_stream).time_base,
                        (*out_stream).time_base,
                    );
                }
                pkt.pos = -1;

                let ret =
                    ffi::av_interleaved_write_frame(ofmt_ctx, &mut pkt);
                ffi::av_packet_unref(&mut pkt);

                if ret < 0 {
                    log::error!("Error writing audio packet: {ret}");
                    break;
                }
            }

            // 15. Write trailer and cleanup
            ffi::av_write_trailer(ofmt_ctx);

            if !(*ofmt_ctx).pb.is_null() {
                ffi::avio_closep(&mut (*ofmt_ctx).pb);
            }
            ffi::avformat_close_input(&mut ifmt_video);
            ffi::avformat_close_input(&mut ifmt_audio);
            ffi::avformat_free_context(ofmt_ctx);
        }

        Ok(())
    }
}
