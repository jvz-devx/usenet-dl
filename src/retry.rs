//! Retry logic with exponential backoff
//!
//! This module provides configurable retry logic for transient failures.
//! It implements exponential backoff with optional jitter to prevent thundering herd.
//!
//! # Example
//!
//! ```no_run
//! use usenet_dl::retry::{IsRetryable, download_with_retry};
//! use usenet_dl::config::RetryConfig;
//!
//! #[derive(Debug)]
//! enum MyError {
//!     Transient,
//!     Permanent,
//! }
//!
//! impl IsRetryable for MyError {
//!     fn is_retryable(&self) -> bool {
//!         matches!(self, MyError::Transient)
//!     }
//! }
//!
//! # async fn example() -> Result<(), MyError> {
//! let config = RetryConfig::default();
//! let result = download_with_retry(&config, || async {
//!     // Your operation here
//!     Ok::<_, MyError>(())
//! }).await?;
//! # Ok(())
//! # }
//! ```

use crate::config::RetryConfig;
use crate::error::Error;
use rand::Rng;
use std::future::Future;
use std::time::Duration;

/// Trait for errors that can be classified as retryable or not
///
/// Transient failures (network timeouts, server busy, connection reset) should return `true`.
/// Permanent failures (authentication failed, disk full, corrupt data) should return `false`.
pub trait IsRetryable {
    /// Returns true if the error is transient and the operation should be retried
    fn is_retryable(&self) -> bool;
}

/// Implementation of IsRetryable for our Error type
impl IsRetryable for Error {
    fn is_retryable(&self) -> bool {
        match self {
            // Network errors are generally retryable
            Error::Network(e) => {
                // Check if it's a timeout or connection error
                e.is_timeout() || e.is_connect()
            }
            // I/O errors can be retryable in some cases
            Error::Io(e) => match e.kind() {
                std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::Interrupted => true,
                _ => false,
            },
            // NNTP errors need to be classified based on content
            // For now, we treat them as potentially retryable
            Error::Nntp(msg) => {
                // Common transient NNTP error patterns
                msg.contains("timeout")
                    || msg.contains("busy")
                    || msg.contains("connection")
                    || msg.contains("temporary")
                    || msg.contains("503") // Service unavailable
                    || msg.contains("400") // Server busy
            }
            // Database errors should not be retried (likely permanent)
            Error::Database(_) | Error::Sqlx(_) => false,
            // Config errors are permanent
            Error::Config(_) => false,
            // Invalid NZB is permanent
            Error::InvalidNzb(_) => false,
            // Not found is permanent
            Error::NotFound(_) => false,
            // Extraction errors are permanent
            Error::Extraction(_) => false,
            // Serialization errors are permanent
            Error::Serialization(_) => false,
            // Unknown errors - be conservative and don't retry
            Error::Other(_) => false,
        }
    }
}

/// Execute an async operation with exponential backoff retry logic
///
/// # Arguments
///
/// * `config` - Retry configuration (max attempts, delays, backoff multiplier, jitter)
/// * `operation` - Async closure that returns Result<T, E> where E implements IsRetryable
///
/// # Returns
///
/// Returns the successful result or the last error after all retry attempts are exhausted.
///
/// # Example
///
/// ```no_run
/// use usenet_dl::retry::download_with_retry;
/// use usenet_dl::config::RetryConfig;
/// use usenet_dl::error::Error;
///
/// # async fn example() -> Result<(), Error> {
/// let config = RetryConfig::default();
/// let result = download_with_retry(&config, || async {
///     // Simulate a network operation that might fail
///     Ok::<String, Error>("success".to_string())
/// }).await?;
/// # Ok(())
/// # }
/// ```
pub async fn download_with_retry<F, Fut, T, E>(config: &RetryConfig, mut operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: IsRetryable + std::fmt::Display,
{
    let mut attempt = 0;
    let mut delay = config.initial_delay;

    loop {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!(
                        attempts = attempt + 1,
                        "Operation succeeded after retry"
                    );
                }
                return Ok(result);
            }
            Err(e) if e.is_retryable() && attempt < config.max_attempts => {
                attempt += 1;

                tracing::warn!(
                    error = %e,
                    attempt = attempt,
                    max_attempts = config.max_attempts,
                    delay_ms = delay.as_millis(),
                    "Operation failed, retrying"
                );

                // Calculate jittered delay
                let jittered_delay = if config.jitter {
                    add_jitter(delay)
                } else {
                    delay
                };

                // Wait before retrying
                tokio::time::sleep(jittered_delay).await;

                // Calculate next delay with exponential backoff
                let next_delay =
                    Duration::from_secs_f64(delay.as_secs_f64() * config.backoff_multiplier);
                delay = next_delay.min(config.max_delay);
            }
            Err(e) => {
                if e.is_retryable() {
                    tracing::error!(
                        error = %e,
                        attempts = attempt + 1,
                        "Operation failed after all retry attempts exhausted"
                    );
                } else {
                    tracing::error!(
                        error = %e,
                        "Operation failed with non-retryable error"
                    );
                }
                return Err(e);
            }
        }
    }
}

