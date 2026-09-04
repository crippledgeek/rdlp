//! The one retry mechanism every download path uses.
//!
//! Four paths need the same thing — walk a backoff, retry only errors worth
//! retrying, log each attempt — and before issue #570 three of them spelled it
//! out separately (`http`'s own `with_retry`, `dash::download::download_one`,
//! and `http::parallel::download_chunk_with_retry`'s hand-rolled loop). What
//! varies between them is *policy* (how many attempts, how long between them,
//! what else must hold) and the work itself, so both are passed in as values
//! and the mechanism exists once.
//!
//! Cancellation is part of the mechanism rather than bolted on by each caller:
//! [`with_retry_cancellable`] races the whole loop, backoff sleeps included, so
//! a cancelled download does not sit out a 60-second sleep before noticing.

use std::fmt::Display;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use backon::Retryable as _;
use log::warn;
use rdlp_core::{RdlpError, Result, RetryConfig, is_retryable_error};
use tokio_util::sync::CancellationToken;

/// How one operation retries: which backoff to walk, what to call it in logs,
/// and what must hold beyond the error simply being transient.
///
/// Copy rather than move-only so [`with_retry_cancellable`] can hand the same
/// policy to either arm of its `select!` — every field is a shared reference.
#[derive(Clone, Copy)]
pub(crate) struct RetryPolicy<'a> {
    config: &'a RetryConfig,
    /// Names the operation in the retry log line. Callers that fetch one of
    /// many similar things (a fragment, a chunk) put the identifier here.
    ///
    /// `Display` rather than `&str` so a caller whose label costs something to
    /// build — a sanitized URL, a formatted id — pays only on the retry path.
    /// The overwhelming majority of operations never retry.
    context: &'a (dyn Display + Sync),
    gate: Option<&'a (dyn Fn() -> bool + Sync)>,
    counter: Option<&'a AtomicU64>,
}

impl<'a> RetryPolicy<'a> {
    /// Retry `config`'s transient errors, logging them under `context`.
    pub(crate) const fn new(config: &'a RetryConfig, context: &'a (dyn Display + Sync)) -> Self {
        Self {
            config,
            context,
            gate: None,
            counter: None,
        }
    }

