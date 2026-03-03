//! Video + audio stream merging (stream copy).
//!
//! Uses two-way timestamp-interleaved merging to avoid ENOMEM when
//! `av_interleaved_write_frame` buffers one complete stream waiting
//! for the other.

mod mkv_raw_ffi;
mod raw_ffi_helpers;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use log::info;

use crate::error::{PostProcessError, Result};

use super::log_capture::LogSuppressGuard;
use super::{FFmpegRunner, RemuxOptions, ensure_init};

use raw_ffi_helpers::{dts_in_us, read_next_raw, rescale_and_write_raw};

impl FFmpegRunner {
    /// Merge separate video and audio files into a single container (stream copy).
    ///
    /// Takes two input files (one containing video, one containing audio) and
    /// combines them into a single output file without re-encoding.
    /// The MP4 muxer automatically handles AAC ADTS->ASC conversion when needed.
    pub async fn merge(
        &self,
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
        progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>>,
    ) -> Result<()> {
        let video_input = video_input.as_ref().to_path_buf();
        let audio_input = audio_input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("merge", move || {
            Self::merge_sync(
                &video_input,
                &audio_input,
                &output,
                &opts,
                progress_fn.as_deref(),
            )
        })
        .await
    }

    /// Merge separate video and audio files synchronously (stream copy).
    pub(crate) fn merge_sync(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
        opts: &RemuxOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
    ) -> Result<()> {
        ensure_init()?;

        // Suppress FFmpeg's internal muxer trace/debug spam (e.g. matroska "Writing block"
        // messages) while keeping actual errors visible.
        let _log_suppress = LogSuppressGuard::error_level();

        // MKV: use raw FFI with proper stream property copying for VLC compatibility.
        // The key is copying avg_frame_rate which sets Matroska's "Default duration" element.
        let is_mkv = output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mkv"));
        if is_mkv {
            return Self::merge_mkv_raw_ffi(video_input, audio_input, output, progress_fn);
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
        // Set audio as default stream so players select it automatically
        unsafe {
            (*ost_audio.as_mut_ptr()).disposition = ffmpeg_the_third::ffi::AV_DISPOSITION_DEFAULT;
        }
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

        info!("Merge: video=stream#{video_ost_index}, audio=stream#{audio_ost_index} (DEFAULT)");

        // Byte-based progress: sum input file sizes before starting
        let total_input_bytes = std::fs::metadata(video_input).map(|m| m.len()).unwrap_or(0)
            + std::fs::metadata(audio_input).map(|m| m.len()).unwrap_or(0);
        let mut bytes_written: u64 = 0;
        let mut last_progress = Instant::now();
        let throttle = Duration::from_millis(100);

        // Two-way timestamp-interleaved merge: read packets from both inputs
        // and write them in DTS order to avoid ENOMEM from buffering an entire
        // stream while waiting for the other.
        //
        // SAFETY: ictx_video/ictx_audio own valid AVFormatContext instances.
        // octx owns a valid, header-written output context.
        unsafe {
            use ffmpeg_the_third::ffi;

            let video_ctx = ictx_video.as_mut_ptr();
            let audio_ctx = ictx_audio.as_mut_ptr();
            let out_ctx = octx.as_mut_ptr();

            let in_video_stream = *(*video_ctx).streams.add(video_ist_index);
            let in_audio_stream = *(*audio_ctx).streams.add(audio_ist_index);

            let mut vpkt: ffi::AVPacket = std::mem::zeroed();
            vpkt.pts = ffi::AV_NOPTS_VALUE;
            vpkt.dts = ffi::AV_NOPTS_VALUE;
            vpkt.pos = -1;

            let mut apkt: ffi::AVPacket = std::mem::zeroed();
            apkt.pts = ffi::AV_NOPTS_VALUE;
            apkt.dts = ffi::AV_NOPTS_VALUE;
            apkt.pos = -1;

            let mut have_video = read_next_raw(video_ctx, video_ist_index, &mut vpkt);
            let mut have_audio = read_next_raw(audio_ctx, audio_ist_index, &mut apkt);

            loop {
                match (have_video, have_audio) {
                    (false, false) => break,
                    (true, false) => {
                        bytes_written += vpkt.size as u64;
                        rescale_and_write_raw(
                            &mut vpkt,
                            in_video_stream,
                            out_ctx,
                            video_ost_index as i32,
                        )?;
                        ffi::av_packet_unref(&mut vpkt);
                        have_video = read_next_raw(video_ctx, video_ist_index, &mut vpkt);
                    }
                    (false, true) => {
                        bytes_written += apkt.size as u64;
                        rescale_and_write_raw(
                            &mut apkt,
                            in_audio_stream,
                            out_ctx,
                            audio_ost_index as i32,
                        )?;
                        ffi::av_packet_unref(&mut apkt);
                        have_audio = read_next_raw(audio_ctx, audio_ist_index, &mut apkt);
                    }
                    (true, true) => {
                        let v_us = dts_in_us(vpkt.dts, in_video_stream);
                        let a_us = dts_in_us(apkt.dts, in_audio_stream);

                        let write_video = match (v_us, a_us) {
                            // Both NOPTS: write video first (arbitrary)
                            (None, None) => true,
                            // Video NOPTS: write it immediately
                            (None, Some(_)) => true,
                            // Audio NOPTS: write it immediately
                            (Some(_), None) => false,
                            // Both have DTS: write earlier one
                            (Some(v), Some(a)) => v <= a,
                        };

                        if write_video {
                            bytes_written += vpkt.size as u64;
                            rescale_and_write_raw(
                                &mut vpkt,
                                in_video_stream,
                                out_ctx,
                                video_ost_index as i32,
                            )?;
                            ffi::av_packet_unref(&mut vpkt);
                            have_video = read_next_raw(video_ctx, video_ist_index, &mut vpkt);
                        } else {
                            bytes_written += apkt.size as u64;
                            rescale_and_write_raw(
                                &mut apkt,
                                in_audio_stream,
                                out_ctx,
                                audio_ost_index as i32,
                            )?;
                            ffi::av_packet_unref(&mut apkt);
                            have_audio = read_next_raw(audio_ctx, audio_ist_index, &mut apkt);
                        }
                    }
                }

                // Report byte-based progress (throttled to 10 updates/sec)
                if let Some(ref progress) = progress_fn
                    && total_input_bytes > 0
                    && last_progress.elapsed() >= throttle
                {
                    let frac = (bytes_written as f64 / total_input_bytes as f64).clamp(0.0, 1.0);
                    progress(frac);
                    last_progress = Instant::now();
                }
            }

            // Clean up any unreleased packets
            ffi::av_packet_unref(&mut vpkt);
            ffi::av_packet_unref(&mut apkt);
        }

        // Emit final 1.0 on completion
        if let Some(ref progress) = progress_fn {
            progress(1.0);
        }

        octx.write_trailer()
            .map_err(|e| PostProcessError::FFmpegLibraryError {
                message: format!("failed to write output trailer: {e}"),
            })?;

        Ok(())
    }
}
