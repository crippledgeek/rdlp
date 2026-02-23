//! Audio encode/mux pipeline: interleaved write path.
//!
//! Provides `receive_and_process_audio`, `drain_filter_to_encoder`, and
//! `drain_encoder_packets` for the standard interleaved-write audio
//! transcoding pipeline used by `extract_audio_transcode_sync`.

use log::{debug, warn};

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
    /// Uses interleaved writes — appropriate for multi-stream output.
    pub(crate) fn receive_and_process_audio(
        decoder: &mut ffmpeg_the_third::decoder::Audio,
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
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
            Self::drain_filter_to_encoder(filter, encoder, octx, ost_index, enc_time_base, timing)?;
        }
        Ok(())
    }

    /// Pull filtered frames from filter graph, encode, and write.
    ///
    /// Uses interleaved writes — appropriate for multi-stream output.
    pub(crate) fn drain_filter_to_encoder(
        filter: &mut ffmpeg_the_third::filter::Graph,
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
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
            Self::drain_encoder_packets(encoder, octx, ost_index, enc_time_base, timing)?;
        }
        Ok(())
    }

    /// Receive encoded packets from encoder and write to output (interleaved).
    ///
    /// Rescales packet timestamps from encoder timebase to output stream
    /// timebase, enforces monotonic DTS, fixes zero-duration packets, then
    /// writes via direct FFI `av_interleaved_write_frame` to capture the raw
    /// return code and I/O diagnostics on failure. Appropriate for
    /// multi-stream output.
    pub(crate) fn drain_encoder_packets(
        encoder: &mut ffmpeg_the_third::encoder::audio::Audio,
        octx: &mut ffmpeg_the_third::format::context::Output,
        ost_index: usize,
        enc_time_base: ffmpeg_the_third::Rational,
        timing: &mut MuxTimingState,
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
                    "First encoded packet: pts={:?}, dts={:?}, size={}, dur={}, enc_tb={}/{}, ost_tb={}/{}, exp_dur={}",
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

            // Capture packet metadata before write (av_interleaved_write_frame unrefs)
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;
            let dur = packet.duration();

            // Direct FFI call to capture raw return code
            // SAFETY: octx and packet are valid; av_interleaved_write_frame takes
            // ownership of the packet buffer and unrefs it on success.
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
                // SAFETY: octx owns a valid format context
                let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                return Err(PostProcessError::MuxWriteError {
                    message: format!("ret={ret} ({strerr}), dur={dur}, {io_diag}"),
                    operation: "av_interleaved_write_frame".into(),
                    stream_index: ost_index,
                    pts,
                    dts,
                    packet_size: size,
                    time_base_num: ost_time_base.numerator(),
                    time_base_den: ost_time_base.denominator(),
                });
            }
            timing.pkt_count += 1;

            // Mux progress watchdog (interleaved path): flush AVIO then check
            // pos and file size, same dual-signal approach as the direct path.
            if timing.pkt_count % 256 == 0 {
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
                if current_pos > timing.last_pos_check {
                    timing.last_pos_check = current_pos;
                    timing.stall_count = 0;
                } else {
                    timing.stall_count += 1;
                    if timing.stall_count >= 3 {
                        let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                        return Err(PostProcessError::MuxWriteError {
                            message: format!(
                                "mux stall detected: pb->pos={current_pos} unchanged for {} packets, {io_diag}",
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