    /// Additionally require `gate` before any retry.
    ///
    /// Consulted only once [`is_retryable_error`] has already said yes, so a
    /// 404 never draws on it. The fragment path passes its list-wide budget
    /// here, which is why the gate may have side effects — it is called
    /// exactly once per would-be retry.
    pub(crate) const fn gated_by(mut self, gate: &'a (dyn Fn() -> bool + Sync)) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Count retries actually taken into `counter`.
    ///
    /// Incremented from the notify hook, which backon calls only when a delay
    /// was obtained and another attempt will follow — so this counts attempts
    /// made, not failures seen. Feeds `DownloadStats::retries`.
    pub(crate) const fn counting_into(mut self, counter: &'a AtomicU64) -> Self {
        self.counter = Some(counter);
        self
    }
}

/// Run `operation` under `policy`, retrying transient failures.
///
/// Non-retryable errors ([`is_retryable_error`] admits only 5xx, 429, and
/// network/I/O) return on the first response, so a dead URL is not hammered.
pub(crate) async fn with_retry<F, Fut, T>(policy: RetryPolicy<'_>, operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    operation
        .retry(policy.config.to_backoff())
        .when(|err: &RdlpError| is_retryable_error(err) && policy.gate.is_none_or(|gate| gate()))
        .notify(|err, dur| {
            if let Some(counter) = policy.counter {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            warn!(delay:? = dur; "{} failed, retrying: {err}", policy.context);
        })
        .await
}

/// Run `operation` under `policy`, racing the retry loop against `cancel`.
///
/// The race covers the whole loop rather than a single attempt: without it a
/// cancel arriving during a backoff sleep would not be observed until the
/// sleep expired, which at the default policy's ceiling is a minute. `biased`
/// gives the cancel arm priority, so an already-cancelled token returns
/// [`RdlpError::Cancelled`] without issuing a request.
///
/// `None` means the caller has no token and the loop runs to completion.
pub(crate) async fn with_retry_cancellable<F, Fut, T>(
    policy: RetryPolicy<'_>,
    cancel: Option<&CancellationToken>,
    operation: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    match cancel {
        Some(token) => tokio::select! {
            biased;
            () = token.cancelled() => Err(RdlpError::Cancelled),
            res = with_retry(policy, operation) => res,
        },
        None => with_retry(policy, operation).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// Millisecond backoff so tests exercise the real loop without sleeping.
    fn fast(max_retries: usize) -> RetryConfig {
        RetryConfig::new(
            max_retries,
            Duration::from_millis(1),
            Duration::from_millis(5),
            2.0,
        )
    }

    fn transient() -> RdlpError {
        RdlpError::Network {
            message: "transient".to_string(),
            url: None,
        }
    }

    #[tokio::test]
    async fn retries_a_transient_failure_until_it_succeeds() {
        let attempts = AtomicUsize::new(0);
        let config = fast(3);
        let out: u32 = with_retry(RetryPolicy::new(&config, &"test"), || async {
            if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                return Err(transient());
            }
            Ok(7)
        })
        .await
        .expect("third attempt succeeds");
        assert_eq!(out, 7);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn stops_at_the_configured_attempt_ceiling() {
        // Both sides of the bound: 2 retries means 3 attempts, not 2 and not 4.
        let attempts = AtomicUsize::new(0);
        let config = fast(2);
        let err = with_retry(RetryPolicy::new(&config, &"test"), || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(transient())
        })
        .await
        .expect_err("an always-failing operation must give up");
        assert!(matches!(err, RdlpError::Network { .. }));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn non_retryable_error_is_returned_on_the_first_attempt() {
        let attempts = AtomicUsize::new(0);
        let config = fast(5);
        let err = with_retry(RetryPolicy::new(&config, &"test"), || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(RdlpError::Http {
                status: 404,
                reason: "gone".to_string(),
            })
        })
        .await
        .expect_err("404 must not be retried");
        assert!(matches!(err, RdlpError::Http { status: 404, .. }));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "a dead URL must be hit exactly once"
        );
    }

    #[tokio::test]
    async fn a_closed_gate_prevents_retrying_an_otherwise_retryable_error() {
        let attempts = AtomicUsize::new(0);
        let config = fast(5);
        let closed = || false;
        let err = with_retry(
            RetryPolicy::new(&config, &"test").gated_by(&closed),
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(transient())
            },
        )
        .await
        .expect_err("the gate must be able to stop a retryable error");
        assert!(matches!(err, RdlpError::Network { .. }));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_gate_is_not_consulted_for_a_non_retryable_error() {
        // The `&&` short-circuit is load-bearing: the fragment path's gate
        // spends a budget token, and a 404 must not cost one.
        let consulted = AtomicUsize::new(0);
        let config = fast(5);
        let gate = || {
            consulted.fetch_add(1, Ordering::SeqCst);
            true
        };
        let _ = with_retry(
            RetryPolicy::new(&config, &"test").gated_by(&gate),
            || async {
                Err::<(), _>(RdlpError::Http {
                    status: 403,
                    reason: "denied".to_string(),
                })
            },
        )
        .await;
        assert_eq!(consulted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn retries_taken_are_counted() {
        let counter = AtomicU64::new(0);
        let attempts = AtomicUsize::new(0);
        let config = fast(5);
        with_retry(
            RetryPolicy::new(&config, &"test").counting_into(&counter),
            || async {
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    return Err(transient());
                }
                Ok(())
            },
        )
        .await
        .expect("succeeds on the third attempt");
        assert_eq!(
            counter.load(Ordering::Relaxed),
            2,
            "three attempts is two retries"
        );
    }

    #[tokio::test]
    async fn an_already_cancelled_token_skips_the_operation_entirely() {
        let attempts = AtomicUsize::new(0);
        let config = fast(3);
        let token = CancellationToken::new();
        token.cancel();
        let err =
            with_retry_cancellable(RetryPolicy::new(&config, &"test"), Some(&token), || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok::<(), RdlpError>(())
            })
            .await
            .expect_err("a cancelled token must not issue the request");
        assert!(matches!(err, RdlpError::Cancelled));
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_cancel_during_backoff_returns_without_waiting_out_the_sleep() {
        // The guarantee this whole wrapper exists for: the operation always
        // fails, and the backoff between attempts is a minute, so anything
        // that waits for a sleep cannot finish inside the assertion below.
        // 30s between attempts: far longer than the assertion below tolerates,
        // so a wrapper that waited for the sleep could not pass.
        let config = RetryConfig::new(5, Duration::from_secs(30), Duration::from_secs(30), 2.0);
        let token = CancellationToken::new();
        let cancel_token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_token.cancel();
        });

        let started = std::time::Instant::now();
        let err =
            with_retry_cancellable(RetryPolicy::new(&config, &"test"), Some(&token), || async {
                Err::<(), _>(transient())
            })
            .await
            .expect_err("cancelling mid-backoff must end the loop");
        assert!(matches!(err, RdlpError::Cancelled));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "returned only after {:?}; the 60s backoff sleep was not interrupted",
            started.elapsed()
        );
    }
}
