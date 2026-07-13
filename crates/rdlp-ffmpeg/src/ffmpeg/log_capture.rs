//! Thread-safe `FFmpeg` log capture via `av_log_set_callback`.
//!
//! Provides an RAII guard that installs a custom `FFmpeg` log callback to capture
//! log messages (e.g., loudnorm JSON output). On drop, restores the default
//! callback and error-only log level.
//!
//! # Lint allowances
//!
//! - `clippy::cast_*`: `usize`→`i32` cast for `av_log_format_line2` buffer length
//!   argument is safe: the buffer is 2048 bytes, well within `i32::MAX`.
//! - `clippy::borrow_as_ptr`: `buf.as_mut_ptr() as *mut c_char` required by the
//!   `av_log_format_line2` C signature.
//! - `clippy::redundant_pub_crate`: `pub(crate)` items in this private module are
//!   accessed from sibling modules via `crate::ffmpeg::log_capture::*`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::borrow_as_ptr,
    clippy::redundant_pub_crate
)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::{Arc, Mutex};

use crate::error::PostProcessError;

/// Platform-specific `va_list` parameter type for `FFmpeg` log callbacks.
///
/// On Linux `x86_64`, `va_list` is `[__va_list_tag; 1]` (a fixed-size array),
/// but `FFmpeg` 7+ bindgen generates callback function pointer signatures with
/// `*mut __va_list_tag` (pointer to first element). These are ABI-compatible
/// in C (array decays to pointer) but Rust treats them as distinct types.
///
/// On Windows and macOS, `va_list` is already a pointer type that matches
/// the callback signature directly.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
type VaListParam = *mut ffmpeg_the_third::ffi::__va_list_tag;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
type VaListParam = ffmpeg_the_third::ffi::va_list;

/// RAII guard that suppresses `FFmpeg`'s internal C log messages.
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

// ── Install-once global callback + thread-local capture routing ───────────
//
// `av_log_set_callback` is a single PROCESS-GLOBAL slot, and the callback is
// invoked from whichever thread emits the log (including FFmpeg's own worker
// threads). The previous design installed/restored the callback per operation
// and serialized that with a process-wide spinlock held for the whole
// operation. That spinlock self-deadlocked: an outer capture session (e.g. the
// recode log bridge) held it across `convert_video`, whose nested salvage
// integrity check called `begin()` again and span forever (see the
// `bugfix/ffmpeg-log-capture-deadlock` investigation).
//
// New design: install `capture_callback` exactly ONCE (idempotent), never
// restore it. Per-operation CAPTURE buffers live in a THREAD-LOCAL stack —
// each `LogCaptureGuard::begin()` pushes a buffer (RAII pop on drop), so nested
// captures on the same thread compose without any lock and concurrent captures
// on different threads are isolated. Real-time FORWARDING (recode → UI) stays
// in a process-global slot (`LOG_FORWARDER`) because the forwarder is created
// on the async task thread while the FFmpeg work — and thus the log emission —
// happens on a `spawn_blocking` worker thread; a thread-local would be set on
// the wrong thread and capture nothing. When neither a capture buffer nor a
// forwarder is active, the callback falls through to `av_log_default_callback`
// (normal stderr behaviour).

/// Ensures `capture_callback` is installed as the global `FFmpeg` log callback
/// exactly once for the lifetime of the process.
static CALLBACK_INSTALLED: std::sync::Once = std::sync::Once::new();

fn ensure_callback_installed() {
    CALLBACK_INSTALLED.call_once(|| unsafe {
        ffmpeg_the_third::ffi::av_log_set_callback(Some(capture_callback));
    });
}

