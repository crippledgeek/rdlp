use crate::{Result, RdlpError};
use std::future::Future;
use std::time::Duration;

/// Retry configuration for network operations
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Exponential backoff multiplier (typically 2.0)
    pub multiplier: f64,
}

impl RetryConfig {
    /// Create a new retry configuration
    pub fn new(max_retries: usize, initial_delay: Duration, max_delay: Duration, multiplier: f64) -> Self {
        Self {
            max_retries,
            initial_delay,
            max_delay,
            multiplier,
        }
    }

    /// Create default retry configuration (10 retries, 1s-60s backoff)
    pub fn default_config() -> Self {
        Self {
            max_retries: 10,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
        }
    }

    /// Calculate delay for a given attempt number (0-indexed)
    pub fn calculate_delay(&self, attempt: usize) -> Duration {
        let delay_secs = self.initial_delay.as_secs_f64() * self.multiplier.powi(attempt as i32);
        Duration::from_secs_f64(delay_secs.min(self.max_delay.as_secs_f64()))
    }
}

/// Execute an async operation with retry logic and exponential backoff
///
/// # Arguments
/// * `config` - Retry configuration
/// * `operation_name` - Human-readable name for logging
/// * `f` - Async function to retry (takes attempt number as parameter)
///
/// # Returns
/// Result from the operation, or the last error if all retries exhausted
///
/// # Example
/// ```rust,ignore
/// let result = retry_with_backoff(
///     &retry_config,
///     "download chunk",
///     |attempt| async move {
///         client.get(url).send().await
///     }
/// ).await?;
/// ```
pub async fn retry_with_backoff<F, Fut, T>(
    config: &RetryConfig,
    operation_name: &str,
    mut f: F,
) -> Result<T>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut last_error: Option<RdlpError> = None;

    for attempt in 0..=config.max_retries {
        match f(attempt).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                last_error = Some(e);

                // Don't retry on the last attempt
                if attempt < config.max_retries {
                    let delay = config.calculate_delay(attempt);
                    eprintln!(
                        "⚠️  {} failed (attempt {}/{}), retrying in {:.1}s...",
                        operation_name,
                        attempt + 1,
                        config.max_retries + 1,
                        delay.as_secs_f64()
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    // All retries exhausted
    Err(last_error.unwrap_or_else(|| {
        RdlpError::Network(format!("{} failed after {} attempts", operation_name, config.max_retries + 1))
    }))
}

/// Check if an error is retryable (transient network error)
pub fn is_retryable_error(error: &RdlpError) -> bool {
    match error {
        // Network errors are generally retryable
        RdlpError::Network(msg) => {
            // Don't retry HTTP 4xx client errors (except 429 rate limit)
            if msg.contains("HTTP error 4") && !msg.contains("429") {
                return false;
            }
            true
        }
        // I/O errors might be retryable (disk full, temp failure)
        RdlpError::Io(_) => true,
        // Other errors are not retryable
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_calculate_delay() {
        let config = RetryConfig::new(
            5,
            Duration::from_secs(1),
            Duration::from_secs(30),
            2.0,
        );

        // Attempt 0: 1s * 2^0 = 1s
        assert_eq!(config.calculate_delay(0), Duration::from_secs(1));

        // Attempt 1: 1s * 2^1 = 2s
        assert_eq!(config.calculate_delay(1), Duration::from_secs(2));

        // Attempt 2: 1s * 2^2 = 4s
        assert_eq!(config.calculate_delay(2), Duration::from_secs(4));

        // Attempt 3: 1s * 2^3 = 8s
        assert_eq!(config.calculate_delay(3), Duration::from_secs(8));

        // Attempt 4: 1s * 2^4 = 16s
        assert_eq!(config.calculate_delay(4), Duration::from_secs(16));

        // Attempt 5: 1s * 2^5 = 32s, but capped at 30s
        assert_eq!(config.calculate_delay(5), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_second_attempt() {
        let config = RetryConfig::new(
            3,
            Duration::from_millis(10),
            Duration::from_millis(100),
            2.0,
        );

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let attempt_counter_clone = attempt_counter.clone();

        let result = retry_with_backoff(&config, "test operation", |_attempt| {
            let counter = attempt_counter_clone.clone();
            async move {
                let attempts = counter.fetch_add(1, Ordering::SeqCst);
                if attempts < 1 {
                    // Fail on first attempt
                    Err(RdlpError::Network("temporary failure".to_string()))
                } else {
                    // Succeed on second attempt
                    Ok(42)
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_exhausts_all_attempts() {
        let config = RetryConfig::new(
            2,
            Duration::from_millis(10),
            Duration::from_millis(100),
            2.0,
        );

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let attempt_counter_clone = attempt_counter.clone();

        let result: Result<i32> = retry_with_backoff(&config, "test operation", |_attempt| {
            let counter = attempt_counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // Always fail
                Err(RdlpError::Network("persistent failure".to_string()))
            }
        })
        .await;

        assert!(result.is_err());
        // Should try: initial + 2 retries = 3 total attempts
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_is_retryable_error() {
        // Network errors are retryable
        assert!(is_retryable_error(&RdlpError::Network("timeout".to_string())));
        assert!(is_retryable_error(&RdlpError::Network("HTTP error 503".to_string())));
        assert!(is_retryable_error(&RdlpError::Network("HTTP error 429".to_string())));

        // 4xx client errors (except 429) are not retryable
        assert!(!is_retryable_error(&RdlpError::Network("HTTP error 404".to_string())));
        assert!(!is_retryable_error(&RdlpError::Network("HTTP error 403".to_string())));

        // I/O errors are retryable
        assert!(is_retryable_error(&RdlpError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));

        // Other errors are not retryable
        assert!(!is_retryable_error(&RdlpError::Extraction("invalid format".to_string())));
    }
}
