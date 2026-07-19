//! MKV thumbnail embedding via raw FFI.
//!
//! Adds a thumbnail as a native Matroska attachment with proper
//! stream property copying and `cluster_time_limit=500` for VLC
//! compatibility.
//!
//! # Lint allowances
//!
//! - `clippy::borrow_as_ptr`: `&mut (*ctx).field` required for `FFmpeg` C APIs
//!   taking `**AVDictionary`. No safe abstraction exists.
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types. All casts are
//!   manually audited and within valid ranges for FFmpeg-returned values.
//! - `clippy::expect_used`: `CString::new("static literal")` cannot fail;
//!   `av_packet_alloc` OOM is unrecoverable.
//! - `clippy::indexing_slicing`: loop index comes from iterating the same
//!   pre-allocated range, so bounds are guaranteed by construction.

#![allow(
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::ffi::CString;
use std::ptr;

use log::debug;

use ffmpeg_the_third::ffi;

use crate::error::{PostProcessError, Result};

use super::super::FFmpegRunner;

impl FFmpegRunner {
    /// Embed thumbnail in MKV using raw FFI with full stream property copying.
    ///
    /// Like `remux_mkv_raw_ffi`, this copies all essential stream properties
    /// (`avg_frame_rate`, `time_base`, etc.) and sets `cluster_time_limit=500` for VLC.
    /// The thumbnail is added as a Matroska attachment stream.
    #[allow(clippy::too_many_lines, clippy::needless_range_loop)]
    pub(super) fn embed_thumbnail_mkv_raw_ffi(
        media: &std::path::Path,
        thumbnail: &std::path::Path,
        output: &std::path::Path,
        encoding_tool_override: Option<&str>,
    ) -> Result<()> {
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
            let matroska_name = CString::new("matroska").expect("static string has no null bytes");
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
                let ret = ffi::avcodec_parameters_copy((*out_stream).codecpar, codecpar);
                if ret < 0 {
                    ffi::avformat_close_input(&mut media_ctx);
                    ffi::avformat_close_input(&mut thumb_ctx);
                    ffi::avformat_free_context(ofmt_ctx);
                    return Err(PostProcessError::FFmpegLibraryError {
                        message: format!(
                            "Failed to copy codec params for stream {i}: error code {ret}"
                        ),
                    });
                }
                (*(*out_stream).codecpar).codec_tag = 0;

                // Copy per-stream metadata (preserves encoder tags set by RecodeStage)
                ffi::av_dict_copy(&mut (*out_stream).metadata, (*in_stream).metadata, 0);

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
            // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
            #[allow(clippy::disallowed_methods)]
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
            ptr::copy_nonoverlapping(
                thumb_data.as_ptr(),
                extradata.cast::<u8>(),
                thumb_data.len(),
            );
            (*codecpar).extradata = extradata.cast::<u8>();
            (*codecpar).extradata_size = thumb_data.len() as i32;

            // Set mimetype and filename metadata (required by Matroska muxer).
            //
            // Derived from the THUMBNAIL BYTES (`thumb_data`, already read
            // above) via `rdlp_types::sniff_thumbnail_format` — the same
            // content sniffer used everywhere else in rdlp (download naming,
            // discovery, the MP4 `covr` atom) — rather than a second,
            // parallel `AVCodecID` table. `thumb_codec_id` above is a
            // separate concern: it only tags the attachment stream's
            // `codecpar.codec_id`, which no downstream reader consults for
            // cover-art rendering.
            //
            // `ThumbnailFormat::matroska_attachment` is the single source of
            // truth for which formats the linked `FFmpeg` build actually
            // renders as a visible cover (jpeg/png/gif/tiff) versus which
            // silently produce an invisible, generic attachment (bmp/webp) —
            // see #530. A format that resolves to `None` here means the
            // caller failed to normalize it first; that is a contract
            // violation, not something to paper over with a fallback
            // mimetype (the exact defect #530 reports: gif/tiff/bmp silently
            // relabeled `image/jpeg`).
            let sniffed = rdlp_types::sniff_thumbnail_format(&thumb_data);
            let Some((mimetype, filename)) =
                sniffed.and_then(rdlp_types::ThumbnailFormat::matroska_attachment)
            else {
                let reason = sniffed.map_or_else(
                    || "unrecognized image bytes".to_string(),
                    |format| format!("{} content", format.extension()),
                );
                ffi::avformat_close_input(&mut media_ctx);
                ffi::avformat_close_input(&mut thumb_ctx);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!(
                        "cannot attach {reason} as a Matroska cover without normalizing to \
                         jpeg/png/gif/tiff first"
                    ),
                });
            };

            let key_mime = CString::new("mimetype").expect("static string has no null bytes");
            let val_mime = CString::new(mimetype).expect("mimetype has no null bytes");
            ffi::av_dict_set(
                &mut (*out_stream).metadata,
                key_mime.as_ptr(),
                val_mime.as_ptr(),
                0,
            );

            let key_fname = CString::new("filename").expect("static string has no null bytes");
            let val_fname = CString::new(filename).expect("filename has no null bytes");
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
            let key = CString::new("cluster_time_limit").expect("static string has no null bytes");
            let value = CString::new("500").expect("static string has no null bytes");
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

            // Copy format-level metadata from media input (preserves encoding_tool from prior stages)
            ffi::av_dict_copy(&mut (*ofmt_ctx).metadata, (*media_ctx).metadata, 0);

            // Set encoding_tool: use override if provided, otherwise fall back to "thumbnail"
            if let Some(tag) = encoding_tool_override {
                crate::ffmpeg::encoding_tag::set_encoding_tool_ffi(ofmt_ctx, tag);
            } else {
                crate::ffmpeg::encoding_tag::set_encoding_tool_ffi_if_missing(
                    ofmt_ctx,
                    "thumbnail",
                );
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

            // One DTS synthesizer per OUTPUT stream, seeded from its
            // codecpar.video_delay (B-frame reorder depth). Used in the write
            // loop below to fill in dts that the demuxer left unset.
            let nb_out = (*ofmt_ctx).nb_streams as usize;
            let mut dts_synths: Vec<crate::ffmpeg::DtsSynthesizer> = (0..nb_out)
                .map(|i| {
                    let st = *(*ofmt_ctx).streams.add(i);
                    let delay = if st.is_null() || (*st).codecpar.is_null() {
                        0
                    } else {
                        (*(*st).codecpar).video_delay
                    };
                    crate::ffmpeg::DtsSynthesizer::new(delay)
                })
                .collect();

            // 10. Copy media packets
            // Errors are captured and propagated after cleanup.
            let copy_result: Result<()> = (|| {
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

                    // Synthesize a monotonic dts <= pts when the source left it unset (MKV
                    // demuxer B-frame warmup, or raw sources). Mirrors libavformat's own
                    // reconstruction so matroskaenc neither warns ("Timestamps are unset")
                    // nor hard-fails ("unknown timestamp"). See dts_synth.rs.
                    let opt = |v: i64| (v != ffi::AV_NOPTS_VALUE).then_some(v);
                    // Checked index (survives release; a negative idx casts to a
                    // huge usize -> None -> a clean panic, never out-of-bounds UB).
                    let synth = dts_synths
                        .get_mut(out_stream_idx as usize)
                        .expect("out_stream_idx within output stream count");
                    pkt.dts = synth.next_dts(opt(pkt.pts), opt(pkt.dts), pkt.duration);

                    let ret = ffi::av_interleaved_write_frame(ofmt_ctx, &mut pkt);
                    ffi::av_packet_unref(&mut pkt);

                    if ret < 0 {
                        return Err(PostProcessError::FFmpegLibraryError {
                            message: format!("av_interleaved_write_frame failed: error code {ret}"),
                        });
                    }
                }

                Ok(())
            })();

            // 11. Cleanup (always runs, even on copy error)
            ffi::av_write_trailer(ofmt_ctx);

            if !(*ofmt_ctx).pb.is_null() {
                ffi::avio_closep(&mut (*ofmt_ctx).pb);
            }
            ffi::avformat_close_input(&mut media_ctx);
            ffi::avformat_close_input(&mut thumb_ctx);
            ffi::avformat_free_context(ofmt_ctx);

            // Propagate copy errors after cleanup
            copy_result?;
        }

        Ok(())
    }
}
