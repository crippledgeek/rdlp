use crate::RdlpError;
use std::time::Duration;

// Re-export backon types for convenience
pub use backon::{ExponentialBuilder, Retryable};

/// Retry configuration for network operations
///
/// This struct provides user-facing configuration that can be converted
/// to a backon `ExponentialBuilder` for actual retry execution.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Initial delay before first retry
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Exponential backoff multiplier (typically 2.0)
    pub multiplier: f32,
    /// Enable jitter to prevent thundering herd
    pub jitter: bool,
}

impl RetryConfig {
    /// Create a new retry configuration
    pub fn new(
        max_retries: usize,
        initial_delay: Duration,
        max_delay: Duration,
        multiplier: f32,
    ) -> Self {
        Self {
            max_retries,
            initial_delay,
            max_delay,
            multiplier,
            jitter: true, // Enable jitter by default for production resilience
        }
    }

    /// Create default retry configuration (10 retries, 1s-60s backoff, jitter enabled)
    pub fn default_config() -> Self {
        Self {
            max_retries: 10,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
            jitter: true,
        }
    }

    /// Enable or disable jitter
    #[must_use]
    pub fn with_jitter(mut self, enabled: bool) -> Self {
        self.jitter = enabled;
        self
    }

    /// Convert to backon ExponentialBuilder
    ///
    /// This allows using the RetryConfig with backon's `.retry()` trait extension.
    ///
    /// # Example
    /// ```rust,no_run
    /// use rdlp_core::{RetryConfig, Retryable, RdlpError};
    ///
    /// async fn fetch_data() -> Result<String, RdlpError> {
    ///     Ok("data".to_string())
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let config = RetryConfig::default_config();
    ///     let result = (|| async { fetch_data().await })
    ///         .retry(config.to_backoff())
    ///         .await;
    /// }
    /// ```
    pub fn to_backoff(&self) -> ExponentialBuilder {
        let mut builder = ExponentialBuilder::default()
            .with_min_delay(self.initial_delay)
            .with_max_delay(self.max_delay)
            .with_max_times(self.max_retries)
            .with_factor(self.multiplier);

        if self.jitter {
            builder = builder.with_jitter();
        }

        builder
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self::default_config()
    }
}

