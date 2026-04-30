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

// ── H2 fix: process-wide spinlock for av_log_set_callback ─────────────────

/// Process-wide serialization flag for `av_log_set_callback`.
///
/// FFmpeg's `av_log_set_callback` + `capture_callback` share two process-global
/// pointers (`LOG_BUFFER`, `LOG_FORWARDER`) and a single global C callback
/// slot. Two concurrent callers to `bridge_ffmpeg_logs` (or two independent
/// `LogCaptureGuard::begin()` calls) would race on all three.
///
/// `FFMPEG_LOG_LOCKED` acts as a spinlock: `LogCaptureGuard::begin()` spins
/// until it can CAS `false → true`, then resets it to `false` on drop.
///
/// Using an `AtomicBool` spinlock instead of `std::sync::Mutex` avoids the
/// `!Send` bound that `MutexGuard<'static, ()>` would impose on
/// `FfmpegLogBridge`, which must cross `.await` points inside async stage
/// `process()` fns.
///
/// Contention is expected to be rare (only two `spawn_blocking` workers
/// racing on a single job boundary), so spinning is acceptable here.
static FFMPEG_LOG_LOCKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// RAII guard that holds the `FFMPEG_LOG_LOCKED` spinlock for the duration of
/// a capture session. Released on drop.
///
/// `Send` because the spinlock state is an `AtomicBool` — no thread-affinity
/// constraint. The guard may be held across an `.await` point (inside
/// `FfmpegLogBridge`) because `FfmpegLogBridge` is dropped after the
/// `spawn_blocking` future resolves, not inside the blocking thread itself.
struct FfmpegLogLockGuard;

impl FfmpegLogLockGuard {
    /// Spin until the spinlock can be acquired (`false → true` CAS).
    fn acquire() -> Self {
        use std::sync::atomic::Ordering;
        // Spin with a hint to yield the CPU. This avoids burning a core
        // while another spawn_blocking worker is tearing down its session.
        while FFMPEG_LOG_LOCKED
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::hint::spin_loop();
        }
        Self
    }
}

impl Drop for FfmpegLogLockGuard {
    fn drop(&mut self) {
        FFMPEG_LOG_LOCKED.store(false, std::sync::atomic::Ordering::Release);
    }
}

// ── end H2 fix ─────────────────────────────────────────────────────────────

/// Global buffer for captured FFmpeg log messages.
///
/// Protected by a Mutex. Only populated when a `LogCaptureGuard` is active.
///
/// # Single-session invariant
///
/// At most **one** `LogCaptureGuard` may be active at any time. Nested or
/// concurrent `begin()` calls would silently overwrite the in-progress buffer,
/// corrupting the capture results. The invariant is enforced by:
/// - `FFMPEG_LOG_LOCKED` spinlock (primary guarantee — serialises all callers).
/// - Callers using `spawn_blocking` (secondary guarantee — typically one at a time).
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
///
/// Holds `FFMPEG_LOG_LOCKED` (spinlock) for its entire lifetime, preventing
/// concurrent `bridge_ffmpeg_logs` / `begin()` calls from racing on the global
/// callback and buffer state (audit finding H2).
pub(crate) struct LogCaptureGuard {
    /// Serializes all concurrent FFmpeg log capture sessions process-wide.
    _lock: FfmpegLogLockGuard,
}

impl LogCaptureGuard {
    /// Begin capturing FFmpeg log messages.
    ///
    /// Acquires `FFMPEG_LOG_LOCKED` (spinlock) and installs a custom
    /// `av_log_set_callback` that buffers messages. Holding the spinlock
    /// prevents concurrent callers from racing on the global callback slot
    /// and `LOG_BUFFER` (audit finding H2).
    ///
    /// The spinlock is released when this guard is dropped.
    pub(crate) fn begin() -> Result<Self, PostProcessError> {
        // Acquire the process-wide serialization spinlock first.
        let lock_guard = FfmpegLogLockGuard::acquire();

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

        Ok(Self { _lock: lock_guard })
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
        // _lock drops here → FFMPEG_LOG_LOCKED reset to false.
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

    /// All tests in this module share process-global state (`LOG_BUFFER`,
    /// `LOG_FORWARDER`, `av_log_set_level`, `av_log_set_callback`). This mutex
    /// serializes them to prevent races when `cargo test` runs in parallel.
    ///
    /// Note: `TEST_MUTEX` must be acquired BEFORE `LogCaptureGuard::begin()`
    /// so it is held for the full duration of each test. `LogCaptureGuard`
    /// acquires `FFMPEG_LOG_LOCKED` internally; the test mutex is an extra
    /// layer that prevents test-level interference (e.g. stale LOG_FORWARDER
    /// from a prior test leaking into the next).
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_log_forwarder_receives_messages() {
        let _lock = TEST_MUTEX.lock().unwrap();
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::<(i32, String)>::new()));
        let received_clone = received.clone();

