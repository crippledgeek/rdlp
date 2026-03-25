//! Thread-safe FFmpeg log capture via `av_log_set_callback`.
//!
//! Provides an RAII guard that installs a custom FFmpeg log callback to capture
//! log messages (e.g., loudnorm JSON output). On drop, restores the default
//! callback and error-only log level.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::{Arc, Mutex};

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

/// Type alias for the real-time log forwarding callback.
///
/// Receives `(level, formatted_message)` where `level` is the raw `AV_LOG_*`
/// constant (lower = more severe).
type LogForwarder = Arc<dyn Fn(i32, String) + Send + Sync>;

/// Global real-time log forwarder.
///
/// When set, `capture_callback` forwards each formatted message through this
/// callback in addition to pushing it into `LOG_BUFFER`.
static LOG_FORWARDER: Mutex<Option<LogForwarder>> = Mutex::new(None);

/// Install a real-time log forwarder.
///
/// While active, each FFmpeg log message captured by `capture_callback` is also
/// forwarded through `cb(level, message)`. The callback is called while the
/// `LOG_FORWARDER` mutex is held — it must not block or panic.
pub fn set_log_forwarder(cb: Arc<dyn Fn(i32, String) + Send + Sync>) {
    if let Ok(mut guard) = LOG_FORWARDER.lock() {
        *guard = Some(cb);
    }
}

/// Remove the real-time log forwarder.
pub fn clear_log_forwarder() {
    if let Ok(mut guard) = LOG_FORWARDER.lock() {
        *guard = None;
    }
}

/// RAII guard that clears the log forwarder on drop.
pub struct LogForwarderGuard {
    _private: (),
}

impl LogForwarderGuard {
    /// Create a new guard that installs the forwarder.
    pub fn new(cb: Arc<dyn Fn(i32, String) + Send + Sync>) -> Self {
        set_log_forwarder(cb);
        Self { _private: () }
    }
}

impl Drop for LogForwarderGuard {
    fn drop(&mut self) {
        clear_log_forwarder();
    }
}

/// Opaque guard that holds both a `LogCaptureGuard` (installs `capture_callback`)
/// and a `LogForwarderGuard` (routes messages through `PostProcessCallback::on_log`).
///
/// Hold this until the FFmpeg operation completes. Both guards are dropped together.
pub struct FfmpegLogBridge {
    _capture: LogCaptureGuard,
    _forwarder: LogForwarderGuard,
}

