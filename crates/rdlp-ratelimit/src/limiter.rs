//! Token-bucket rate limiter implementation.

use std::sync::{Arc, Mutex};
use tokio::time::{Duration, Instant};

/// Internal mutable state for the token bucket.
struct TokenBucketState {
    /// Available tokens (bytes). Can go negative to represent debt.
    tokens: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

/// Async token-bucket rate limiter for bandwidth throttling.
///
/// Shared across all parallel connections via `Arc` internally.
/// When rate limiting is active, each chunk write calls [`acquire`](Self::acquire)
/// which sleeps if the byte budget is exhausted.
///
/// # Design
///
/// - Tokens (bytes) refill at `bytes_per_second` rate
/// - Burst capacity: up to 1 second of data
/// - One instance shared by all HTTP and HLS connections
/// - Zero overhead when wrapped in `Option<Arc<RateLimiter>>`
#[derive(Clone)]
pub struct RateLimiter {
    bytes_per_second: f64,
    // std::sync::Mutex is intentional: guards never cross an .await point.
    // The critical section is pure arithmetic; the sleep is outside the guard.
    // See docs/implementation/tls-impersonation/phase-1-report.md Finding 2.1.
    state: Arc<Mutex<TokenBucketState>>,
}

impl RateLimiter {
    /// Create a new rate limiter with the given bytes-per-second limit.
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Self {
        // Intentional: u64 -> f64 precision loss is acceptable for bandwidth values
        // (rates above 2^53 bytes/s = 8 petabytes/s are not realistic)
        #[allow(clippy::cast_precision_loss)]
        let bps = bytes_per_second as f64;
        Self {
            bytes_per_second: bps,
            state: Arc::new(Mutex::new(TokenBucketState {
                tokens: bps, // Start with full bucket (1-second burst)
                last_refill: Instant::now(),
            })),
        }
    }

    /// Compute the sleep duration needed after consuming `bytes` tokens.
    ///
    /// Returns the sleep duration if tokens went negative (throttling needed),
    /// or `None` if there are sufficient tokens. The mutex is released
    /// before this function returns so the caller can sleep without holding it.
    #[allow(clippy::cast_precision_loss)] // usize->f64: chunk sizes are never > 2^52
    fn compute_sleep(&self, bytes: usize) -> Option<Duration> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();

        // Refill tokens based on elapsed time, capped at burst size (1s)
        state.tokens = elapsed
            .mul_add(self.bytes_per_second, state.tokens)
            .min(self.bytes_per_second);
        state.last_refill = now;

        // Consume tokens
        state.tokens -= bytes as f64;

        // If tokens went negative, calculate sleep time for the deficit.
        // Drop `state` (MutexGuard) before returning — caller will sleep.
        if state.tokens < 0.0 {
            let deficit = -state.tokens;
            drop(state);
            Some(Duration::from_secs_f64(deficit / self.bytes_per_second))
        } else {
            None
        }
    }

    /// Acquire permission to transfer `bytes` bytes.
    ///
    /// If insufficient tokens are available, sleeps until enough accumulate.
    /// The mutex is released before sleeping so other tasks are not blocked.
    pub async fn acquire(&self, bytes: usize) {
        if let Some(duration) = self.compute_sleep(bytes) {
            tokio::time::sleep(duration).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_no_delay_within_burst() {
        let limiter = RateLimiter::new(1_000_000); // 1 MB/s
        let start = Instant::now();

        // 500 KB is within burst capacity (1 MB), should be near-instant
        limiter.acquire(500_000).await;

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "Should be near-instant within burst: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_throttle_beyond_burst() {
        let limiter = RateLimiter::new(100_000); // 100 KB/s
        let start = Instant::now();

        // Consume initial burst (100KB) + 100KB extra = need ~1s sleep
        for _ in 0..20 {
            limiter.acquire(10_000).await; // 10 KB each, 200 KB total
        }

        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() > 800, "Should throttle: {elapsed:?}");
        assert!(
            elapsed.as_millis() < 1500,
            "Should not over-throttle: {elapsed:?}"
        );
    }
}
