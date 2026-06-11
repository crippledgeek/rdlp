//! Cross-platform shutdown-signal handling for the CLI (#413).
//!
//! Handles SIGINT + SIGTERM (Unix) and `CTRL_C` + `CTRL_CLOSE` (Windows) via
//! `tokio::signal`. First interrupt = graceful cancel (keep the resumable
//! partial); second SIGINT = force-exit. Uses `tokio::signal` — not the
//! `ctrlc` crate, which fights tokio's signal driver.

use std::future;
use tokio::signal;

/// Which class of shutdown signal fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGINT / Ctrl+C — interactive; escalates on repeat.
    Interrupt,
    /// SIGTERM / Windows console-close — programmatic; never escalates.
    Terminate,
}

/// What the signal task should do for `(graceful_started, signal)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    /// Begin graceful cancel; carry the exit code to emit later (130/143).
    GracefulCancel(u8),
    /// A human pressed Ctrl+C again (or during a programmatic stop) — exit now.
    ForceExit,
    /// Repeat programmatic signal while already cancelling — ignore.
    Ignore,
}

/// Pure state transition for the escalation state machine (#413, validated).
/// `graceful_started` is the flag the caller keeps across iterations.
#[must_use]
pub const fn next_action(graceful_started: bool, sig: Signal) -> SignalAction {
    match (graceful_started, sig) {
        // First interrupt OR first programmatic stop: begin graceful cancel.
        (false, Signal::Interrupt) => SignalAction::GracefulCancel(130),
        (false, Signal::Terminate) => SignalAction::GracefulCancel(143),
        // Any human interrupt after a graceful cancel has begun: force-exit now.
        (true, Signal::Interrupt) => SignalAction::ForceExit,
        // Repeat programmatic signal while already cancelling: ignore (preserve
        // the systemd/docker grace window — do NOT force).
        (true, Signal::Terminate) => SignalAction::Ignore,
    }
}

/// Resolve on the first shutdown signal; report its class.
///
/// cfg-gated `let`-bindings (NOT `#[cfg]` inside `select!` — unsupported,
/// tokio #3974) with `future::pending()` stubs on the non-applicable platform.
///
/// # Panics
///
/// Panics if the OS refuses to install a signal handler (out of file
/// descriptors, or a double-registration race). Either condition is
/// unrecoverable at startup, so a panic is the correct response.
pub async fn wait_for_signal() -> Signal {
    let ctrl_c = async {
        #[allow(clippy::expect_used)]
        // tokio signal install — propagation impossible; panic is correct
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        #[allow(clippy::expect_used)]
        // tokio signal install — propagation impossible; panic is correct
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = future::pending::<()>();

    #[cfg(windows)]
    let win_close = async {
        #[allow(clippy::expect_used)]
        // tokio signal install — propagation impossible; panic is correct
        signal::windows::ctrl_close()
            .expect("failed to install CtrlClose handler")
            .recv()
            .await;
    };
    #[cfg(not(windows))]
    let win_close = future::pending::<()>();

    tokio::select! {
        () = ctrl_c    => Signal::Interrupt,
        () = terminate => Signal::Terminate,
        () = win_close => Signal::Terminate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_sigint_graceful_130() {
        assert_eq!(
            next_action(false, Signal::Interrupt),
            SignalAction::GracefulCancel(130)
        );
    }
    #[test]
    fn test_first_sigterm_graceful_143() {
        assert_eq!(
            next_action(false, Signal::Terminate),
            SignalAction::GracefulCancel(143)
        );
    }
    #[test]
    fn test_second_sigint_force_exits() {
        assert_eq!(
            next_action(true, Signal::Interrupt),
            SignalAction::ForceExit
        );
    }
    #[test]
    fn test_sigint_after_sigterm_force_exits() {
        // Load-bearing: a human must be able to force-quit a slow programmatic
        // (SIGTERM) shutdown. `graceful_started` is signal-agnostic, so this is
        // deliberately the SAME (true, Interrupt) arm as test_second_sigint_force_exits
        // — documented separately because it pins a distinct user-facing guarantee.
        assert_eq!(
            next_action(true, Signal::Interrupt),
            SignalAction::ForceExit
        );
    }
    #[test]
    fn test_repeat_sigterm_ignored() {
        assert_eq!(next_action(true, Signal::Terminate), SignalAction::Ignore);
    }
}
