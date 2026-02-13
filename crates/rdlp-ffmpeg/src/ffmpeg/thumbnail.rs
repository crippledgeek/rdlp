//! Thumbnail embedding into media containers.
//!
//! Container-specific strategies:
//! - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
//! - **MKV/MKA**: Native Matroska attachment via raw FFI
//! - **MP3**: Map audio only + thumbnail as video with ID3v2 metadata
//! - **FLAC/OGG/Opus**: Map all streams + thumbnail with `ATTACHED_PIC`

use std::path::Path;

use log::debug;

use crate::error::{PostProcessError, Result};

use super::{FFmpegRunner, ensure_init};

impl FFmpegRunner {
    /// Embed a thumbnail image into a media file via stream copy (remux).
    ///
    /// Opens both the media file and thumbnail image, copies all media streams,
    /// and adds the thumbnail as a video stream with `ATTACHED_PIC` disposition.
    /// Container-specific handling for MKV (attachment) and MP3 (ID3v2).
    pub async fn embed_thumbnail(
        &self,
        media: impl AsRef<Path>,
        thumbnail: impl AsRef<Path>,
        output: impl AsRef<Path>,
        container: &str,
    ) -> Result<()> {
        let media = media.as_ref().to_path_buf();
        let thumbnail = thumbnail.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let container = container.to_string();
        Self::spawn_blocking("embed_thumbnail", move || {
            Self::embed_thumbnail_sync(&media, &thumbnail, &output, &container)
        })
        .await
    }

