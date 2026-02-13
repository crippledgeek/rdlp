//! Container remuxing (stream copy, no re-encoding).

use std::path::Path;

use log::debug;

use crate::error::{PostProcessError, Result};

use super::{FFmpegRunner, RemuxOptions, ensure_init};

impl FFmpegRunner {
    /// Remux a file (stream copy, no re-encoding) with optional faststart.
    ///
    /// This performs a container-level copy without transcoding, useful for:
    /// - Moving the moov atom to the start of MP4 files (faststart)
    /// - Fixing timestamps and container structure
    /// - Converting between container formats
    pub async fn remux(
        &self,
        input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
    ) -> Result<()> {
        let input = input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("remux", move || Self::remux_sync(&input, &output, &opts)).await
    }

    /// Remux a single input file synchronously (stream copy).
    ///
    /// Normalizes PTS/DTS timestamps to start at 0 (fixes HLS streams with non-zero start times).
    /// For MKV output, uses raw FFI with proper stream property copying for VLC compatibility.
    pub(crate) fn remux_sync(input: &Path, output: &Path, opts: &RemuxOptions) -> Result<()> {
        ensure_init()?;

        // MKV: use raw FFI with proper stream property copying for VLC compatibility.
        // The key is copying avg_frame_rate which sets Matroska's "Default duration" element.
        let is_mkv = output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mkv"));
        if is_mkv {
            return Self::remux_mkv_raw_ffi(input, output);
        }

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

        let stream_count = ictx.streams().count();
        let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
        let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
        // Track PTS offset per stream for normalization (in stream time_base units)
        let mut ist_pts_offsets: Vec<i64> = vec![0; stream_count];
        let mut ost_index: i32 = 0;

        // Find minimum start_time across all video/audio streams for normalization
        let min_start_time_seconds: f64 = ictx
            .streams()
            .filter(|ist| {
                matches!(
                    ist.parameters().medium(),
                    ffmpeg_the_third::media::Type::Video | ffmpeg_the_third::media::Type::Audio
                )
            })
            .filter_map(|ist| {
                let start = ist.start_time();
                if start > 0 {
                    let tb = ist.time_base();
                    Some(start as f64 * tb.0 as f64 / tb.1 as f64)
                } else {
                    None
                }
            })
            .reduce(f64::min)
            .unwrap_or(0.0);

        for (ist_index, ist) in ictx.streams().enumerate() {
            let medium = ist.parameters().medium();
            if !matches!(
                medium,
                ffmpeg_the_third::media::Type::Video | ffmpeg_the_third::media::Type::Audio
            ) {
                continue;
            }

            stream_mapping[ist_index] = ost_index;
            ist_time_bases[ist_index] = ist.time_base();

            // Calculate PTS offset for this stream in its time_base units
            // offset = min_start_time_seconds * time_base_denominator / time_base_numerator
            let tb = ist.time_base();
            ist_pts_offsets[ist_index] =
                (min_start_time_seconds * tb.1 as f64 / tb.0 as f64) as i64;

            ost_index += 1;

            let mut ost = octx
                .add_stream(ffmpeg_the_third::encoder::find(
                    ffmpeg_the_third::codec::Id::None,
                ))
                .map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to add output stream: {e}"),
                })?;
            ost.set_parameters(ist.parameters());
            Self::clear_codec_tag(ost.parameters().as_ptr());
        }

        // Copy format-level metadata
        octx.set_metadata(ictx.metadata().to_owned());

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MP4/MOV: enable faststart (moov atom at beginning) for streaming
        if opts.faststart {
            dict.set("movflags", "+faststart");
        }

        // Write header with muxer options
        octx.write_header_with(dict)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // Copy packets with PTS normalization (shifts timestamps to start at 0)
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read packet: {e}"),
                })?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;

            // Apply PTS offset to normalize timestamps to start at 0
            let offset = ist_pts_offsets[ist_index];
            if offset > 0 {
                if let Some(pts) = packet.pts() {
                    packet.set_pts(Some(pts.saturating_sub(offset)));
                }
                if let Some(dts) = packet.dts() {
                    packet.set_dts(Some(dts.saturating_sub(offset)));
                }
            }

            let ost_time_base = octx
                .stream(ost_idx)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!("output stream {ost_idx} not found"))
                })?
                .time_base();
            packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
            packet.set_position(-1);
            packet.set_stream(ost_idx);
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

    /// Remux to MKV using raw FFI with full CLI-equivalent stream setup.
    ///
    /// This copies stream properties that are essential for proper Matroska playback:
    /// - `avg_frame_rate` — critical for "Default duration" element (VLC needs this)
    /// - `r_frame_rate` — real base frame rate
    /// - `time_base` — preserves source timing
    /// - `sample_aspect_ratio` — pixel aspect ratio
    /// - `cluster_time_limit=500` — 500ms clusters for smooth seeking
    /// - `avoid_negative_ts` — timestamp normalization
    /// - `max_interleave_delta=0` — disables delta-based queue flushing (safe
    ///   for remux since packets arrive in muxer order from a single input)
    #[allow(clippy::too_many_lines, clippy::needless_range_loop)]
    fn remux_mkv_raw_ffi(input: &Path, output: &Path) -> Result<()> {
        use ffmpeg_the_third::ffi;
        use std::ffi::CString;
        use std::ptr;

        debug!("MKV remux via raw FFI with avg_frame_rate + cluster_time_limit=500");

        let input_cstr = CString::new(input.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid input path: {e}"),
            }
        })?;
        let output_cstr = CString::new(output.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid output path: {e}"),
            }
        })?;

        unsafe {
            // 1. Open input
            let mut ifmt_ctx: *mut ffi::AVFormatContext = ptr::null_mut();
            let ret = ffi::avformat_open_input(
                &mut ifmt_ctx,
                input_cstr.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to open input: error code {ret}"),
                });
            }

            let ret = ffi::avformat_find_stream_info(ifmt_ctx, ptr::null_mut());
            if ret < 0 {
                ffi::avformat_close_input(&mut ifmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to find stream info: error code {ret}"),
                });
            }

            // 2. Create output context - EXPLICITLY request Matroska muxer
            let mut ofmt_ctx: *mut ffi::AVFormatContext = ptr::null_mut();
            let matroska_name = CString::new("matroska").unwrap();
            let ret = ffi::avformat_alloc_output_context2(
                &mut ofmt_ctx,
                ptr::null(),
                matroska_name.as_ptr(),
                output_cstr.as_ptr(),
            );
            if ret < 0 || ofmt_ctx.is_null() {
                ffi::avformat_close_input(&mut ifmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to create output context: error code {ret}"),
                });
            }

            // 3. Copy streams with FULL property preservation (like CLI does)
            let nb_streams = (*ifmt_ctx).nb_streams as usize;
            let mut stream_mapping: Vec<i32> = vec![-1; nb_streams];
            let mut out_stream_idx = 0i32;

            for i in 0..nb_streams {
                let in_stream = *(*ifmt_ctx).streams.add(i);
                let codecpar = (*in_stream).codecpar;
                let codec_type = (*codecpar).codec_type;

                // Only copy video, audio, subtitle streams
                if codec_type != ffi::AVMediaType::AVMEDIA_TYPE_VIDEO
                    && codec_type != ffi::AVMediaType::AVMEDIA_TYPE_AUDIO
                    && codec_type != ffi::AVMediaType::AVMEDIA_TYPE_SUBTITLE
                {
                    continue;
                }

                stream_mapping[i] = out_stream_idx;
                out_stream_idx += 1;

                let out_stream = ffi::avformat_new_stream(ofmt_ctx, ptr::null());
                if out_stream.is_null() {
                    ffi::avformat_close_input(&mut ifmt_ctx);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: "Failed to create output stream".into(),
                    });
                }

                // Copy codec parameters
                let ret = ffi::avcodec_parameters_copy((*out_stream).codecpar, codecpar);
                if ret < 0 {
                    ffi::avformat_close_input(&mut ifmt_ctx);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!("Failed to copy codec params: error code {ret}"),
                    });
                }

                // Reset codec tag for container compatibility
                (*(*out_stream).codecpar).codec_tag = 0;

                // ============================================================
                // CRITICAL: Copy stream properties that CLI copies but we missed
                // ============================================================

                // Copy time_base (for stream copy, preserve source timing)
                (*out_stream).time_base = (*in_stream).time_base;

                // Copy avg_frame_rate (CRITICAL for Matroska "Default duration")
                (*out_stream).avg_frame_rate = (*in_stream).avg_frame_rate;

                // Copy r_frame_rate (real base frame rate)
                (*out_stream).r_frame_rate = (*in_stream).r_frame_rate;

                // Copy sample_aspect_ratio
                (*out_stream).sample_aspect_ratio = (*in_stream).sample_aspect_ratio;

                log::debug!(
                    "Stream {i}: time_base={}/{}, avg_frame_rate={}/{}, r_frame_rate={}/{}",
                    (*out_stream).time_base.num,
                    (*out_stream).time_base.den,
                    (*out_stream).avg_frame_rate.num,
                    (*out_stream).avg_frame_rate.den,
                    (*out_stream).r_frame_rate.num,
                    (*out_stream).r_frame_rate.den,
                );
            }

            // 4. Set format context options (like CLI does)
            // Enable avoid_negative_ts to normalize timestamps
            (*ofmt_ctx).avoid_negative_ts = ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;

            // Disable delta-based interleave flushing. Safe for remux since
            // packets arrive in muxer order from a single input, keeping the
            // interleave queue small. 0 = no delta limit (not "flush immediately").
            (*ofmt_ctx).max_interleave_delta = 0;

            // Enable auto bitstream filters
            (*ofmt_ctx).flags |= ffi::AVFMT_FLAG_AUTO_BSF;

            // 5. Open output file (AVIO)
            if ((*(*ofmt_ctx).oformat).flags & ffi::AVFMT_NOFILE) == 0 {
                let ret = ffi::avio_open(
                    &mut (*ofmt_ctx).pb,
                    output_cstr.as_ptr(),
                    ffi::AVIO_FLAG_WRITE,
                );
                if ret < 0 {
                    ffi::avformat_close_input(&mut ifmt_ctx);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!("Failed to open output file: error code {ret}"),
                    });
                }
            }

            // 6. Build options dictionary with cluster_time_limit
            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            let key = CString::new("cluster_time_limit").unwrap();
            let value = CString::new("500").unwrap();
            ffi::av_dict_set(&mut opts, key.as_ptr(), value.as_ptr(), 0);

            // 7. Initialize muxer with options
            let ret = ffi::avformat_init_output(ofmt_ctx, &mut opts);

            // Check for unconsumed options
            let mut e: *mut ffi::AVDictionaryEntry = ptr::null_mut();
            loop {
                e = ffi::av_dict_get(opts, c"".as_ptr(), e, ffi::AV_DICT_IGNORE_SUFFIX);
                if e.is_null() {
                    break;
                }
                let k = std::ffi::CStr::from_ptr((*e).key).to_string_lossy();
                let v = std::ffi::CStr::from_ptr((*e).value).to_string_lossy();
                log::warn!("Unconsumed FFI option: {k}={v}");
            }
            ffi::av_dict_free(&mut opts);

            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut ifmt_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avformat_init_output failed: error code {ret}"),
                });
            }

            // 8. Write header
            let ret = ffi::avformat_write_header(ofmt_ctx, ptr::null_mut());
            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut ifmt_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avformat_write_header failed: error code {ret}"),
                });
            }

            // 9. Copy packets
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
                let ret = ffi::av_read_frame(ifmt_ctx, &mut pkt);
                if ret < 0 {
                    break; // EOF or error
                }

                let in_stream_idx = pkt.stream_index as usize;
                if in_stream_idx >= nb_streams || stream_mapping[in_stream_idx] < 0 {
                    ffi::av_packet_unref(&mut pkt);
                    continue;
                }

                let out_stream_idx = stream_mapping[in_stream_idx];
                pkt.stream_index = out_stream_idx;

                // Rescale timestamps
                let in_stream = *(*ifmt_ctx).streams.add(in_stream_idx);
                let out_stream = *(*ofmt_ctx).streams.add(out_stream_idx as usize);

                // Guard against AV_NOPTS_VALUE before rescaling
                if pkt.pts != ffi::AV_NOPTS_VALUE {
                    pkt.pts = ffi::av_rescale_q_rnd(
                        pkt.pts,
                        (*in_stream).time_base,
                        (*out_stream).time_base,
                        ffi::AVRounding::AV_ROUND_NEAR_INF,
                    );
                }
                if pkt.dts != ffi::AV_NOPTS_VALUE {
                    pkt.dts = ffi::av_rescale_q_rnd(
                        pkt.dts,
                        (*in_stream).time_base,
                        (*out_stream).time_base,
                        ffi::AVRounding::AV_ROUND_NEAR_INF,
                    );
                }
                if pkt.duration > 0 {
                    pkt.duration = ffi::av_rescale_q(
                        pkt.duration,
                        (*in_stream).time_base,
                        (*out_stream).time_base,
                    );
                }
                pkt.pos = -1;

                let ret = ffi::av_interleaved_write_frame(ofmt_ctx, &mut pkt);
                ffi::av_packet_unref(&mut pkt);

                if ret < 0 {
                    log::error!("Error writing packet: {ret}");
                    break;
                }
            }

            // 10. Write trailer and cleanup
            ffi::av_write_trailer(ofmt_ctx);

            if !(*ofmt_ctx).pb.is_null() {
                ffi::avio_closep(&mut (*ofmt_ctx).pb);
            }
            ffi::avformat_close_input(&mut ifmt_ctx);
            ffi::avformat_free_context(ofmt_ctx);
        }

        Ok(())
    }
}
