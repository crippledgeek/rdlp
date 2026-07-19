//! Video + audio stream merging (stream copy).
//!
//! Uses two-way timestamp-interleaved merging to avoid ENOMEM when
//! `av_interleaved_write_frame` buffers one complete stream waiting
//! for the other.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `FFmpeg` APIs use mixed C integer types (timestamps as
//!   `i64`/`u64`, stream counts as `usize`/`i32`). Each cast is audited and
//!   within valid ranges for `FFmpeg`-returned values.
//! - `clippy::borrow_as_ptr`: `&mut (*ctx).field` required for `**AVDictionary` APIs.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::borrow_as_ptr,
    clippy::similar_names,  // ist_index/ost_index, video_stream/audio_stream are FFmpeg convention
    clippy::match_same_arms,  // future per-arm tuning
)]

mod av_packet_owned;
mod mkv_raw_ffi;
mod raw_ffi_helpers;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use log::info;
use tokio_util::sync::CancellationToken;

use crate::error::{PostProcessError, Result};

use super::ffi_helpers::cleanup_partial_output;
use super::log_capture::LogSuppressGuard;
use super::{FFmpegRunner, RemuxOptions, ensure_init};

use av_packet_owned::AvPacketOwned;
use raw_ffi_helpers::{dts_in_us, out_codecpar_video_delay, read_next_raw, rescale_and_write_raw};

impl FFmpegRunner {
    /// Merge separate video and audio files into a single container (stream copy).
    ///
    /// Takes two input files (one containing video, one containing audio) and
    /// combines them into a single output file without re-encoding.
    /// The MP4 muxer automatically handles AAC ADTS->ASC conversion when needed.
    ///
    /// # Errors
    ///
    /// Returns an error if `FFmpeg` fails to open the input files, create the
    /// output container, or write packets (including I/O errors and mux failures).
    pub async fn merge(
        &self,
        video_input: impl AsRef<Path>,
        audio_input: impl AsRef<Path>,
        output: impl AsRef<Path>,
        opts: &RemuxOptions,
        progress_fn: Option<Arc<dyn Fn(f64) + Send + Sync>>,
        cancel: Option<CancellationToken>,
    ) -> Result<()> {
        let video_input = video_input.as_ref().to_path_buf();
        let audio_input = audio_input.as_ref().to_path_buf();
        let output = output.as_ref().to_path_buf();
        let opts = opts.clone();
        Self::spawn_blocking("merge", move || -> Result<()> {
            Ok(Self::merge_sync(
                &video_input,
                &audio_input,
                &output,
                &opts,
                progress_fn.as_deref(),
                cancel.as_ref(),
            )?)
        })
        .await
    }

    /// Merge separate video and audio files synchronously (stream copy).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn merge_sync(
        video_input: &Path,
        audio_input: &Path,
        output: &Path,
        opts: &RemuxOptions,
        progress_fn: Option<&(dyn Fn(f64) + Send + Sync)>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<()> {
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
            return Ok(Self::merge_mkv_raw_ffi(
                video_input,
                audio_input,
                output,
                progress_fn,
                opts.encoding_tool_override.as_deref(),
                cancel,
            )?);
        }