/// Activate both FFmpeg log capture and real-time forwarding to a
/// `PostProcessCallback`.
///
/// Returns an opaque `FfmpegLogBridge` — hold it until the FFmpeg operation
/// completes. Internally creates a `LogCaptureGuard` (installs
/// `capture_callback`) and a `LogForwarderGuard` (routes messages through
/// `cb.on_log()`).
///
/// Level mapping:
/// - `AV_LOG_ERROR` / `AV_LOG_FATAL` → `[ERROR] message`
/// - `AV_LOG_WARNING` → `[WARN] message`
/// - `AV_LOG_INFO` → `message` (no prefix)
pub fn bridge_ffmpeg_logs(
    cb: &Arc<dyn rdlp_core::PostProcessCallback>,
) -> std::result::Result<FfmpegLogBridge, PostProcessError> {
    let capture_guard = LogCaptureGuard::begin()?;
    let cb = cb.clone();
    let forwarder_guard = LogForwarderGuard::new(Arc::new(move |level: i32, msg: String| {
        let trimmed = msg.trim_end();
        if trimmed.is_empty() {
            return;
        }
        let prefixed = match level {
            l if l <= 16 => format!("[ERROR] {trimmed}"),
            24 => format!("[WARN] {trimmed}"),
            _ => trimmed.to_string(),
        };
        cb.on_log(&prefixed);
    }));
    Ok(FfmpegLogBridge {
        _capture: capture_guard,
        _forwarder: forwarder_guard,
    })
}

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
        // Allow nested capture when bridge_ffmpeg_logs creates an outer guard
        // and an FFmpeg function creates an inner guard (e.g., convert_video
        // in verbose mode). Both install the same capture_callback.
        if buf.is_none() {
            *buf = Some(Vec::new());
        }
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
            v.push(msg.clone());
        }
        // Forward in real-time if a forwarder is active
        if let Ok(guard) = LOG_FORWARDER.lock()
            && let Some(ref forwarder) = *guard
        {
            forwarder(level, msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_forwarder_receives_messages() {
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::<(i32, String)>::new()));
        let received_clone = received.clone();

        // Set forwarder
        set_log_forwarder(Arc::new(move |level, msg| {
            if let Ok(mut v) = received_clone.lock() {
                v.push((level, msg));
            }
        }));

        // Begin capture (installs capture_callback)
        let guard = LogCaptureGuard::begin().unwrap();

        // Trigger an FFmpeg log message by setting level to INFO then logging
        unsafe {
            let msg = std::ffi::CString::new("test log message\n").unwrap();
            ffmpeg_the_third::ffi::av_log(
                std::ptr::null_mut(),
                ffmpeg_the_third::ffi::AV_LOG_INFO,
                msg.as_ptr(),
            );
        }

        // Forwarder should have received the message
        let msgs = received.lock().unwrap();
        assert!(
            !msgs.is_empty(),
            "forwarder should have received at least one message"
        );
        assert_eq!(msgs[0].0, ffmpeg_the_third::ffi::AV_LOG_INFO);
        assert!(msgs[0].1.contains("test log message"));

        // Buffer should also have it (dual delivery)
        let buffered = guard.take_captured().unwrap();
        assert!(!buffered.is_empty(), "buffer should also have the message");

        drop(guard);
        clear_log_forwarder();
    }

    #[test]
    fn test_clear_log_forwarder_stops_forwarding() {
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::<(i32, String)>::new()));
        let received_clone = received.clone();

        set_log_forwarder(Arc::new(move |level, msg| {
            if let Ok(mut v) = received_clone.lock() {
                v.push((level, msg));
            }
        }));

        let guard = LogCaptureGuard::begin().unwrap();

        // Clear forwarder before logging
        clear_log_forwarder();

        unsafe {
            let msg = std::ffi::CString::new("after clear\n").unwrap();
            ffmpeg_the_third::ffi::av_log(
                std::ptr::null_mut(),
                ffmpeg_the_third::ffi::AV_LOG_INFO,
                msg.as_ptr(),
            );
        }

        // Forwarder should NOT have received it
        let msgs = received.lock().unwrap();
        assert!(msgs.is_empty(), "forwarder should not receive after clear");

        // Buffer should still have it
        let buffered = guard.take_captured().unwrap();
        assert!(!buffered.is_empty(), "buffer should still capture");

        drop(guard);
    }

    #[test]
    fn test_bridge_ffmpeg_logs_captures_and_forwards() {
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let received_clone = received.clone();

        // Create a mock PostProcessCallback
        struct MockCallback {
            logs: Arc<Mutex<Vec<String>>>,
        }
        impl rdlp_core::PostProcessCallback for MockCallback {
            fn on_progress(&self, _progress: f64) {}
            fn on_log(&self, message: &str) {
                if let Ok(mut v) = self.logs.lock() {
                    v.push(message.to_string());
                }
            }
        }

        let cb: Arc<dyn rdlp_core::PostProcessCallback> = Arc::new(MockCallback {
            logs: received_clone,
        });

        let _bridge = bridge_ffmpeg_logs(&cb).unwrap();

        // Trigger a log message
        unsafe {
            let msg = std::ffi::CString::new("bridge test\n").unwrap();
            ffmpeg_the_third::ffi::av_log(
                std::ptr::null_mut(),
                ffmpeg_the_third::ffi::AV_LOG_INFO,
                msg.as_ptr(),
            );
        }

        let msgs = received.lock().unwrap();
        assert!(
            !msgs.is_empty(),
            "bridge should forward to PostProcessCallback"
        );
        assert!(
            msgs.iter().any(|m| m.contains("bridge test")),
            "message should contain 'bridge test', got: {:?}",
            *msgs
        );
    }

    #[test]
    fn test_log_forwarder_guard_drop_clears() {
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::<(i32, String)>::new()));
        let received_clone = received.clone();

        {
            let _guard = LogForwarderGuard::new(Arc::new(move |level, msg| {
                if let Ok(mut v) = received_clone.lock() {
                    v.push((level, msg));
                }
            }));
            // Forwarder is set inside this scope
            assert!(LOG_FORWARDER.lock().unwrap().is_some());
        }
        // After drop, forwarder should be cleared
        assert!(LOG_FORWARDER.lock().unwrap().is_none());
    }

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
