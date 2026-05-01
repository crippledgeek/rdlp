//! IO diagnostics and validation for the normalization encode pipeline.
//!
//! Post-header-write checks: threading knobs, AVIO state dump,
//! seekability assertion, and file size validation.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `u64` file-size values converted to `i64` for `FFmpeg`
//!   seekability check. The values are filesystem sizes (always positive, well
//!   within `i64::MAX`).
//! - `clippy::redundant_pub_crate`: `pub(crate)` function accessed from sibling
//!   normalize modules via `crate::` path.

#![allow(clippy::cast_possible_wrap, clippy::redundant_pub_crate)]

use std::ffi::CStr;
use std::path::Path;

use log::info;

use crate::error::{PostProcessError, Result};

use super::super::ffi_helpers::codec_threading_info;

/// Validate mux header state after `write_header`.
///
/// Logs threading knobs, dumps AVIO state, checks seekability, flushes AVIO,
/// and validates file size. Consolidates the post-header diagnostics into
/// one call.
///
/// # Safety
///
/// All raw pointer dereferences require valid, header-written contexts.
pub(super) unsafe fn validate_mux_header_state(
    octx: &mut ffmpeg_the_third::format::context::Output,
    audio_decoder: &ffmpeg_the_third::decoder::Audio,
    audio_encoder: &ffmpeg_the_third::encoder::Audio,
    output: &Path,
    label: &str,
) -> Result<()> {
    // E1: Log threading knobs, mux flags, and AVIO state
    unsafe {
        let (dec_tc, dec_att) = codec_threading_info(audio_decoder.as_ptr());
        let (enc_tc, enc_att) = codec_threading_info(audio_encoder.as_ptr());
        let mux_flags = (*octx.as_mut_ptr()).flags;
        let pb_buf_size = {
            let pb = (*octx.as_mut_ptr()).pb;
            if pb.is_null() { 0 } else { (*pb).buffer_size }
        };
        info!(
            "[{label}] decoder: thread_count={dec_tc}, \
             active_thread_type={dec_att}; \
             encoder: thread_count={enc_tc}, \
             active_thread_type={enc_att}; \
             mux_flags=0x{mux_flags:x} (flush_packets={}); \
             pb_buffer_size={pb_buf_size}",
            (mux_flags & ffmpeg_the_third::ffi::AVFMT_FLAG_FLUSH_PACKETS) != 0,
        );
    }

    unsafe {
        dump_io_state(octx.as_mut_ptr(), output, label);
    }

    // Fail-fast: reject CUSTOM_IO
    {
        let flags = unsafe { (*octx.as_mut_ptr()).flags };
        if flags & ffmpeg_the_third::ffi::AVFMT_FLAG_CUSTOM_IO != 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!(
                    "[{label}] AVFMT_FLAG_CUSTOM_IO is set \
                     — expected file-based IO"
                ),
            });
        }
    }

    // Fail-fast: assert seekable IO
    unsafe {
        assert_seekable_io(octx.as_mut_ptr(), label)?;
    }

    // Flush AVIO after header write
    unsafe {
        let pb = (*octx.as_mut_ptr()).pb;
        if !pb.is_null() {
            ffmpeg_the_third::ffi::avio_flush(pb);
        }
    }
    // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
    #[allow(clippy::disallowed_methods)]
    let file_size = std::fs::metadata(output).map_or(0, |m| m.len());
    if file_size == 0 {
        return Err(PostProcessError::FFmpegLibraryError {
            message: format!(
                "[{label}] output file is 0 bytes after header write \
                 + avio_flush — IO sink is broken"
            ),
        });
    }
    info!("[{label}] header written: {file_size} bytes on disk");

    Ok(())
}

/// Comprehensive IO state dump for diagnostics.
///
/// Logs muxer name, format context URL and flags, AVIO buffer state,
/// and the actual file size on disk.
///
/// # Safety
///
/// `octx_ptr` must point to a valid, header-written `AVFormatContext`.
pub unsafe fn dump_io_state(
    octx_ptr: *mut ffmpeg_the_third::ffi::AVFormatContext,
    rust_output_path: &Path,
    label: &str,
) {
    unsafe {
        let oformat = (*octx_ptr).oformat;
        let muxer_name = if !oformat.is_null() && !(*oformat).name.is_null() {
            CStr::from_ptr((*oformat).name)
                .to_string_lossy()
                .into_owned()
        } else {
            "unknown".to_string()
        };

        let url = if (*octx_ptr).url.is_null() {
            "NULL".to_string()
        } else {
            CStr::from_ptr((*octx_ptr).url)
                .to_string_lossy()
                .into_owned()
        };

        let flags = (*octx_ptr).flags;
        let custom_io = (flags & ffmpeg_the_third::ffi::AVFMT_FLAG_CUSTOM_IO) != 0;

        let pb = (*octx_ptr).pb;
        if pb.is_null() {
            info!(
                "[{label}] IO dump: muxer={muxer_name}, url={url}, flags=0x{flags:x}, \
                 custom_io={custom_io}, pb=NULL"
            );
            return;
        }

        let seekable = (*pb).seekable;
        let buffer_size = (*pb).buffer_size;
        let pos = (*pb).pos;
        let error = (*pb).error;
        let direct = (*pb).direct;
        let write_flag = (*pb).write_flag;
        let has_write_cb = (*pb).write_packet.is_some();

        // Safe: sync FFmpeg wrapper — all callers invoke via spawn_blocking from async boundaries (see rdlp-ffmpeg/src/ffmpeg/mod.rs spawn_blocking helper).
        #[allow(clippy::disallowed_methods)]
        let file_size = std::fs::metadata(rust_output_path).map_or(-1, |m| m.len() as i64);

        info!(
            "[{label}] IO dump: muxer={muxer_name}, url={url}, flags=0x{flags:x}, \
             custom_io={custom_io}, pb={{seekable={seekable}, buffer_size={buffer_size}, \
             pos={pos}, error={error}, direct={direct}, write_flag={write_flag}, \
             write_cb={}}}, rust_path={}, file_size={file_size}",
            if has_write_cb { "present" } else { "NULL" },
            rust_output_path.display(),
        );
    }
}

/// Assert that the output format context has a seekable IO context.
///
/// Matroska requires seeking for Cues/SeekHead; a non-seekable output
/// silently stalls the muxer.
///
/// # Safety
///
/// `octx_ptr` must point to a valid, header-written `AVFormatContext`.
unsafe fn assert_seekable_io(
    octx_ptr: *mut ffmpeg_the_third::ffi::AVFormatContext,
    label: &str,
) -> Result<()> {
    unsafe {
        let pb = (*octx_ptr).pb;
        if pb.is_null() {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!("[{label}] output IO context (pb) is NULL — no output possible"),
            });
        }
        if (*pb).seekable == 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!(
                    "[{label}] output IO context is not seekable (pb->seekable=0) — \
                     Matroska muxer requires seeking for Cues/SeekHead"
                ),
            });
        }
        let oformat = (*octx_ptr).oformat;
        if !oformat.is_null() && ((*oformat).flags & ffmpeg_the_third::ffi::AVFMT_NOFILE) != 0 {
            return Err(PostProcessError::FFmpegLibraryError {
                message: format!(
                    "[{label}] output format has AVFMT_NOFILE flag — expected file-based IO"
                ),
            });
        }
        Ok(())
    }
}
