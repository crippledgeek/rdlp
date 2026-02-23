//! Audio encode/mux pipeline: direct (non-interleaved) write path.
//!
//! Provides `receive_and_process_audio_direct`, `drain_filter_to_encoder_direct`,
//! and `drain_encoder_packets_direct` for the direct-write audio pipeline
//! used by audio normalization. Primary path for audio-only normalization
//! to avoid interleave queue ENOMEM.

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
    /// Uses direct (non-interleaved) writes — appropriate for single-stream
    /// audio-only output where the muxer's interleaving buffer is unnecessary.
    /// Primary path for audio-only normalization to avoid interleave queue ENOMEM.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn receive_and_process_audio_direct(
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
            Self::drain_filter_to_encoder_direct(
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

    /// Pull filtered frames from filter graph, encode, and write.
    ///
    /// Uses direct (non-interleaved) writes — appropriate for single-stream
    /// audio-only output. Primary path for normalization to avoid ENOMEM.
    pub(crate) fn drain_filter_to_encoder_direct(
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
            Self::drain_encoder_packets_direct(
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

    /// Receive encoded packets from encoder and write to output (direct).
    ///
    /// Rescales packet timestamps from encoder timebase to output stream
    /// timebase, enforces monotonic DTS, fixes zero-duration packets, then
    /// writes via direct FFI `av_write_frame` (non-interleaved) to bypass
    /// the muxer's interleave queue. This is appropriate for single-stream
    /// audio-only output where interleaving is unnecessary — the queue would
    /// only buffer packets without flushing, risking ENOMEM on long files.
    /// The rescaling is critical for Matroska muxer which changes the stream
    /// timebase after `write_header()` (e.g. from `1/48000` to `1/1000`).
    ///
    /// `output_path` enables the watchdog to check actual file size on disk
    /// as a secondary progress signal alongside `pb->pos`.
    ///
    /// Primary path for audio-only normalization to avoid interleave queue ENOMEM.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn drain_encoder_packets_direct(
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
            packet.set_position(-1);

            if timing.use_sample_clock {
                // Sample-clock: synthesize timestamps from cumulative sample count
                let dur_samples = i64::from(timing.encoder_frame_size).max(1);
                let sr_tb = ffmpeg_the_third::ffi::AVRational {
                    num: 1,
                    den: timing.sample_rate as i32,
                };
                let ost_tb = ffmpeg_the_third::ffi::AVRational {
                    num: ost_time_base.numerator(),
                    den: ost_time_base.denominator(),
                };
                let dts = unsafe {
                    ffmpeg_the_third::ffi::av_rescale_q(timing.samples_written, sr_tb, ost_tb)
                };
                let dur_tb =
                    unsafe { ffmpeg_the_third::ffi::av_rescale_q(dur_samples, sr_tb, ost_tb) }
                        .max(1);

                packet.set_dts(Some(dts));
                packet.set_pts(Some(dts));
                packet.set_duration(dur_tb);
                timing.samples_written += dur_samples;

                // Safety net: run through fix_audio_timestamps (should be no-op)
                let (new_dts, new_pts, new_dur, updated) =
                    fix_audio_timestamps(packet.dts(), packet.pts(), packet.duration(), timing);
                packet.set_dts(new_dts);
                packet.set_pts(new_pts);
                packet.set_duration(new_dur);
                timing.last_dts = updated;
                timing.last_duration = new_dur;
            } else {
                // Legacy path: use encoder timestamps + rescale
                packet.rescale_ts(enc_time_base, ost_time_base);

                if timing.pkt_count == 0 {
                    debug!(
                        "First encoded packet (direct): pts={:?}, dts={:?}, size={}, dur={}, enc_tb={}/{}, ost_tb={}/{}, exp_dur={}",
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
            }

            // Diagnostic logging for first 5 packets (sample-clock path)
            if timing.pkt_count < 5 {
                let pb_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };
                let file_sz = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                info!(
                    "[audio_only_mux] pkt#{}: dts={:?}, pts={:?}, dur={}, \
                     samples_written={}, sr={}, ost_tb={}/{}, pb_pos={}, file={}B",
                    timing.pkt_count,
                    packet.dts(),
                    packet.pts(),
                    packet.duration(),
                    timing.samples_written,
                    timing.sample_rate,
                    ost_time_base.numerator(),
                    ost_time_base.denominator(),
                    pb_pos,
                    file_sz,
                );
            }

            // Capture packet metadata before write (av_write_frame may unref)
            let pts = packet.pts();
            let dts = packet.dts();
            let size = packet.size() as i32;
            let dur = packet.duration();

            // Direct FFI call using av_write_frame (non-interleaved) to bypass
            // the muxer's interleave queue. For single-stream audio output,
            // interleaving is unnecessary and the queue can grow unbounded.
            // SAFETY: octx and packet are valid; av_write_frame writes the
            // packet directly without buffering in an interleave queue.
            let ret = unsafe {
                ffmpeg_the_third::ffi::av_write_frame(octx.as_mut_ptr(), packet.as_ptr() as *mut _)
            };
            // Unref immediately: frees encoded data before next receive_packet.
            // av_write_frame unrefs on success in FFmpeg 8.0, but NOT on failure.
            // Explicit unref is idempotent and matches merge.rs / remux.rs /
            // thumbnail.rs patterns.
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
                    operation: "av_write_frame".into(),
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
            if timing.pkt_count % 10_000 == 0 {
                let pb_pos = unsafe {
                    let pb = (*octx.as_mut_ptr()).pb;
                    if !pb.is_null() { (*pb).pos } else { 0 }
                };
                let file_sz = output_path
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .unwrap_or(0);
                let rss_kb = get_process_rss_kb();
                info!(
                    "[mux progress] pkt={}, dts={:?}, dur={}, samples={}, pos={}, file={}KB, rss={}KB",
                    timing.pkt_count,
                    dts,
                    dur,
                    timing.samples_written,
                    pb_pos,
                    file_sz / 1024,
                    rss_kb,
                );
            }

            // Mux progress watchdog: every 256 packets, flush AVIO so pb->pos
            // reflects actual writes, then check both pos and file size.
            // If neither advances for 3 consecutive checks (768 packets), the
            // muxer is stuck. Abort with a retryable error so salvage/CLI
            // fallback can recover.
            if timing.pkt_count % 256 == 0 {
                // Flush AVIO buffer so pb->pos reflects actual write progress
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
                        // Full IO dump before returning error
                        if let Some(path) = output_path {
                            unsafe {
                                super::super::normalize::dump_io_state(
                                    octx.as_mut_ptr(),
                                    path,
                                    "watchdog_stall",
                                );
                            }
                        }
                        let io_diag = unsafe { diagnose_mux_io(octx.as_mut_ptr()) };
                        return Err(PostProcessError::MuxWriteError {
                            message: format!(
                                "mux stall detected: pb->pos={current_pos}, file_size={file_size} unchanged for {} packets, {io_diag}",
                                timing.stall_count * 256
                            ),
                            operation: "av_write_frame (watchdog)".into(),
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
