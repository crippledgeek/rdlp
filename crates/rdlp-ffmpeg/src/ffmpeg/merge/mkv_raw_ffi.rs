//! MKV merge via raw FFI with full stream property copying.
//!
//! Copies stream properties essential for proper Matroska playback:
//! `avg_frame_rate`, `r_frame_rate`, `time_base`, `sample_aspect_ratio`,
//! plus `cluster_time_limit=500` for VLC-compatible seeking.

use std::path::Path;

use log::{debug, info};

use crate::error::{PostProcessError, Result};

use super::super::FFmpegRunner;
use super::raw_ffi_helpers::{dts_in_us, read_next_raw, rescale_and_write_raw};

impl FFmpegRunner {
    /// Merge video + audio into MKV using raw FFI with full stream property copying.
    ///
    /// This copies stream properties that are essential for proper Matroska playback:
    /// - `avg_frame_rate` -- critical for "Default duration" element (VLC needs this)
    /// - `r_frame_rate` -- real base frame rate
    /// - `time_base` -- preserves source timing
    /// - `sample_aspect_ratio` -- pixel aspect ratio
    /// - `cluster_time_limit=500` -- 500ms clusters for smooth seeking
    /// - `avoid_negative_ts` -- timestamp normalization
    /// - `max_interleave_delta=0` -- disables delta-based queue flushing (packets
    ///   still flush via the two-way interleaved merge loop that feeds packets
    ///   in DTS order, so the queue stays small)
    #[allow(clippy::too_many_lines)]
    pub(crate) fn merge_mkv_raw_ffi(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
    ) -> Result<()> {
        use ffmpeg_the_third::ffi;
        use std::ffi::CString;
        use std::ptr;

        debug!("MKV merge via raw FFI with avg_frame_rate + cluster_time_limit=500");

        let video_cstr = CString::new(video_input.to_string_lossy().as_ref()).map_err(|e| {
            PostProcessError::FFmpegLibraryError {
                message: format!("Invalid video input path: {e}"),
            }
        })?;
        let audio_cstr = CString::new(audio_input.to_string_lossy().as_ref()).map_err(|e| {
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
                    message: format!("Failed to find video stream info: error code {ret}"),
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
                    message: format!("Failed to find audio stream info: error code {ret}"),
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
            let matroska_name = CString::new("matroska").expect("static string has no null bytes");
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
                    message: format!("Failed to create output context: error code {ret}"),
                });
            }

            // 6. Add video output stream with full property copying
            // Safety: avformat_find_stream_info guarantees streams is non-null and
            // has at least nb_streams valid entries.
            assert!(
                !(*ifmt_video).streams.is_null(),
                "streams must be non-null after avformat_find_stream_info"
            );
            let in_video_stream = *(*ifmt_video).streams.add(video_stream_idx);

