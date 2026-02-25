//! Audio encode/mux pipeline: interleaved write fallback path.
//!
//! Provides `receive_and_process_audio_interleaved`,
//! `drain_filter_to_encoder_interleaved`, and
//! `drain_encoder_packets_interleaved` as a fallback for the primary
//! direct-write path. Uses `av_interleaved_write_frame` with
//! `AVFMT_FLAG_FLUSH_PACKETS`.

use std::path::Path;

use log::{debug, info, warn};

use ffmpeg_the_third::packet::Ref as _;

use crate::error::{PostProcessError, Result};

use super::super::FFmpegRunner;
use super::super::ffi_helpers::{frame_unref_audio, packet_unref};
use super::mux_timing::{
    MuxTimingState, av_strerror_string, diagnose_mux_io, fix_audio_timestamps, get_process_rss_kb,
};

impl FFmpegRunner {
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
    #[allow(dead_code, clippy::too_many_lines)]
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
                    code: ret,
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
            if timing.pkt_count.is_multiple_of(10_000) {
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
            if timing.pkt_count.is_multiple_of(256) {
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
                                super::super::normalize::dump_io_state(
                                    octx.as_mut_ptr(),
                                    path,
                                    "watchdog_stall_interleaved",
                                );
                            }
                        }
                        let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                        return Err(PostProcessError::MuxWriteError {
                            code: 0,
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
}