        // Set forwarder
        set_log_forwarder(Arc::new(move |level, msg| {
            if let Ok(mut v) = received_clone.lock() {
                v.push((level, msg));
            }
        }));

        // Begin capture (installs capture_callback, acquires FFMPEG_LOG_LOCKED)
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
        let _lock = TEST_MUTEX.lock().unwrap();
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
        let _lock = TEST_MUTEX.lock().unwrap();
        use std::sync::Arc;

        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let received_clone = received.clone();

        // Create a mock PostProcessCallback
        struct MockCallback {
            logs: Arc<Mutex<Vec<String>>>,
        }
        impl rdlp_core::PostProcessCallback for MockCallback {
            fn on_progress(&self, _progress: rdlp_types::Progress) {}
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
        let _lock = TEST_MUTEX.lock().unwrap();
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
        let _lock = TEST_MUTEX.lock().unwrap();
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
        let _lock = TEST_MUTEX.lock().unwrap();
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
        let _lock = TEST_MUTEX.lock().unwrap();
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

    // ── H2 regression: FFMPEG_LOG_LOCKED serialises concurrent bridge callers ─
    //
    // Two `spawn_blocking` tasks both call `LogCaptureGuard::begin()`.
    // Without `FFMPEG_LOG_LOCKED` the second caller would overwrite the first's
    // callback and buffer mid-session, producing interleaved or missing lines.
    // With the spinlock, the second task blocks until the first's guard drops.
    //
    // The test asserts:
    // 1. Neither task panics.
    // 2. Each task's captured lines contain only that task's own sentinel
    //    (no cross-contamination from the other task).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_bridge_callers_do_not_race() {
        // Deliberately do NOT hold TEST_MUTEX here — this test specifically
        // exercises the production spinlock (FFMPEG_LOG_LOCKED). The two tasks
        // race to acquire it; the spinlock must serialise them safely.

        let task_a = tokio::task::spawn_blocking(|| {
            let guard = LogCaptureGuard::begin().expect("begin a");
            // Inject a unique sentinel directly into the buffer.
            if let Ok(mut buf) = LOG_BUFFER.lock()
                && let Some(v) = buf.as_mut()
            {
                v.push("task-A-sentinel".to_string());
            }
            let captured = guard.take_captured().expect("take a");
            drop(guard);
            captured
        });

        let task_b = tokio::task::spawn_blocking(|| {
            let guard = LogCaptureGuard::begin().expect("begin b");
            if let Ok(mut buf) = LOG_BUFFER.lock()
                && let Some(v) = buf.as_mut()
            {
                v.push("task-B-sentinel".to_string());
            }
            let captured = guard.take_captured().expect("take b");
            drop(guard);
            captured
        });

        let (a_lines, b_lines) = tokio::join!(task_a, task_b);
        let a_lines = a_lines.expect("task-a join");
        let b_lines = b_lines.expect("task-b join");

        // Cross-contamination would mean one task sees the other's sentinel.
        let a_has_b = a_lines.iter().any(|l| l.contains("task-B-sentinel"));
        let b_has_a = b_lines.iter().any(|l| l.contains("task-A-sentinel"));

        assert!(
            !a_has_b,
            "task-A must not see task-B sentinel (cross-contamination)"
        );
        assert!(
            !b_has_a,
            "task-B must not see task-A sentinel (cross-contamination)"
        );
    }
}
