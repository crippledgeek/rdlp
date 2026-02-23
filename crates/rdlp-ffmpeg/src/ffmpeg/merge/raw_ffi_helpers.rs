//! Raw FFI helper functions for two-way merge operations.
//!
//! Provides low-level packet reading, timestamp rescaling, and DTS
//! comparison helpers used by both the standard and MKV merge paths.

use crate::error::{PostProcessError, Result};

/// Common timebase for DTS comparison: 1 microsecond.
pub(super) const COMPARE_TB: ffmpeg_the_third::ffi::AVRational =
    ffmpeg_the_third::ffi::AVRational {
        num: 1,
        den: 1_000_000,
    };

/// Read the next packet for `target_stream_idx` from a raw FFI input context.
///
/// Skips packets from non-target streams. Returns `true` if a packet was read
/// into `pkt`, `false` on EOF/error.
///
/// # Safety
///
/// `ifmt_ctx` must point to a valid, open `AVFormatContext`.
/// `pkt` must point to a valid (possibly unref'd) `AVPacket`.
pub(super) unsafe fn read_next_raw(
    ifmt_ctx: *mut ffmpeg_the_third::ffi::AVFormatContext,
    target_stream_idx: usize,
    pkt: *mut ffmpeg_the_third::ffi::AVPacket,
) -> bool {
    unsafe {
        loop {
            let ret = ffmpeg_the_third::ffi::av_read_frame(ifmt_ctx, pkt);
            if ret < 0 {
                return false; // EOF or read error
            }
            if (*pkt).stream_index as usize == target_stream_idx {
                return true;
            }
            ffmpeg_the_third::ffi::av_packet_unref(pkt);
        }
    }
}

/// Rescale PTS/DTS/duration from input timebase to output timebase, set the
/// output stream index, and write via `av_interleaved_write_frame`.
///
/// Returns `Err` on write failure instead of silently breaking.
///
/// # Safety
///
/// `pkt` must point to a valid packet with data.
/// `in_stream` must point to the source stream.
/// `ofmt_ctx` must point to a valid, header-written output context.
/// `out_stream_idx` must be a valid stream index in `ofmt_ctx`.
pub(super) unsafe fn rescale_and_write_raw(
    pkt: *mut ffmpeg_the_third::ffi::AVPacket,
    in_stream: *const ffmpeg_the_third::ffi::AVStream,
    ofmt_ctx: *mut ffmpeg_the_third::ffi::AVFormatContext,
    out_stream_idx: i32,
) -> Result<()> {
    unsafe {
        (*pkt).stream_index = out_stream_idx;

        let out_stream = *(*ofmt_ctx).streams.add(out_stream_idx as usize);
        if (*pkt).pts != ffmpeg_the_third::ffi::AV_NOPTS_VALUE {
            (*pkt).pts = ffmpeg_the_third::ffi::av_rescale_q_rnd(
                (*pkt).pts,
                (*in_stream).time_base,
                (*out_stream).time_base,
                ffmpeg_the_third::ffi::AVRounding::AV_ROUND_NEAR_INF,
            );
        }
        if (*pkt).dts != ffmpeg_the_third::ffi::AV_NOPTS_VALUE {
            (*pkt).dts = ffmpeg_the_third::ffi::av_rescale_q_rnd(
                (*pkt).dts,
                (*in_stream).time_base,
                (*out_stream).time_base,
                ffmpeg_the_third::ffi::AVRounding::AV_ROUND_NEAR_INF,
            );
        }
        if (*pkt).duration > 0 {
            (*pkt).duration = ffmpeg_the_third::ffi::av_rescale_q(
                (*pkt).duration,
                (*in_stream).time_base,
                (*out_stream).time_base,
            );
        }
        (*pkt).pos = -1;

        let ret = ffmpeg_the_third::ffi::av_interleaved_write_frame(ofmt_ctx, pkt);
        if ret < 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!("av_interleaved_write_frame failed: error code {ret}"),
            });
        }
        Ok(())
    }
}

/// Rescale a DTS value to the comparison timebase for merge-sort ordering.
///
/// Returns `None` for `AV_NOPTS_VALUE`.
///
/// # Safety
///
/// `stream` must point to a valid `AVStream`.
pub(super) unsafe fn dts_in_us(
    dts: i64,
    stream: *const ffmpeg_the_third::ffi::AVStream,
) -> Option<i64> {
    if dts == ffmpeg_the_third::ffi::AV_NOPTS_VALUE {
        return None;
    }
    unsafe {
        Some(ffmpeg_the_third::ffi::av_rescale_q(
            dts,
            (*stream).time_base,
            COMPARE_TB,
        ))
    }
}