thread_local! {
    /// Stack of active per-operation capture buffers on THIS thread.
    ///
    /// `LogCaptureGuard::begin()` pushes; its `Drop` pops (LIFO/RAII). The
    /// innermost (top) buffer receives captured lines. Empty on threads with no
    /// active capture — the callback then forwards/falls-through instead.
    static CAPTURE_STACK: std::cell::RefCell<Vec<Arc<Mutex<Vec<String>>>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Type alias for the real-time log forwarding callback.
///
/// Receives `(level, formatted_message)` where `level` is the raw `AV_LOG_*`
/// constant (lower = more severe).
type LogForwarder = Arc<dyn Fn(i32, String) + Send + Sync>;

/// Global real-time log forwarder.
///
/// When set, `capture_callback` forwards each formatted message through this
/// callback in addition to the active thread-local capture buffer (if any).
static LOG_FORWARDER: Mutex<Option<LogForwarder>> = Mutex::new(None);

/// Install a real-time log forwarder.
///
/// While active, each `FFmpeg` log message captured by `capture_callback` is also
/// forwarded through `forwarder(level, message)`. The callback is called while the
/// `LOG_FORWARDER` mutex is held — it must not block or panic.
pub fn set_log_forwarder(forwarder: Arc<dyn Fn(i32, String) + Send + Sync>) {
    if let Ok(mut guard) = LOG_FORWARDER.lock() {
        *guard = Some(forwarder);
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
    pub fn new(forwarder: Arc<dyn Fn(i32, String) + Send + Sync>) -> Self {
        set_log_forwarder(forwarder);
        Self { _private: () }
    }
}

impl Drop for LogForwarderGuard {
    fn drop(&mut self) {
        clear_log_forwarder();
    }
}

/// Opaque guard that holds a per-thread `LogCaptureGuard` and a global
/// `LogForwarderGuard` (routes messages through `PostProcessCallback::on_log`).
///
/// Hold this until the `FFmpeg` operation completes. Both guards are dropped
/// together. NOTE: the bridge is typically created on the async task thread,
/// while the `FFmpeg` work (and its log emission) runs on a `spawn_blocking`
/// worker thread — so the load-bearing part is the *global* forwarder; the
/// `LogCaptureGuard`'s thread-local buffer (on the async thread) captures
/// nothing and is harmless.
pub struct FfmpegLogBridge {
    _capture: LogCaptureGuard,
    _forwarder: LogForwarderGuard,
}

/// Known-cosmetic `FFmpeg` log lines that are emitted at ERROR level but are
/// actually swallowed by the decoder and have no effect on output. The webp
/// decoder logs this when an embedded EXIF chunk has a bad TIFF header, then
/// skips the chunk and decodes the image normally (libavcodec/webp.c) — which
/// is exactly why `webp.c` itself demotes it to a warning internally. We
/// downgrade the line from `[ERROR]` to `[WARN]`: the message is NEVER dropped
/// or hidden (every `FFmpeg` line stays visible), only its severity is lowered
/// so it no longer reads as a failure.
fn is_cosmetic_decoder_noise(msg: &str) -> bool {
    // Require the `[webp @` module prefix as well as the text, so a genuine
    // error from another module (e.g. a real `[tiff @ ...]` demuxer failure)
    // carrying the same phrase is NOT demoted.
    // NOTE: add further known-cosmetic patterns here as additional `||` clauses.
    msg.contains("[webp @") && msg.contains("invalid TIFF header in Exif data")
}

/// Map an `FFmpeg` `(level, message)` to the user-facing prefixed log line.
///
/// `AV_LOG_ERROR`/`FATAL` (and more severe) → `[ERROR]`; `AV_LOG_WARNING` →
/// `[WARN]`; everything else (e.g. `AV_LOG_INFO`) is emitted without a prefix.
/// Known-cosmetic decoder noise is downgraded from `[ERROR]` to `[WARN]` — it
/// stays visible, only its severity is lowered. No line is ever filtered out.
fn format_ffmpeg_log_line(level: i32, trimmed: &str) -> String {
    use ffmpeg_the_third::ffi::{AV_LOG_ERROR, AV_LOG_WARNING};
    if is_cosmetic_decoder_noise(trimmed) {
        return format!("[WARN] {trimmed}"); // downgrade ERROR -> WARN, never drop
    }
    match level {
        l if l <= AV_LOG_ERROR => format!("[ERROR] {trimmed}"),
        AV_LOG_WARNING => format!("[WARN] {trimmed}"),
        _ => trimmed.to_string(),
    }
}

/// Activate `FFmpeg` real-time log forwarding to a `PostProcessCallback`.
///
/// Returns an opaque `FfmpegLogBridge` — hold it until the `FFmpeg` operation
/// completes. The global `capture_callback` is install-once (see
/// `ensure_callback_installed`); this only registers the per-thread capture
/// buffer and the global forwarder that routes messages through `cb.on_log()`.
///
/// Level mapping:
/// - `AV_LOG_ERROR` / `AV_LOG_FATAL` → `[ERROR] message`
/// - `AV_LOG_WARNING` → `[WARN] message`
/// - `AV_LOG_INFO` → `message` (no prefix)
///
/// # Errors
///
/// Currently never returns an error (`LogCaptureGuard::begin` is infallible);
/// the `Result` is kept for API stability and a future fallible path.
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
        let prefixed = format_ffmpeg_log_line(level, trimmed);
        cb.on_log(&prefixed);
    }));
    Ok(FfmpegLogBridge {
        _capture: capture_guard,
        _forwarder: forwarder_guard,
    })
}

