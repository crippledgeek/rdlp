//! Global panic hook for the Tauri desktop app.
//!
//! Tauri v2 has no built-in panic catch around async command handlers.
//! Without this hook, a panic in any command dies silently and can tear
//! down the webview on some platforms. Installing a global hook captures
//! the panic payload and backtrace to the log so crashes are diagnosable.
//!
//! Research basis: Aptabase's canonical Tauri panic post; sentry-tauri;
//! the Spacedrive project's panic-capture PR (#2504). All three use the
//! same `std::panic::set_hook` pattern as the foundational mechanism.
//!
//! Introduced alongside the removal of per-command `spawn_blocking`
//! wrappers (TLS Phase 1 async audit, Finding 5.R1). The global hook
//! restores panic observability with a stronger blanket guarantee —
//! every command, not just the ones that happened to be wrapped.

use std::backtrace::Backtrace;
use std::panic;

/// Install the global panic hook.
///
/// Idempotent in practice (calling twice simply re-chains the previous
/// hook). Call once at app startup before `tauri::Builder::default()`.
pub fn install_panic_hook() {
    // Preserve the previous hook so tooling (default stderr printer,
    // dev tooling output) still fires after ours runs.
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let backtrace = Backtrace::capture();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".into());
        let payload = panic_payload_str(info.payload());

        log::error!(
            "Tauri app panicked at {location}: {payload}\nbacktrace:\n{backtrace}"
        );

        prev_hook(info);
    }));
}

fn panic_payload_str(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity guard: the installer runs without error.
    ///
    /// Documented per `bug-fix-requires-failing-test.md` §Exception
    /// Handling: the hook firing cannot be safely tested in-process —
    /// panic unwinding would poison the test binary. Behavioural
    /// verification is manual (trigger a panic in dev-mode run and
    /// check log output).
    #[test]
    fn panic_hook_installs_without_error() {
        install_panic_hook();
    }
}