/// Check if an error is retryable (transient network error)
///
/// Use this with backon's `.when()` method to conditionally retry:
///
/// ```rust,no_run
/// use rdlp_core::{is_retryable_error, RetryConfig, Retryable, RdlpError};
///
/// async fn fetch_data() -> Result<String, RdlpError> {
///     Err(RdlpError::Network("timeout".to_string()))
/// }
///
/// #[tokio::main]
/// async fn main() {
///     let config = RetryConfig::default_config();
///     let result = (|| async { fetch_data().await })
///         .retry(config.to_backoff())
///         .when(is_retryable_error)
///         .await;
/// }
/// ```
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
    fn test_default_config() {
        let config = RetryConfig::default_config();
        assert_eq!(config.max_retries, 10);
        assert_eq!(config.initial_delay, Duration::from_secs(1));
        assert_eq!(config.max_delay, Duration::from_secs(60));
        assert_eq!(config.multiplier, 2.0);
        assert!(config.jitter);
    }

    #[test]
    fn test_to_backoff() {
        let config = RetryConfig::new(
            5,
            Duration::from_millis(100),
            Duration::from_secs(10),
            2.0,
        );

        // Should compile and create a valid builder
        let _builder = config.to_backoff();
    }

    #[test]
    fn test_with_jitter() {
        let config = RetryConfig::default_config().with_jitter(false);
        assert!(!config.jitter);
    }

    #[test]
    fn test_is_retryable_error() {
        // Network errors are retryable
        assert!(is_retryable_error(&RdlpError::Network(
            "timeout".to_string()
        )));
        assert!(is_retryable_error(&RdlpError::Network(
            "HTTP error 503".to_string()
        )));
        assert!(is_retryable_error(&RdlpError::Network(
            "HTTP error 429".to_string()
        )));

        // 4xx client errors (except 429) are not retryable
        assert!(!is_retryable_error(&RdlpError::Network(
            "HTTP error 404".to_string()
        )));
        assert!(!is_retryable_error(&RdlpError::Network(
            "HTTP error 403".to_string()
        )));

        // I/O errors are retryable
        assert!(is_retryable_error(&RdlpError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset"
        ))));

        // Other errors are not retryable
        assert!(!is_retryable_error(&RdlpError::Extraction(
            "invalid format".to_string()
        )));
    }

    // =========================================================================
    // Retry Behavior Tests (using backon)
    // =========================================================================

    /// Test that retry succeeds on second attempt after initial failure
    #[tokio::test]
    async fn test_retry_succeeds_on_second_attempt() {
        let config = RetryConfig::new(
            3,
            Duration::from_millis(1),
            Duration::from_millis(10),
            2.0,
        )
        .with_jitter(false);

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = attempt_counter.clone();

        let result: Result<i32, RdlpError> = (|| {
            let counter = counter_clone.clone();
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
        .retry(config.to_backoff())
        .when(is_retryable_error)
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 2);
    }

    /// Test that retry exhausts all attempts and returns the last error
    #[tokio::test]
    async fn test_retry_exhausts_all_attempts() {
        let config = RetryConfig::new(
            2, // max 2 retries = 3 total attempts
            Duration::from_millis(1),
            Duration::from_millis(10),
            2.0,
        )
        .with_jitter(false);

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = attempt_counter.clone();

        let result: Result<i32, RdlpError> = (|| {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // Always fail with retryable error
                Err(RdlpError::Network("persistent failure".to_string()))
            }
        })
        .retry(config.to_backoff())
        .when(is_retryable_error)
        .await;

        assert!(result.is_err());
        // Should try: initial + 2 retries = 3 total attempts
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 3);
    }

    /// Test that permanent (non-retryable) errors stop immediately
    #[tokio::test]
    async fn test_permanent_error_not_retried() {
        let config = RetryConfig::new(
            5,
            Duration::from_millis(1),
            Duration::from_millis(10),
            2.0,
        )
        .with_jitter(false);

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = attempt_counter.clone();

        let result: Result<i32, RdlpError> = (|| {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // Return a non-retryable error (404)
                Err(RdlpError::Network("HTTP error 404 Not Found".to_string()))
            }
        })
        .retry(config.to_backoff())
        .when(is_retryable_error)
        .await;

        assert!(result.is_err());
        // Should only attempt once since 404 is not retryable
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 1);
    }

    /// Test that notify callback is called on each retry
    #[tokio::test]
    async fn test_retry_notify_callback() {
        let config = RetryConfig::new(
            2,
            Duration::from_millis(1),
            Duration::from_millis(10),
            2.0,
        )
        .with_jitter(false);

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let notify_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = attempt_counter.clone();
        let notify_clone = notify_counter.clone();

        let result: Result<i32, RdlpError> = (|| {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err(RdlpError::Network("always fails".to_string()))
            }
        })
        .retry(config.to_backoff())
        .when(is_retryable_error)
        .notify(move |_err, _dur| {
            notify_clone.fetch_add(1, Ordering::SeqCst);
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 3); // 3 attempts
        assert_eq!(notify_counter.load(Ordering::SeqCst), 2); // 2 retries (notified)
    }

    /// Test retry with mixed errors (retryable then permanent)
    #[tokio::test]
    async fn test_retry_stops_on_permanent_after_transient() {
        let config = RetryConfig::new(
            5,
            Duration::from_millis(1),
            Duration::from_millis(10),
            2.0,
        )
        .with_jitter(false);

        let attempt_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = attempt_counter.clone();

        let result: Result<i32, RdlpError> = (|| {
            let counter = counter_clone.clone();
            async move {
                let attempts = counter.fetch_add(1, Ordering::SeqCst);
                if attempts < 2 {
                    // First two attempts: retryable error
                    Err(RdlpError::Network("timeout".to_string()))
                } else {
                    // Third attempt: permanent error
                    Err(RdlpError::Network("HTTP error 403 Forbidden".to_string()))
                }
            }
        })
        .retry(config.to_backoff())
        .when(is_retryable_error)
        .await;

        assert!(result.is_err());
        // Should stop at 3rd attempt when permanent error encountered
        assert_eq!(attempt_counter.load(Ordering::SeqCst), 3);
    }
}