/// RAII guard for `FFmpeg` log capture.
///
/// While active, `FFmpeg` log messages at `AV_LOG_INFO` level and below are
/// captured into this guard's own buffer (registered on the current thread's
/// [`CAPTURE_STACK`]). Nested guards on the same thread stack; the innermost
/// receives messages. On drop, the guard removes its buffer from the stack and
/// restores the log level it raised. The global callback is install-once and is
/// never uninstalled — when no guard is active it falls through to the default.
///
/// No process-wide lock is held, so nested or cross-thread captures cannot
/// deadlock (the previous spinlock self-deadlocked under recode→salvage).
pub(crate) struct LogCaptureGuard {
    /// This session's capture buffer, also referenced from `CAPTURE_STACK`.
    buffer: Arc<Mutex<Vec<String>>>,
    /// Log level active before this guard raised it to INFO; restored on drop.
    saved_level: std::ffi::c_int,
}

impl LogCaptureGuard {
    /// Begin capturing `FFmpeg` log messages on the current thread.
    ///
    /// Installs the global capture callback once (idempotent), pushes a fresh
    /// per-session buffer onto this thread's [`CAPTURE_STACK`], and raises the
    /// log level to INFO so e.g. loudnorm output is emitted. Nesting is safe:
    /// a second `begin()` on the same thread pushes another buffer; the
    /// innermost captures. There is no lock to contend on.
    ///
    /// Returns `Result` for API stability with the call sites (`begin()?` /
    /// `.ok()`) and so the function can regain a fallible path without a
    /// signature change; the current body is infallible.
    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn begin() -> Result<Self, PostProcessError> {
        ensure_callback_installed();

        let buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        CAPTURE_STACK.with(|stack| stack.borrow_mut().push(Arc::clone(&buffer)));

        // Raise to INFO so INFO-level output (loudnorm JSON, etc.) is delivered.
        let saved_level = unsafe { ffmpeg_the_third::ffi::av_log_get_level() };
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(ffmpeg_the_third::ffi::AV_LOG_INFO);
        }

        Ok(Self {
            buffer,
            saved_level,
        })
    }

    /// Take all captured log lines, draining this session's buffer.
    pub(crate) fn take_captured(&self) -> Result<Vec<String>, PostProcessError> {
        let mut buf = self
            .buffer
            .lock()
            .map_err(|e| PostProcessError::LogCaptureFailed {
                message: format!("failed to lock log buffer: {e}"),
            })?;
        Ok(std::mem::take(&mut *buf))
    }
}

impl Drop for LogCaptureGuard {
    fn drop(&mut self) {
        // Remove THIS session's buffer from the thread-local stack (by identity,
        // robust to out-of-order guard drops), then restore the log level.
        CAPTURE_STACK.with(|stack| {
            let mut s = stack.borrow_mut();
            if let Some(pos) = s.iter().rposition(|b| Arc::ptr_eq(b, &self.buffer)) {
                s.remove(pos);
            }
        });
        unsafe {
            ffmpeg_the_third::ffi::av_log_set_level(self.saved_level);
        }
        // The global callback stays installed (install-once); it falls through
        // to av_log_default_callback whenever no capture/forwarder is active.
    }
}

