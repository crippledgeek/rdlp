//! Token-bucket rate limiter implementation.

use std::sync::Arc;
use tokio::sync::Mutex;
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
    state: Arc<Mutex<TokenBucketState>>,
}

impl RateLimiter {
    /// Create a new rate limiter with the given bytes-per-second limit.
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Self {
        let bps = bytes_per_second as f64;
        Self {
            bytes_per_second: bps,
            state: Arc::new(Mutex::new(TokenBucketState {
                tokens: bps, // Start with full bucket (1-second burst)
                last_refill: Instant::now(),
            })),
        }
    }

    /// Acquire permission to transfer `bytes` bytes.
    ///
    /// If insufficient tokens are available, sleeps until enough accumulate.
    /// The mutex is released before sleeping so other tasks are not blocked.
    pub async fn acquire(&self, bytes: usize) {
        let sleep_duration = {
            let mut state = self.state.lock().await;
            let now = Instant::now();
            let elapsed = now.duration_since(state.last_refill).as_secs_f64();

            // Refill tokens based on elapsed time
            state.tokens += elapsed * self.bytes_per_second;
            // Cap at burst size (1 second worth)
            if state.tokens > self.bytes_per_second {
                state.tokens = self.bytes_per_second;
            }
            state.last_refill = now;

            // Consume tokens
            state.tokens -= bytes as f64;

            // If tokens went negative, calculate sleep time for the deficit
            if state.tokens < 0.0 {
                let deficit = -state.tokens;
                Some(Duration::from_secs_f64(deficit / self.bytes_per_second))
            } else {
                None
            }
        }; // Mutex released here — before sleeping

        if let Some(duration) = sleep_duration {
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