        let mut ictx_video = ffmpeg_the_third::format::input(video_input)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open video input for merge {}",
                    video_input.display()
                )
            })?;

        let mut ictx_audio = ffmpeg_the_third::format::input(audio_input)
            .map_err(PostProcessError::from)
            .with_context(|| {
                format!(
                    "failed to open audio input for merge {}",
                    audio_input.display()
                )
            })?;

        let mut octx = ffmpeg_the_third::format::output(output)
            .map_err(PostProcessError::from)
            .with_context(|| format!("failed to open output for merge {}", output.display()))?;

        // Find best video stream from video input
        let video_ist_index = ictx_video
            .streams()
            .best(ffmpeg_the_third::media::Type::Video)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoVideoStream)?;

        // `format::output` above already created (truncated) `output` on
        // disk via `avio_open` — a codec-tag rejection on either stream
        // below must not leave that empty file behind as if a merge had
        // run and produced nothing.
        let video_ost_index = Self::add_stream_copy(
            &mut octx,
            ictx_video
                .stream(video_ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "video input stream {video_ist_index} not found"
                    ))
                })?
                .parameters(),
            "for merge video",
        )
        .inspect_err(|_| cleanup_partial_output(output))?;

        // Find best audio stream from audio input
        let audio_ist_index = ictx_audio
            .streams()
            .best(ffmpeg_the_third::media::Type::Audio)
            .map(|s| s.index())
            .ok_or(PostProcessError::NoAudioStream)?;

        let audio_ost_index = Self::add_stream_copy(
            &mut octx,
            ictx_audio
                .stream(audio_ist_index)
                .ok_or_else(|| {
                    PostProcessError::ffmpeg_failed(format!(
                        "audio input stream {audio_ist_index} not found"
                    ))
                })?
                .parameters(),
            "for merge audio",
        )
        .inspect_err(|_| cleanup_partial_output(output))?;
        // Set audio as default stream so players select it automatically
        if let Some(mut ost_audio) = octx.stream_mut(audio_ost_index) {
            unsafe {
                (*ost_audio.as_mut_ptr()).disposition =
                    ffmpeg_the_third::ffi::AV_DISPOSITION_DEFAULT;
            }
        }

        // Build muxer options dictionary
        let mut dict = ffmpeg_the_third::Dictionary::new();

        // MP4/MOV: enable faststart (moov atom at beginning) for streaming
        if opts.faststart {
            dict.set("movflags", "+faststart");
        }

        // Set format-level encoding_tool metadata
        if let Some(ref override_tag) = opts.encoding_tool_override {
            crate::ffmpeg::encoding_tag::set_encoding_tool(&mut octx, override_tag);
        } else {
            crate::ffmpeg::encoding_tag::set_encoding_tool_if_missing(&mut octx, "merge");
        }

        // Write header with options
        octx.write_header_with(dict)
            .map_err(PostProcessError::from)
            .context("failed to write output header for merge")?;

        info!("Merge: video=stream#{video_ost_index}, audio=stream#{audio_ost_index} (DEFAULT)");

        // Byte-based progress: sum input file sizes before starting
        // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
        #[allow(clippy::disallowed_methods)]
        let total_input_bytes = {
            std::fs::metadata(video_input).map_or(0, |m| m.len())
                + std::fs::metadata(audio_input).map_or(0, |m| m.len())
        };
        let mut bytes_written: u64 = 0;
        let mut last_progress = Instant::now();
        let throttle = Duration::from_millis(100);

        // Two-way timestamp-interleaved merge: read packets from both inputs
        // and write them in DTS order to avoid ENOMEM from buffering an entire
        // stream while waiting for the other.
        //
        // Set by the loop on cooperative cancel; checked after the unsafe block,
        // where the RAII-owned packets have already been dropped, so we bail
        // before write_trailer without finalizing a partial mux.
        let mut cancelled = false;

        // SAFETY: ictx_video/ictx_audio own valid AVFormatContext instances.
        // octx owns a valid, header-written output context.
        unsafe {
            use ffmpeg_the_third::ffi;

            let video_ctx = ictx_video.as_mut_ptr();
            let audio_ctx = ictx_audio.as_mut_ptr();
            let out_ctx = octx.as_mut_ptr();

            let in_video_stream = *(*video_ctx).streams.add(video_ist_index);
            let in_audio_stream = *(*audio_ctx).streams.add(audio_ist_index);

            // RAII-owned packets: dropped on every exit path (write-error `?`,
            // the cancel `break`, or normal loop completion), so a write error
            // propagated via `?` no longer leaks the packets. Mirrors the MKV
            // path in mkv_raw_ffi.rs.
            let vpkt_owned = AvPacketOwned::alloc()?;
            let apkt_owned = AvPacketOwned::alloc()?;
            let vpkt = vpkt_owned.as_ptr();
            let apkt = apkt_owned.as_ptr();

            // One DTS synthesizer per OUTPUT stream, seeded from the stream's
            // video_delay (B-frame reorder depth). av_write_frame is direct
            // (non-buffered), so matroskaenc/mp4 reject packets with an unset
            // dts; the synthesizer fills in a monotonic dts <= pts.
            let mut video_dts = crate::ffmpeg::DtsSynthesizer::new(out_codecpar_video_delay(
                out_ctx,
                video_ost_index as i32,
            ));
            // video_delay == 0 for all audio codecs (no B-frame reorder delay).
            let mut audio_dts = crate::ffmpeg::DtsSynthesizer::new(0);

            let mut have_video = read_next_raw(video_ctx, video_ist_index, vpkt);
            let mut have_audio = read_next_raw(audio_ctx, audio_ist_index, apkt);

            loop {
                // Cooperative cancel: set a flag and `break`. The RAII-owned
                // packets drop when the unsafe block ends; the `cancelled` check
                // after the block bails before write_trailer so a partial,
                // cancelled mux is never finalized.
                if cancel.is_some_and(CancellationToken::is_cancelled) {
                    cancelled = true;
                    break;
                }
                match (have_video, have_audio) {
                    (false, false) => break,
                    (true, false) => {
                        bytes_written += (*vpkt).size as u64;
                        rescale_and_write_raw(
                            vpkt,
                            in_video_stream,
                            out_ctx,
                            video_ost_index as i32,
                            &mut video_dts,
                        )?;
                        ffi::av_packet_unref(vpkt);
                        have_video = read_next_raw(video_ctx, video_ist_index, vpkt);
                    }
                    (false, true) => {
                        bytes_written += (*apkt).size as u64;
                        rescale_and_write_raw(
                            apkt,
                            in_audio_stream,
                            out_ctx,
                            audio_ost_index as i32,
                            &mut audio_dts,
                        )?;
                        ffi::av_packet_unref(apkt);
                        have_audio = read_next_raw(audio_ctx, audio_ist_index, apkt);
                    }
                    (true, true) => {
                        let v_us = dts_in_us((*vpkt).dts, in_video_stream);
                        let a_us = dts_in_us((*apkt).dts, in_audio_stream);

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
                            bytes_written += (*vpkt).size as u64;
                            rescale_and_write_raw(
                                vpkt,
                                in_video_stream,
                                out_ctx,
                                video_ost_index as i32,
                                &mut video_dts,
                            )?;
                            ffi::av_packet_unref(vpkt);
                            have_video = read_next_raw(video_ctx, video_ist_index, vpkt);
                        } else {
                            bytes_written += (*apkt).size as u64;
                            rescale_and_write_raw(
                                apkt,
                                in_audio_stream,
                                out_ctx,
                                audio_ost_index as i32,
                                &mut audio_dts,
                            )?;
                            ffi::av_packet_unref(apkt);
                            have_audio = read_next_raw(audio_ctx, audio_ist_index, apkt);
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

            // Unref any still-referenced packet data, then drop the RAII owners
            // which call av_packet_free. Explicit drops document the release
            // point; they would otherwise run at end-of-scope all the same.
            ffi::av_packet_unref(vpkt);
            ffi::av_packet_unref(apkt);
            drop(vpkt_owned);
            drop(apkt_owned);
        }

        // Cancelled: the RAII-owned packets were freed when the unsafe block
        // ended (Drop calls av_packet_free). The AVFormatContexts (ictx_video /
        // ictx_audio / octx) are owned ffmpeg-the-third Input/Output values —
        // RAII drop frees them with no leak. Skip write_trailer: do not finalize
        // a partial, cancelled mux.
        if cancelled {
            anyhow::bail!("cancelled by user");
        }

        // Emit final 1.0 on completion
        if let Some(ref progress) = progress_fn {
            progress(1.0);
        }

        octx.write_trailer()
            .map_err(PostProcessError::from)
            .context("failed to write output trailer for merge")?;

        Ok(())
    }
}