/// The process-global, install-once `FFmpeg` log callback.
///
/// Routing (decided BEFORE the `va_list` is consumed, since `av_log_format_line2`
/// consumes it and it must not be reused):
/// - If no per-thread capture buffer is active AND no global forwarder is set,
///   delegate to `av_log_default_callback` (normal stderr) with the untouched
///   `va_list` and return.
/// - Otherwise format the line once and route it to this thread's innermost
///   capture buffer (if any) and the global forwarder (if any).
///
/// # Safety
///
/// Called by `FFmpeg`'s internal logging system, possibly from `FFmpeg` worker
/// threads. All pointer parameters are valid for the duration of the call.
/// `av_log_format_line2` formats the `va_list` safely; the `va_list` is consumed
/// at most once across this call.
unsafe extern "C" fn capture_callback(
    avcl: *mut c_void,
    level: c_int,
    fmt: *const c_char,
    vl: VaListParam,
) {
    // Capture/forward only AV_LOG_INFO (32) and more important (lower values).
    let level_ok = level <= ffmpeg_the_third::ffi::AV_LOG_INFO;
    let has_capture = level_ok && CAPTURE_STACK.with(|stack| stack.borrow().last().is_some());
    // Snapshot the forwarder (clone the Arc) so we neither hold the LOG_FORWARDER
    // lock during formatting/dispatch nor invoke the forwarder under the lock.
    let forwarder: Option<LogForwarder> = if level_ok {
        LOG_FORWARDER.lock().ok().and_then(|g| g.clone())
    } else {
        None
    };

    // No active sink for this message → preserve default stderr behaviour.
    // The `va_list` has NOT been consumed yet, so it is safe to pass through.
    if !has_capture && forwarder.is_none() {
        unsafe {
            ffmpeg_the_third::ffi::av_log_default_callback(avcl, level, fmt, vl);
        }
        return;
    }

    let mut buf = [0i8; 2048];
    let mut prefix: c_int = 1;
    // SAFETY: All pointers are valid for the duration of this callback invocation.
    // av_log_format_line2 writes at most buf.len()-1 chars + null terminator and
    // consumes `vl` exactly once (we do not reuse it afterwards).
    let len = unsafe {
        ffmpeg_the_third::ffi::av_log_format_line2(
            avcl,
            level,
            fmt,
            vl,
            buf.as_mut_ptr().cast::<c_char>(),
            buf.len() as c_int,
            &raw mut prefix,
        )
    };
    if len <= 0 {
        return;
    }

    // SAFETY: av_log_format_line2 null-terminates the buffer.
    let msg = unsafe { CStr::from_ptr(buf.as_ptr().cast::<c_char>()) }
        .to_string_lossy()
        .to_string();

    let push_to_capture = |line: String| {
        CAPTURE_STACK.with(|stack| {
            if let Some(top) = stack.borrow().last()
                && let Ok(mut v) = top.lock()
            {
                v.push(line);
            }
        });
    };

    // Clone only when BOTH sinks are live (the message is consumed twice). On
    // the common capture-only path — the integrity scan and loudnorm capture
    // run with no forwarder — move `msg` into the buffer instead of cloning,
    // saving one allocation per captured log line.
    match forwarder {
        Some(forwarder) => {
            if has_capture {
                push_to_capture(msg.clone());
            }
            forwarder(level, msg);
        }
        None => {
            if has_capture {
                push_to_capture(msg);
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,        // msgs[0] is guarded by assert!(!msgs.is_empty())
    clippy::items_after_statements,  // use Arc inside test fn body is intentional for clarity
    clippy::significant_drop_tightening, // _lock must stay alive for full test scope
)]
mod tests {
    use super::*;

    /// All tests in this module share process-global state (`LOG_FORWARDER`,
    /// `av_log_set_level`) plus the per-thread `CAPTURE_STACK`. This mutex
    /// serializes them to prevent races when `cargo test` runs in parallel.
    ///
    /// Note: `TEST_MUTEX` must be acquired BEFORE `LogCaptureGuard::begin()`
    /// so it is held for the full duration of each test. The test mutex
    /// prevents test-level interference (e.g. stale `LOG_FORWARDER`
    /// from a prior test leaking into the next).
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn cosmetic_webp_exif_line_is_downgraded_to_warn_not_dropped() {
        // webp.c logs this at AV_LOG_ERROR then swallows it and decodes the
        // image anyway. It must NOT surface as [ERROR], but it must STILL be
        // emitted (never filtered) — downgraded to [WARN], carrying the text.
        let out = format_ffmpeg_log_line(
            ffmpeg_the_third::ffi::AV_LOG_ERROR,
            "[webp @ 0x1] invalid TIFF header in Exif data",
        );
        assert!(
            out.starts_with("[WARN]"),
            "cosmetic webp Exif line should be downgraded to [WARN], got: {out}"
        );
        assert!(
            out.contains("invalid TIFF header in Exif data"),
            "the line must still be emitted in full, not dropped, got: {out}"
        );
    }

    #[test]
    fn genuine_error_still_prefixed() {
        let out = format_ffmpeg_log_line(
            ffmpeg_the_third::ffi::AV_LOG_ERROR,
            "[matroska @ 0x1] some real failure",
        );
        assert!(
            out.starts_with("[ERROR]"),
            "genuine error should be prefixed [ERROR], got: {out}"
        );
    }

    #[test]
    fn non_webp_tiff_error_is_not_downgraded() {
        // A real error from another module carrying the same phrase must keep
        // its [ERROR] severity — the cosmetic downgrade is webp-only.
        let out = format_ffmpeg_log_line(
            ffmpeg_the_third::ffi::AV_LOG_ERROR,
            "[tiff @ 0x1] invalid TIFF header in Exif data",
        );
        assert!(
            out.starts_with("[ERROR]"),
            "non-webp TIFF error must stay [ERROR], got: {out}"
        );
    }

    #[test]
    fn warning_level_maps_to_warn() {
        let out = format_ffmpeg_log_line(ffmpeg_the_third::ffi::AV_LOG_WARNING, "x");
        assert!(
            out.starts_with("[WARN]"),
            "warning level should map to [WARN], got: {out}"
        );
    }

    #[test]
    fn test_log_forwarder_receives_messages() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let received = Arc::new(Mutex::new(Vec::<(i32, String)>::new()));
        let received_clone = received.clone();

        // Set forwarder
        set_log_forwarder(Arc::new(move |level, msg| {
            if let Ok(mut v) = received_clone.lock() {
                v.push((level, msg));
            }
        }));

        // Begin capture (pushes a thread-local capture buffer)
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
        let received = Arc::new(Mutex::new(Vec::<(i32, String)>::new()));
        let received_clone = received;

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
    fn test_capture_stack_push_and_pop() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let depth = || CAPTURE_STACK.with(|s| s.borrow().len());
        let before = depth();

        // Begin capture → pushes one buffer onto this thread's stack.
        let guard = LogCaptureGuard::begin().unwrap();
        assert_eq!(depth(), before + 1, "begin() pushes a capture buffer");

        // Fresh buffer drains empty.
        assert!(guard.take_captured().unwrap().is_empty());

        // Drop → pops this session's buffer back off the stack.
        drop(guard);
        assert_eq!(depth(), before, "drop pops the capture buffer");
    }

    /// Regression: nested `begin()` on the SAME thread must not deadlock and
    /// each level must capture into its own buffer. Under the previous
    /// `FFMPEG_LOG_LOCKED` spinlock the inner `begin()` span forever
    /// (recode→salvage self-deadlock); the thread-local stack composes safely.
    #[test]
    fn test_nested_begin_does_not_deadlock_and_isolates() {
        let _lock = TEST_MUTEX.lock().unwrap();
        let depth = || CAPTURE_STACK.with(|s| s.borrow().len());
        let base = depth();

        let outer = LogCaptureGuard::begin().unwrap();
        {
            // Inner begin would spin forever on the old spinlock. It returns here.
            let inner = LogCaptureGuard::begin().unwrap();
            assert_eq!(depth(), base + 2, "nested begin stacks");

            // A log emitted now lands in the INNER (top) buffer only.
            unsafe {
                let msg = std::ffi::CString::new("inner-line\n").unwrap();
                ffmpeg_the_third::ffi::av_log(
                    std::ptr::null_mut(),
                    ffmpeg_the_third::ffi::AV_LOG_INFO,
                    msg.as_ptr(),
                );
            }
            let inner_lines = inner.take_captured().unwrap();
            assert!(
                inner_lines.iter().any(|l| l.contains("inner-line")),
                "inner buffer must capture the line emitted while it was top"
            );
            drop(inner);
        }
        assert_eq!(depth(), base + 1, "inner drop pops back to outer");

        // After inner drops, the outer buffer is top again.
        unsafe {
            let msg = std::ffi::CString::new("outer-line\n").unwrap();
            ffmpeg_the_third::ffi::av_log(
                std::ptr::null_mut(),
                ffmpeg_the_third::ffi::AV_LOG_INFO,
                msg.as_ptr(),
            );
        }
        let outer_lines = outer.take_captured().unwrap();
        assert!(
            outer_lines.iter().any(|l| l.contains("outer-line")),
            "outer buffer captures after inner pops"
        );
        assert!(
            !outer_lines.iter().any(|l| l.contains("inner-line")),
            "outer must NOT see the inner session's line (isolation)"
        );
        drop(outer);
        assert_eq!(depth(), base);
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

    /// When no capture buffer is active on the thread AND no forwarder is set,
    /// the install-once callback must fall through to `av_log_default_callback`
    /// (stderr) without panicking — and the orphan message must NOT be
    /// retroactively captured by a later session. Exercises the no-sink branch
    /// of `capture_callback` (the coverage gap noted in code review).
    #[test]
    fn test_no_active_sink_falls_through_without_capture() {
        let _lock = TEST_MUTEX.lock().unwrap();
        clear_log_forwarder();
        // Ensure the callback is installed and no capture is active on this thread.
        drop(LogCaptureGuard::begin().expect("begin"));
        assert_eq!(
            CAPTURE_STACK.with(|s| s.borrow().len()),
            0,
            "no capture session should be active"
        );
        assert!(LOG_FORWARDER.lock().unwrap().is_none());

        // Emit with no sink active → must route to the default callback (stderr),
        // not panic, and not be buffered anywhere.
        unsafe {
            let msg = std::ffi::CString::new("orphan-no-sink-line\n").unwrap();
            ffmpeg_the_third::ffi::av_log(
                std::ptr::null_mut(),
                ffmpeg_the_third::ffi::AV_LOG_ERROR,
                msg.as_ptr(),
            );
        }

        // A subsequently-started capture must not contain the orphan line.
        let guard = LogCaptureGuard::begin().expect("begin 2");
        let captured = guard.take_captured().expect("take");
        assert!(
            !captured.iter().any(|l| l.contains("orphan-no-sink-line")),
            "message emitted with no active sink must not be captured later; got {captured:?}"
        );
    }

    // ── Regression: concurrent capture sessions are thread-isolated ──────────
    //
    // Two `spawn_blocking` tasks each call `LogCaptureGuard::begin()` on their
    // own blocking-pool thread and emit a unique sentinel via `av_log`. Capture
    // buffers are thread-local, so each task must see only its own line — no
    // cross-contamination — and neither may hang (the old process-wide spinlock
    // serialised them; the thread-local design needs no lock at all).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_bridge_callers_do_not_race() {
        // No TEST_MUTEX: this test specifically exercises two concurrent
        // capture sessions on different threads.

        fn emit(line: &str) {
            // SAFETY: literal fmt with no conversions; valid for the call.
            unsafe {
                let msg = std::ffi::CString::new(line).unwrap();
                ffmpeg_the_third::ffi::av_log(
                    std::ptr::null_mut(),
                    ffmpeg_the_third::ffi::AV_LOG_INFO,
                    msg.as_ptr(),
                );
            }
        }

        let task_a = tokio::task::spawn_blocking(|| {
            let guard = LogCaptureGuard::begin().expect("begin a");
            emit("task-A-sentinel\n");
            let captured = guard.take_captured().expect("take a");
            drop(guard);
            captured
        });

        let task_b = tokio::task::spawn_blocking(|| {
            let guard = LogCaptureGuard::begin().expect("begin b");
            emit("task-B-sentinel\n");
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