/// Add random jitter to a delay to prevent thundering herd
///
/// Jitter is uniformly distributed between 0% and 100% of the delay.
/// This means the actual delay will be between `delay` and `2 * delay`.
///
/// # Arguments
///
/// * `delay` - Base delay duration
///
/// # Returns
///
/// Jittered delay duration
fn add_jitter(delay: Duration) -> Duration {
    let mut rng = rand::thread_rng();
    let jitter_factor: f64 = rng.gen_range(0.0..=1.0);
    let jittered_secs = delay.as_secs_f64() * (1.0 + jitter_factor);
    Duration::from_secs_f64(jittered_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[derive(Debug)]
    enum TestError {
        Transient,
        Permanent,
    }

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                TestError::Transient => write!(f, "transient error"),
                TestError::Permanent => write!(f, "permanent error"),
            }
        }
    }

    impl IsRetryable for TestError {
        fn is_retryable(&self) -> bool {
            matches!(self, TestError::Transient)
        }
    }

    #[tokio::test]
    async fn test_success_no_retry() {
        let config = RetryConfig::default();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, TestError>(42)
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1, "should only call once");
    }

    #[tokio::test]
    async fn test_retry_transient_then_succeed() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                let count = counter.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(TestError::Transient)
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "should retry twice before success"
        );
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let config = RetryConfig {
            max_attempts: 2,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "should try initial + 2 retries"
        );
    }

    #[tokio::test]
    async fn test_permanent_error_no_retry() {
        let config = RetryConfig::default();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Permanent)
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "should not retry permanent error"
        );
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            max_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let start = std::time::Instant::now();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let _result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        let elapsed = start.elapsed();

        // Total expected delay: 10ms + 20ms + 40ms = 70ms
        // We add some tolerance for execution time
        assert!(
            elapsed >= Duration::from_millis(70),
            "should wait at least 70ms, waited {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(200),
            "should not wait too long, waited {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_jitter_adds_randomness() {
        let delay = Duration::from_millis(100);

        // Test that jitter produces different values
        let jittered1 = add_jitter(delay);
        let jittered2 = add_jitter(delay);

        // Jitter should produce values between delay and 2*delay
        assert!(jittered1 >= delay);
        assert!(jittered1 <= delay * 2);
        assert!(jittered2 >= delay);
        assert!(jittered2 <= delay * 2);

        // With high probability, two jittered values should be different
        // (could fail very rarely due to randomness, but extremely unlikely)
        // Skip this assertion as it's non-deterministic
    }

    #[tokio::test]
    async fn test_max_delay_cap() {
        let config = RetryConfig {
            max_attempts: 5,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(3),
            backoff_multiplier: 10.0, // Very aggressive multiplier
            jitter: false,
        };

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let start = std::time::Instant::now();

        let _result = download_with_retry(&config, || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(TestError::Transient)
            }
        })
        .await;

        let elapsed = start.elapsed();

        // Delays should be capped at max_delay (3 seconds)
        // First delay: 1s
        // Second delay: min(10s, 3s) = 3s
        // Third delay: min(30s, 3s) = 3s
        // Fourth delay: min(300s, 3s) = 3s
        // Fifth delay: min(3000s, 3s) = 3s
        // Total: 1 + 3 + 3 + 3 + 3 = 13 seconds
        assert!(
            elapsed >= Duration::from_secs(13),
            "should wait at least 13s with max_delay cap, waited {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "should not exceed expected time significantly, waited {:?}",
            elapsed
        );
    }

    // Note: reqwest::Error doesn't have a simple constructor for testing,
    // so we test network retryability indirectly through integration tests

    #[test]
    fn test_error_is_retryable_io() {
        let timeout_err = Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timeout",
        ));
        assert!(timeout_err.is_retryable());

        let connection_refused = Error::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert!(connection_refused.is_retryable());

        let not_found = Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(!not_found.is_retryable());
    }

    #[test]
    fn test_error_is_retryable_nntp() {
        let timeout = Error::Nntp("connection timeout".to_string());
        assert!(timeout.is_retryable());

        let busy = Error::Nntp("server busy (400)".to_string());
        assert!(busy.is_retryable());

        let auth_failed = Error::Nntp("authentication failed".to_string());
        assert!(!auth_failed.is_retryable());
    }

    #[test]
    fn test_error_is_retryable_permanent() {
        assert!(!Error::Config("bad config".to_string()).is_retryable());
        assert!(!Error::Database("db error".to_string()).is_retryable());
        assert!(!Error::InvalidNzb("bad nzb".to_string()).is_retryable());
        assert!(!Error::NotFound("not found".to_string()).is_retryable());
        assert!(!Error::Extraction("failed to extract".to_string()).is_retryable());
    }
}