    /// Embed thumbnail synchronously.
    ///
    /// Strategy varies by container:
    /// - **MP4/MOV/M4A/M4V**: Map all streams + thumbnail as video with `ATTACHED_PIC`
    /// - **MKV/MKA**: Map all streams + thumbnail as attachment with mimetype metadata
    /// - **MP3**: Map audio only + thumbnail as video with ID3v2 metadata
    /// - **FLAC/OGG/Opus**: Map all streams + thumbnail with `ATTACHED_PIC`
    fn embed_thumbnail_sync(
        media: &Path,
        thumbnail: &Path,
        output: &Path,
        container: &str,
    ) -> Result<()> {
        ensure_init()?;

        // MKV: use raw FFI with proper stream property copying for VLC compatibility
        let is_mkv = container.eq_ignore_ascii_case("mkv") || container.eq_ignore_ascii_case("mka");
        if is_mkv {
            return Self::embed_thumbnail_mkv_raw_ffi(media, thumbnail, output);
        }

        // Open media input
        let mut ictx = ffmpeg_the_third::format::input(media).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open media input {}: {e}", media.display()),
            }
        })?;

        // Open thumbnail input
        let mut thumb_ictx = ffmpeg_the_third::format::input(thumbnail).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to open thumbnail {}: {e}", thumbnail.display()),
            }
        })?;

        // Create output
        let mut octx = ffmpeg_the_third::format::output(output).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("failed to create output {}: {e}", output.display()),
            }
        })?;

        let is_mp3 = container.eq_ignore_ascii_case("mp3");

        // Map media streams to output
        let stream_count = ictx.streams().count();
        let mut stream_mapping: Vec<i32> = vec![-1; stream_count];
        let mut ist_time_bases = vec![ffmpeg_the_third::Rational(0, 1); stream_count];
        let mut ost_index: i32 = 0;

        for (ist_index, ist) in ictx.streams().enumerate() {
            let medium = ist.parameters().medium();

            // For MP3: only map audio streams (thumbnail replaces any video)
            if is_mp3 && medium != ffmpeg_the_third::media::Type::Audio {
                continue;
            }

            if !matches!(
                medium,
                ffmpeg_the_third::media::Type::Video | ffmpeg_the_third::media::Type::Audio
            ) {
                continue;
            }

            stream_mapping[ist_index] = ost_index;
            ist_time_bases[ist_index] = ist.time_base();
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

        // Add thumbnail stream
        let thumb_ist = thumb_ictx
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .ok_or(PostProcessError::ffmpeg_failed(
                "no video stream found in thumbnail",
            ))?;
        let thumb_ist_index = thumb_ist.index();
        let thumb_ist_time_base = thumb_ist.time_base();
        let thumb_params = thumb_ist.parameters();

        // Add thumbnail as video stream with ATTACHED_PIC disposition
        let mut ost = octx
            .add_stream(ffmpeg_the_third::encoder::find(
                ffmpeg_the_third::codec::Id::None,
            ))
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to add thumbnail stream: {e}"),
            })?;
        let thumb_ost_index = ost.index();
        ost.set_parameters(thumb_params);
        // SAFETY: ost is a valid output stream in a live output context.
        Self::set_attached_pic_disposition(unsafe { ost.as_mut_ptr() });

        // For MP3: set ID3v2 metadata on the thumbnail stream
        if is_mp3 {
            let mut dict = ffmpeg_the_third::Dictionary::new();
            dict.set("title", "Album cover");
            dict.set("comment", "Cover (Front)");
            ost.set_metadata(dict);
        }

        // Copy format-level metadata from media input
        octx.set_metadata(ictx.metadata().to_owned());

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MP4/MOV: enable faststart (moov atom at beginning) for Windows Explorer thumbnail visibility
        let is_mp4_mov = container.eq_ignore_ascii_case("mp4")
            || container.eq_ignore_ascii_case("m4a")
            || container.eq_ignore_ascii_case("m4v")
            || container.eq_ignore_ascii_case("mov");
        if is_mp4_mov {
            dict.set("movflags", "+faststart");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output header: {e}"),
            })?;

        // For FLAC/OGG/Opus: write thumbnail packets BEFORE media packets.
        // These formats store picture metadata in the file header (METADATA_BLOCK_PICTURE
        // for FLAC, Vorbis comment for OGG/Opus), so the muxer needs picture data before
        // audio frames are flushed. For other formats (MP4, MP3), order doesn't matter.
        let is_header_picture_format = container.eq_ignore_ascii_case("flac")
            || container.eq_ignore_ascii_case("ogg")
            || container.eq_ignore_ascii_case("opus");

        if is_header_picture_format {
            Self::write_thumbnail_packets(
                &mut thumb_ictx,
                &mut octx,
                thumb_ist_index,
                thumb_ist_time_base,
                thumb_ost_index,
            )?;
        }

        // Copy media packets
        for result in ictx.packets() {
            let (stream, mut packet) =
                result.map_err(|e| PostProcessError::FFmpegLibraryError {
                    message: format!("failed to read media packet: {e}"),
                })?;
            let ist_index = stream.index();
            let ost_idx = stream_mapping[ist_index];
            if ost_idx < 0 {
                continue;
            }
            let ost_idx = ost_idx as usize;
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
                    message: format!("failed to write media packet: {e}"),
                }
            })?;
        }

        // Copy thumbnail packet(s) for formats that don't need them in the header.
        // MKV: handled by embed_thumbnail_mkv_raw_ffi. FLAC/OGG/Opus: already written above.
        if !is_header_picture_format {
            Self::write_thumbnail_packets(
                &mut thumb_ictx,
                &mut octx,
                thumb_ist_index,
                thumb_ist_time_base,
                thumb_ost_index,
            )?;
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }

    /// Embed thumbnail in MKV using raw FFI with full stream property copying.
    ///
    /// Like `remux_mkv_raw_ffi`, this copies all essential stream properties
    /// (avg_frame_rate, time_base, etc.) and sets cluster_time_limit=500 for VLC.
    /// The thumbnail is added as a Matroska attachment stream.
    #[allow(clippy::too_many_lines, clippy::needless_range_loop)]
    fn embed_thumbnail_mkv_raw_ffi(media: &Path, thumbnail: &Path, output: &Path) -> Result<()> {
        use ffmpeg_the_third::ffi;
        use std::ffi::CString;
        use std::ptr;

        debug!("MKV thumbnail embed as native Matroska attachment via raw FFI");

        let media_cstr = CString::new(media.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid media path: {e}"),
            }
        })?;
        let thumb_cstr = CString::new(thumbnail.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid thumbnail path: {e}"),
            }
        })?;
        let output_cstr = CString::new(output.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid output path: {e}"),
            }
        })?;

        unsafe {
            // 1. Open media input
            let mut media_ctx: *mut ffi::AVFormatContext = ptr::null_mut();
            let ret = ffi::avformat_open_input(
                &mut media_ctx,
                media_cstr.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
            if ret < 0 {
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to open media input: error code {ret}"),
                });
            }

            let ret = ffi::avformat_find_stream_info(media_ctx, ptr::null_mut());
            if ret < 0 {
                ffi::avformat_close_input(&mut media_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to find media stream info: error code {ret}"),
                });
            }

            // 2. Open thumbnail input
            let mut thumb_ctx: *mut ffi::AVFormatContext = ptr::null_mut();
            let ret = ffi::avformat_open_input(
                &mut thumb_ctx,
                thumb_cstr.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
            if ret < 0 {
                ffi::avformat_close_input(&mut media_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to open thumbnail: error code {ret}"),
                });
            }

            let ret = ffi::avformat_find_stream_info(thumb_ctx, ptr::null_mut());
            if ret < 0 {
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to find thumbnail stream info: error code {ret}"),
                });
            }

            // 3. Create output context
            let mut ofmt_ctx: *mut ffi::AVFormatContext = ptr::null_mut();
            let matroska_name = CString::new("matroska").unwrap();
            let ret = ffi::avformat_alloc_output_context2(
                &mut ofmt_ctx,
                ptr::null(),
                matroska_name.as_ptr(),
                output_cstr.as_ptr(),
            );
            if ret < 0 || ofmt_ctx.is_null() {
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to create output context: error code {ret}"),
                });
            }

            // 4. Copy media streams with full property preservation
            let nb_media_streams = (*media_ctx).nb_streams as usize;
            let mut stream_mapping: Vec<i32> = vec![-1; nb_media_streams];
            let mut out_stream_idx = 0i32;

            for i in 0..nb_media_streams {
                let in_stream = *(*media_ctx).streams.add(i);
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
                    ffi::avformat_close_input(&mut media_ctx);
                    ffi::avformat_close_input(&mut thumb_ctx);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: "Failed to create output stream".into(),
                    });
                }

                // Copy codec parameters
                ffi::avcodec_parameters_copy((*out_stream).codecpar, codecpar);
                (*(*out_stream).codecpar).codec_tag = 0;

                // Copy stream properties (critical for VLC)
                (*out_stream).time_base = (*in_stream).time_base;
                (*out_stream).avg_frame_rate = (*in_stream).avg_frame_rate;
                (*out_stream).r_frame_rate = (*in_stream).r_frame_rate;
                (*out_stream).sample_aspect_ratio = (*in_stream).sample_aspect_ratio;
            }

            // 5. Add thumbnail as native Matroska attachment (not a video track)
            // Detect image codec from thumbnail input
            let mut thumb_codec_id = ffi::AVCodecID::AV_CODEC_ID_MJPEG;
            for i in 0..(*thumb_ctx).nb_streams as usize {
                let in_stream = *(*thumb_ctx).streams.add(i);
                let codecpar = (*in_stream).codecpar;
                if (*codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO {
                    thumb_codec_id = (*codecpar).codec_id;
                    break;
                }
            }

            // Read raw thumbnail file bytes for attachment extradata
            let thumb_data = std::fs::read(thumbnail).map_err(|e| {
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                PostProcessError::FFmpegLibraryError {
                    message: format!("Failed to read thumbnail file: {e}"),
                }
            })?;

            let out_stream = ffi::avformat_new_stream(ofmt_ctx, ptr::null());
            if out_stream.is_null() {
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "Failed to create attachment stream".into(),
                });
            }

            // Configure as attachment stream
            let codecpar = (*out_stream).codecpar;
            (*codecpar).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_ATTACHMENT;
            (*codecpar).codec_id = thumb_codec_id;

            // Copy thumbnail data into extradata (must be av_malloc'd)
            let alloc_size = thumb_data.len() + ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize;
            let extradata = ffi::av_mallocz(alloc_size);
            if extradata.is_null() {
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: "Failed to allocate memory for thumbnail attachment".into(),
                });
            }
            ptr::copy_nonoverlapping(thumb_data.as_ptr(), extradata as *mut u8, thumb_data.len());
            (*codecpar).extradata = extradata as *mut u8;
            (*codecpar).extradata_size = thumb_data.len() as i32;

            // Set mimetype and filename metadata (required by Matroska muxer)
            let (mimetype, filename) = match thumb_codec_id {
                ffi::AVCodecID::AV_CODEC_ID_PNG => ("image/png", "cover.png"),
                ffi::AVCodecID::AV_CODEC_ID_WEBP => ("image/webp", "cover.webp"),
                _ => ("image/jpeg", "cover.jpg"),
            };

            let key_mime = CString::new("mimetype").unwrap();
            let val_mime = CString::new(mimetype).unwrap();
            ffi::av_dict_set(
                &mut (*out_stream).metadata,
                key_mime.as_ptr(),
                val_mime.as_ptr(),
                0,
            );

            let key_fname = CString::new("filename").unwrap();
            let val_fname = CString::new(filename).unwrap();
            ffi::av_dict_set(
                &mut (*out_stream).metadata,
                key_fname.as_ptr(),
                val_fname.as_ptr(),
                0,
            );

            // 6. Set format options
            (*ofmt_ctx).avoid_negative_ts = ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
            // Disable delta-based interleave flushing. 0 = no delta limit
            // (not "flush immediately"). Safe here because thumbnail embed
            // copies packets in input order from a single source.
            (*ofmt_ctx).max_interleave_delta = 0;
            (*ofmt_ctx).flags |= ffi::AVFMT_FLAG_AUTO_BSF;

            // 7. Open output file
            if ((*(*ofmt_ctx).oformat).flags & ffi::AVFMT_NOFILE) == 0 {
                let ret = ffi::avio_open(
                    &mut (*ofmt_ctx).pb,
                    output_cstr.as_ptr(),
                    ffi::AVIO_FLAG_WRITE,
                );
                if ret < 0 {
                    ffi::avformat_close_input(&mut media_ctx);
                    ffi::avformat_close_input(&mut thumb_ctx);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!("Failed to open output file: error code {ret}"),
                    });
                }
            }

            // 8. Set cluster_time_limit and write header
            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            let key = CString::new("cluster_time_limit").unwrap();
            let value = CString::new("500").unwrap();
            ffi::av_dict_set(&mut opts, key.as_ptr(), value.as_ptr(), 0);

            let ret = ffi::avformat_init_output(ofmt_ctx, &mut opts);
            ffi::av_dict_free(&mut opts);

            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avformat_init_output failed: error code {ret}"),
                });
            }

            let ret = ffi::avformat_write_header(ofmt_ctx, ptr::null_mut());
            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avformat_write_header failed: error code {ret}"),
                });
            }

            // 9. Thumbnail data is in attachment extradata — no packets to write

            // 10. Copy media packets
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
                let ret = ffi::av_read_frame(media_ctx, &mut pkt);
                if ret < 0 {
                    break;
                }

                let in_stream_idx = pkt.stream_index as usize;
                if in_stream_idx >= nb_media_streams || stream_mapping[in_stream_idx] < 0 {
                    ffi::av_packet_unref(&mut pkt);
                    continue;
                }

                let out_stream_idx = stream_mapping[in_stream_idx];
                pkt.stream_index = out_stream_idx;

                let in_stream = *(*media_ctx).streams.add(in_stream_idx);
                let out_stream = *(*ofmt_ctx).streams.add(out_stream_idx as usize);

                // Guard against AV_NOPTS_VALUE before rescaling (MKV demuxer may
                // not infer DTS for B-frame content, leaving it as AV_NOPTS_VALUE;
                // rescaling INT64_MIN overflows and causes the muxer to reject packets)
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

            // 11. Cleanup
            ffi::av_write_trailer(ofmt_ctx);

            if !(*ofmt_ctx).pb.is_null() {
                ffi::avio_closep(&mut (*ofmt_ctx).pb);
            }
            ffi::avformat_close_input(&mut media_ctx);
            ffi::avformat_close_input(&mut thumb_ctx);
            ffi::avformat_free_context(ofmt_ctx);
        }

        Ok(())
    }
}
