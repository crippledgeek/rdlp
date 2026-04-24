//! Mux timing state and timestamp correction for audio encoding.
//!
//! Provides `MuxTimingState` for tracking DTS monotonicity across drain
//! calls, `fix_audio_timestamps` for correcting non-monotonic or
//! zero-duration packets, and diagnostic helpers for mux write failures.

use log::warn;

/// Persistent muxing state for monotonic DTS enforcement across drain calls.
///
/// Created once per encode session and threaded through all pipeline functions
/// so that DTS monotonicity is enforced across call boundaries, not just within
/// a single drain call.
#[derive(Default, Debug)]
pub(crate) struct MuxTimingState {
    /// Last DTS written to the muxer (in output stream timebase).
    pub last_dts: Option<i64>,
    /// Duration of the last packet written (output stream timebase).
    /// Used for duration-aware DTS correction to maintain proper packet spacing.
    pub last_duration: i64,
    /// Precomputed expected packet duration in output stream timebase.
    /// Derived from `encoder_frame_size` rescaled from encoder timebase to
    /// output stream timebase. Used as the primary step size when correcting
    /// DTS regressions, ensuring the Matroska muxer's cluster boundaries
    /// trigger correctly (preventing unbounded cache growth / ENOMEM).
    pub expected_duration: i64,
    /// Total packets written in this encode session.
    pub pkt_count: u64,
    /// Encoder frame size in samples. Used for sample-clock DTS synthesis
    /// (`dur_samples = encoder_frame_size`) and retained for diagnostics.
    pub encoder_frame_size: u32,
    /// `pb->pos` at last watchdog check (mux progress tracking).
    pub last_pos_check: i64,
    /// Consecutive watchdog checks without `pb->pos` advancement.
    /// When this reaches the stall threshold, the encode is aborted
    /// with a retryable `MuxWriteError`.
    pub stall_count: u32,
    /// File size on disk at last watchdog check (secondary progress signal).
    pub last_file_size: i64,
    /// Cumulative samples written. Primary clock for audio-only outputs.
    /// DTS = rescale_q(samples_written, 1/sample_rate, ost_tb).
    pub samples_written: i64,
    /// Audio sample rate (Hz). Set once during init.
    pub sample_rate: u32,
    /// When true, synthesize DTS/PTS from `samples_written`
    /// instead of using encoder-produced timestamps.
    pub use_sample_clock: bool,
}

/// Fix audio packet timestamps for muxer compatibility.
///
/// Ensures:
/// 1. Duration is set (uses `expected_duration` if packet duration is 0/negative)
/// 2. DTS is strictly increasing with proper packet spacing
/// 3. PTS >= DTS (muxer invariant)
///
/// When correcting DTS regressions, steps by `expected_duration` (not 1) to
/// maintain correct packet spacing. This prevents the Matroska muxer from
/// accumulating an unbounded cluster cache due to artificially dense timestamps.
///
/// Returns `(corrected_dts, corrected_pts, duration, updated_last_dts)`.
pub(crate) fn fix_audio_timestamps(
    dts: Option<i64>,
    pts: Option<i64>,
    duration: i64,
    timing: &MuxTimingState,
) -> (Option<i64>, Option<i64>, i64, Option<i64>) {
    // 1. Fix duration first
    let dur = if duration > 0 {
        duration
    } else if timing.expected_duration > 0 {
        timing.expected_duration
    } else {
        timing.last_duration.max(1)
    };

    // 2. Fix DTS
    let Some(d) = dts else {
        return (dts, pts, dur, timing.last_dts);
    };

    if let Some(prev) = timing.last_dts
        && d <= prev
    {
        let step = if timing.expected_duration > 0 {
            timing.expected_duration
        } else {
            dur.max(1)
        };
        let corrected = prev + step;
        let p = pts.map(|p| p.max(corrected)).or(Some(corrected));
        return (Some(corrected), p, dur, Some(corrected));
    }

    // 3. Ensure pts >= dts even without correction (pts must be Some when dts is Some)
    let p = pts.map(|p| p.max(d)).or(Some(d));
    (Some(d), p, dur, Some(d))
}

/// Convert an FFmpeg error code to a human-readable string via `av_strerror`.
pub(crate) fn av_strerror_string(errnum: i32) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is a stack-allocated array with known size; av_strerror
    // writes at most errbuf_size bytes including the NUL terminator.
    unsafe {
        ffmpeg_the_third::ffi::av_strerror(errnum, buf.as_mut_ptr() as *mut _, buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Flush the muxer's interleave queue before writing the trailer.
///
/// Sends a NULL packet to `av_interleaved_write_frame` which signals
/// the muxer to flush any buffered packets in the interleave queue.
/// Only needed for the interleaved write path; the direct `av_write_frame`
/// path bypasses the queue entirely.
pub(crate) fn flush_interleave_queue(octx: &mut ffmpeg_the_third::format::context::Output) {
    // SAFETY: octx is a valid output format context; passing a null packet
    // signals the muxer to flush its internal interleave queue.
    let ret = unsafe {
        ffmpeg_the_third::ffi::av_interleaved_write_frame(octx.as_mut_ptr(), std::ptr::null_mut())
    };
    if ret < 0 {
        let strerr = av_strerror_string(ret);
        warn!("Interleave queue flush returned {ret} ({strerr})");
    }
}

/// Diagnose the I/O context of an output format context at a write failure.
///
/// # Safety
///
/// `octx_ptr` must point to a valid `AVFormatContext`.
pub(crate) unsafe fn diagnose_mux_io(
    octx_ptr: *mut ffmpeg_the_third::ffi::AVFormatContext,
) -> String {
    // SAFETY: caller guarantees octx_ptr is valid; all field reads are from
    // FFmpeg-allocated structs whose layout is defined by the C ABI.
    unsafe {
        let pb = (*octx_ptr).pb;
        if pb.is_null() {
            return "pb=NULL".to_string();
        }
        let error = (*pb).error;
        let write_flag = (*pb).write_flag;
        let has_write_cb = (*pb).write_packet.is_some();
        let pos = (*pb).pos;
        let eof = (*pb).eof_reached;
        format!(
            "pb={{write_flag={write_flag}, error={error}, pos={pos}, eof_reached={eof}, write_cb={}}}",
            if has_write_cb { "present" } else { "NULL" }
        )
    }
}

/// Get current process RSS in KB. Returns 0 if unavailable.
pub(crate) fn get_process_rss_kb() -> u64 {
    #[cfg(target_os = "windows")]
    {
        use std::mem;

        #[repr(C)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }

        unsafe extern "system" {
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                ppsmemcounters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
        }

        unsafe {
            let mut pmc: ProcessMemoryCounters = mem::zeroed();
            pmc.cb = mem::size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
                return (pmc.working_set_size / 1024) as u64;
            }
        }
        0
    }
    #[cfg(target_os = "linux")]
    {
        // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
        #[allow(clippy::disallowed_methods)]
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(0)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        0
    }
}