            let out_video_stream = ffi::avformat_new_stream(ofmt_ctx, ptr::null());
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
                    message: format!("Failed to copy video codec params: error code {ret}"),
                });
            }
            (*(*out_video_stream).codecpar).codec_tag = 0;

            // CRITICAL: Copy stream timing properties for Matroska
            (*out_video_stream).time_base = (*in_video_stream).time_base;
            (*out_video_stream).avg_frame_rate = (*in_video_stream).avg_frame_rate;
            (*out_video_stream).r_frame_rate = (*in_video_stream).r_frame_rate;
            (*out_video_stream).sample_aspect_ratio = (*in_video_stream).sample_aspect_ratio;

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
            // Safety: avformat_find_stream_info guarantees streams is non-null and
            // has at least nb_streams valid entries.
            assert!(
                !(*ifmt_audio).streams.is_null(),
                "streams must be non-null after avformat_find_stream_info"
            );
            let in_audio_stream = *(*ifmt_audio).streams.add(audio_stream_idx);

            let out_audio_stream = ffi::avformat_new_stream(ofmt_ctx, ptr::null());
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
                    message: format!("Failed to copy audio codec params: error code {ret}"),
                });
            }
            (*(*out_audio_stream).codecpar).codec_tag = 0;

            // Copy audio stream timing properties
            (*out_audio_stream).time_base = (*in_audio_stream).time_base;
            (*out_audio_stream).avg_frame_rate = (*in_audio_stream).avg_frame_rate;
            (*out_audio_stream).r_frame_rate = (*in_audio_stream).r_frame_rate;
            (*out_audio_stream).sample_aspect_ratio = (*in_audio_stream).sample_aspect_ratio;

            // Set audio as default stream so players select it automatically
            (*out_audio_stream).disposition = ffi::AV_DISPOSITION_DEFAULT;

            let audio_out_idx = (*out_audio_stream).index;

            debug!(
                "Audio stream: time_base={}/{}, avg_frame_rate={}/{}",
                (*out_audio_stream).time_base.num,
                (*out_audio_stream).time_base.den,
                (*out_audio_stream).avg_frame_rate.num,
                (*out_audio_stream).avg_frame_rate.den,
            );

            // 8. Set format context options
            (*ofmt_ctx).avoid_negative_ts = ffi::AVFMT_AVOID_NEG_TS_MAKE_NON_NEGATIVE;
            // Disable delta-based interleave flushing. Safe here because the
            // two-way merge loop already feeds packets in DTS order, keeping
            // the interleave queue small. 0 = no delta limit (not "flush immediately").
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
                        message: format!("Failed to open output file: error code {ret}"),
                    });
                }
            }

            // 10. Build options dictionary with cluster_time_limit
            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            let key = CString::new("cluster_time_limit").expect("static string has no null bytes");
            let value = CString::new("500").expect("static string has no null bytes");
            ffi::av_dict_set(&mut opts, key.as_ptr(), value.as_ptr(), 0);

            // 11. Initialize muxer with options
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
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avformat_init_output failed: error code {ret}"),
                });
            }

            // 12. Write header
            let ret = ffi::avformat_write_header(ofmt_ctx, ptr::null_mut());
            if ret < 0 {
                if !(*ofmt_ctx).pb.is_null() {
                    ffi::avio_closep(&mut (*ofmt_ctx).pb);
                }
                ffi::avformat_close_input(&mut ifmt_video);
                ffi::avformat_close_input(&mut ifmt_audio);
                ffi::avformat_free_context(ofmt_ctx);
                return Err(PostProcessError::FFmpegLibraryError {
                    message: format!("avformat_write_header failed: error code {ret}"),
                });
            }

            info!("Merge: video=stream#{video_out_idx}, audio=stream#{audio_out_idx} (DEFAULT)");

            // 13. Two-way timestamp-interleaved merge
            //
            // Write packets from both inputs in DTS order to avoid ENOMEM
            // from buffering an entire stream while waiting for the other.
            // Errors are captured and propagated after cleanup.
            let merge_result: Result<()> = (|| {
                let mut vpkt: ffi::AVPacket = std::mem::zeroed();
                vpkt.pts = ffi::AV_NOPTS_VALUE;
                vpkt.dts = ffi::AV_NOPTS_VALUE;
                vpkt.pos = -1;

                let mut apkt: ffi::AVPacket = std::mem::zeroed();
                apkt.pts = ffi::AV_NOPTS_VALUE;
                apkt.dts = ffi::AV_NOPTS_VALUE;
                apkt.pos = -1;

                let mut have_video = read_next_raw(ifmt_video, video_stream_idx, &mut vpkt);
                let mut have_audio = read_next_raw(ifmt_audio, audio_stream_idx, &mut apkt);

                loop {
                    match (have_video, have_audio) {
                        (false, false) => break,
                        (true, false) => {
                            rescale_and_write_raw(
                                &mut vpkt,
                                in_video_stream,
                                ofmt_ctx,
                                video_out_idx,
                            )?;
                            ffi::av_packet_unref(&mut vpkt);
                            have_video = read_next_raw(ifmt_video, video_stream_idx, &mut vpkt);
                        }
                        (false, true) => {
                            rescale_and_write_raw(
                                &mut apkt,
                                in_audio_stream,
                                ofmt_ctx,
                                audio_out_idx,
                            )?;
                            ffi::av_packet_unref(&mut apkt);
                            have_audio = read_next_raw(ifmt_audio, audio_stream_idx, &mut apkt);
                        }
                        (true, true) => {
                            let v_us = dts_in_us(vpkt.dts, in_video_stream);
                            let a_us = dts_in_us(apkt.dts, in_audio_stream);

                            let write_video = match (v_us, a_us) {
                                (None, None) => true,
                                (None, Some(_)) => true,
                                (Some(_), None) => false,
                                (Some(v), Some(a)) => v <= a,
                            };

                            if write_video {
                                rescale_and_write_raw(
                                    &mut vpkt,
                                    in_video_stream,
                                    ofmt_ctx,
                                    video_out_idx,
                                )?;
                                ffi::av_packet_unref(&mut vpkt);
                                have_video = read_next_raw(ifmt_video, video_stream_idx, &mut vpkt);
                            } else {
                                rescale_and_write_raw(
                                    &mut apkt,
                                    in_audio_stream,
                                    ofmt_ctx,
                                    audio_out_idx,
                                )?;
                                ffi::av_packet_unref(&mut apkt);
                                have_audio = read_next_raw(ifmt_audio, audio_stream_idx, &mut apkt);
                            }
                        }
                    }
                }

                ffi::av_packet_unref(&mut vpkt);
                ffi::av_packet_unref(&mut apkt);
                Ok(())
            })();

            // 14. Write trailer and cleanup (always runs, even on merge error)
            ffi::av_write_trailer(ofmt_ctx);

            if !(*ofmt_ctx).pb.is_null() {
                ffi::avio_closep(&mut (*ofmt_ctx).pb);
            }
            ffi::avformat_close_input(&mut ifmt_video);
            ffi::avformat_close_input(&mut ifmt_audio);
            ffi::avformat_free_context(ofmt_ctx);

            // Propagate merge errors after cleanup
            merge_result?;
        }

        Ok(())
    }
}
