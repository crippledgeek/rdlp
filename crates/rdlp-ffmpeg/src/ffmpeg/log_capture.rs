//! Thread-safe FFmpeg log capture via `av_log_set_callback`.
//!
//! Provides an RAII guard that installs a custom FFmpeg log callback to capture
//! log messages (e.g., loudnorm JSON output). On drop, restores the default
//! callback and error-only log level.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;

use crate::error::PostProcessError;

/// Platform-specific `va_list` parameter type for FFmpeg log callbacks.
///
/// On Linux x86_64, `va_list` is `[__va_list_tag; 1]` (a fixed-size array),
/// but FFmpeg 7+ bindgen generates callback function pointer signatures with
/// `*mut __va_list_tag` (pointer to first element). These are ABI-compatible
/// in C (array decays to pointer) but Rust treats them as distinct types.
///
/// On Windows and macOS, `va_list` is already a pointer type that matches
/// the callback signature directly.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
type VaListParam = *mut ffmpeg_the_third::ffi::__va_list_tag;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
type VaListParam = ffmpeg_the_third::ffi::va_list;

/// RAII guard that suppresses FFmpeg's internal C log messages.
///
/// Saves the current log level on creation, sets the requested suppression
/// level, then restores the saved level on drop. Used during audio
/// normalization decode loops to prevent the AAC decoder from spamming
/// stderr with per-packet `get_buffer() failed` lines — we handle those
/// errors at the Rust level.
pub(crate) struct LogSuppressGuard {
    /// Log level that was active before this guard was created; restored on drop.
    saved_level: std::ffi::c_int,
}

impl LogSuppressGuard {
    pub(crate) fn new() -> Self {
        let saved_level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_FATAL);
        }
        Self { saved_level }
    }

    /// Suppress decoder WARNING spam while keeping muxer ERROR messages visible.
    ///
    /// Sets `av_log_set_level` to `AV_LOG_ERROR` instead of `AV_LOG_FATAL`.
    /// Use this during encode loops where muxer errors must remain diagnosable
    /// but decoder `get_buffer() failed` spam should be suppressed.
    pub(crate) fn error_level() -> Self {
        let saved_level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_ERROR);
        }
        Self { saved_level }
    }
}

impl Drop for LogSuppressGuard {
    fn drop(&mut self) {
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(self.saved_level);
        }
    }
}

/// Global buffer for captured FFmpeg log messages.
///
/// Protected by a Mutex. Only populated when a `LogCaptureGuard` is active.
///
/// # Single-session invariant
///
/// At most **one** `LogCaptureGuard` may be active at any time. Nested or
/// concurrent `begin()` calls would silently overwrite the in-progress buffer,
/// corrupting the capture results. The invariant is enforced by:
/// - Callers using `spawn_blocking` serialization (primary guarantee).
/// - A `debug_assert!` in `begin()` that fires in debug builds if violated.
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
        // Single-session invariant: nested capture overwrites the active buffer.
        // This fires in debug builds to catch misuse early.
        debug_assert!(buf.is_none(), "LogCaptureGuard: nested capture detected");
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

        Ok(buf.as_mut().map(std::mem::take).unwrap_or_default())
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
///
/// The `vl` parameter uses `VaListParam` to handle platform differences
/// in `va_list` representation (see type alias documentation above).
unsafe extern "C" fn capture_callback(
    avcl: *mut c_void,
    level: c_int,
    fmt: *const c_char,
    vl: VaListParam,
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
        if let Ok(mut guard) = LOG_BUFFER.lock()
            && let Some(v) = guard.as_mut()
        {
            v.push(msg);
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
        // error_level() sets AV_LOG_ERROR; drop restores the saved level
        unsafe { ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_WARNING) };
        let guard = LogSuppressGuard::error_level();
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_ERROR);
        drop(guard);
        // Must restore the level that was active before construction
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_WARNING);
        // Restore process-wide level to a clean state
        unsafe { ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_ERROR) };
    }

    #[test]
    fn test_log_suppress_guard_fatal_level() {
        // new() sets AV_LOG_FATAL; drop restores the saved level
        unsafe { ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_WARNING) };
        let guard = LogSuppressGuard::new();
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_FATAL);
        drop(guard);
        // Must restore the level that was active before construction
        let level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        assert_eq!(level, ffmpeg_the_third::ffi::AV_LOG_WARNING);
        // Restore process-wide level to a clean state
        unsafe { ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_ERROR) };
    }
}
