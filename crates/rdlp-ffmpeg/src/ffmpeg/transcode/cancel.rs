//! Per-iteration cooperative cancellation poll for blocking `FFmpeg` loops.
use tokio_util::sync::CancellationToken;

/// Returns `Err` if `cancel` is present and cancelled; otherwise `Ok(())`.
/// Safe to call from a `spawn_blocking` closure (`is_cancelled()` is synchronous).
#[allow(dead_code)]
pub fn check_cancelled(cancel: Option<&CancellationToken>) -> anyhow::Result<()> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        anyhow::bail!("cancelled by user");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_ok() {
        assert!(check_cancelled(None).is_ok());
    }

    #[test]
    fn live_token_is_ok() {
        let t = CancellationToken::new();
        assert!(check_cancelled(Some(&t)).is_ok());
    }

    #[test]
    fn cancelled_token_is_err() {
        let t = CancellationToken::new();
        t.cancel();
        assert!(check_cancelled(Some(&t)).is_err());
    }
}
