//! Thread-safe FFmpeg log capture via `av_log_set_callback`.
//!
//! Provides an RAII guard that installs a custom FFmpeg log callback to capture
//! log messages (e.g., loudnorm JSON output). On drop, restores the default
//! callback and error-only log level.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;

use crate::error::PostProcessError;

/// RAII guard that suppresses FFmpeg's internal C log messages.
///
/// Sets `av_log_set_level` to `AV_LOG_FATAL` on creation, restoring
/// `AV_LOG_ERROR` on drop. Used during audio normalization decode loops
/// to prevent the AAC decoder from spamming stderr with per-packet
/// `get_buffer() failed` lines — we handle those errors at the Rust level.
pub(crate) struct LogSuppressGuard {
    _private: (),
}

impl LogSuppressGuard {
    pub(crate) fn new() -> Self {
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_FATAL);
        }
        Self { _private: () }
    }

    /// Suppress decoder WARNING spam while keeping muxer ERROR messages visible.
    ///
    /// Sets `av_log_set_level` to `AV_LOG_ERROR` instead of `AV_LOG_FATAL`.
    /// Use this during encode loops where muxer errors must remain diagnosable
    /// but decoder `get_buffer() failed` spam should be suppressed.
    pub(crate) fn error_level() -> Self {
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_ERROR);
        }
        Self { _private: () }
    }
}

impl Drop for LogSuppressGuard {
    fn drop(&mut self) {
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_ERROR);
        }
    }
}

/// Global buffer for captured FFmpeg log messages.
///
/// Protected by a Mutex. Only populated when a `LogCaptureGuard` is active.
static LOG_BUFFER: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// RAII guard for FFmpeg log capture.
///
/// While active, FFmpeg log messages at `AV_LOG_INFO` level and below (i.e.,
/// more important) are captured into a global buffer. On drop, the default
/// callback is restored and log level is set back to error-only.
pub(crate) struct LogCaptureGuard {
    _private: (), // prevent external construction
}

impl LogCaptureGuard {
    /// Begin capturing FFmpeg log messages.
    ///
    /// Installs a custom `av_log_set_callback` that buffers messages. Only one
    /// capture session should be active at a time (enforced by `spawn_blocking`
    /// serialization).
    pub(crate) fn begin() -> Result<Self, PostProcessError> {
        // Initialize buffer
        let mut buf = LOG_BUFFER
            .lock()
            .map_err(|e| PostProcessError::LogCaptureFailed {
                message: format!("failed to lock log buffer: {e}"),
            })?;
        *buf = Some(Vec::new());
        drop(buf);

        // Set log level to INFO so loudnorm output is emitted
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_INFO);
            ffmpeg_the_third::ffi::av_log_set_callback(Some(capture_callback));
        }

        Ok(Self { _private: () })
    }

    /// Take all captured log lines, draining the buffer.
    pub(crate) fn take_captured(&self) -> Result<Vec<String>, PostProcessError> {
        let mut buf = LOG_BUFFER
            .lock()
            .map_err(|e| PostProcessError::LogCaptureFailed {
                message: format!("failed to lock log buffer: {e}"),
            })?;

        match buf.as_mut() {
            Some(v) => Ok(std::mem::take(v)),
            None => Ok(Vec::new()),
        }
    }
}

impl Drop for LogCaptureGuard {
    fn drop(&mut self) {
        // Restore default callback and error-only level
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_callback(Some(
                ffmpeg_the_third::ffi::av_log_default_callback,
            ));
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_ERROR);
        }

        // Clear buffer
        if let Ok(mut buf) = LOG_BUFFER.lock() {
            *buf = None;
        }
    }
}

/// Custom FFmpeg log callback that captures messages into the global buffer.
///
/// # Safety
///
/// Called by FFmpeg's internal logging system. All pointer parameters are
/// guaranteed valid by FFmpeg for the duration of the call. Uses
/// `av_log_format_line2` to safely format the `va_list` arguments.
unsafe extern "C" fn capture_callback(
    avcl: *mut c_void,
    level: c_int,
    fmt: *const c_char,
    vl: ffmpeg_the_third::ffi::va_list,
) {
    // Only capture AV_LOG_INFO (32) and more important (lower values)
    if level > ffmpeg_the_third::ffi::AV_LOG_INFO {
        return;
    }

    let mut buf = [0i8; 2048];
    let mut prefix: c_int = 1;
    // SAFETY: All pointers are valid for the duration of this callback invocation.
    // av_log_format_line2 writes at most buf.len()-1 chars + null terminator.
    let len = unsafe {
        ffmpeg_the_third::ffi::av_log_format_line2(
            avcl,
            level,
            fmt,
            vl,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as c_int,
            &mut prefix,
        )
    };

    if len > 0 {
        // SAFETY: av_log_format_line2 null-terminates the buffer
        let msg = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .to_string();
        if let Ok(mut guard) = LOG_BUFFER.lock() {
            if let Some(v) = guard.as_mut() {
                v.push(msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_init_and_clear() {
        // Ensure buffer starts as None
        {
            let buf = LOG_BUFFER.lock().unwrap();
            assert!(buf.is_none() || buf.as_ref().unwrap().is_empty());
        }

        // Begin capture
        let guard = LogCaptureGuard::begin().unwrap();

        // Buffer should be Some(empty vec)
        {
            let buf = LOG_BUFFER.lock().unwrap();
            assert!(buf.is_some());
            assert!(buf.as_ref().unwrap().is_empty());
        }

        // Take should return empty
        let captured = guard.take_captured().unwrap();
        assert!(captured.is_empty());

        // Drop guard
        drop(guard);

        // Buffer should be None again
        {
            let buf = LOG_BUFFER.lock().unwrap();
            assert!(buf.is_none());
        }
    }

    #[test]
    fn test_log_suppress_guard_error_level() {
        // error_level() sets AV_LOG_ERROR; drop restores AV_LOG_ERROR (no-op restore)
        let guard = LogSuppressGuard::error_level();
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_ERROR);
        drop(guard);
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_ERROR);
    }

    #[test]
    fn test_log_suppress_guard_fatal_level() {
        // new() sets AV_LOG_FATAL; drop restores AV_LOG_ERROR
        let guard = LogSuppressGuard::new();
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_FATAL);
        drop(guard);
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_ERROR);
    }
}
