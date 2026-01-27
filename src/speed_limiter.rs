//! Speed limiting using token bucket algorithm
//!
//! The SpeedLimiter provides global bandwidth limiting across all concurrent downloads
//! using an efficient lock-free token bucket implementation.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Global speed limiter shared across all downloads
///
/// Uses a token bucket algorithm for efficient, lock-free bandwidth limiting.
/// All concurrent downloads share the same bucket, naturally distributing
/// bandwidth based on demand.
///
/// # Algorithm
///
/// - Tokens represent bytes that can be transferred
/// - Tokens refill at a constant rate (limit_bps)
/// - Downloads acquire tokens before transferring data
/// - If insufficient tokens, download waits until refill
///
/// # Implementation
///
/// Uses AtomicU64 for lock-free token tracking:
/// - `limit_bps`: Speed limit in bytes per second (0 = unlimited)
/// - `tokens`: Available tokens (bytes that can be transferred now)
/// - `last_refill`: Timestamp of last token refill (nanoseconds since epoch)
#[derive(Clone)]
pub struct SpeedLimiter {
    /// Speed limit in bytes per second (0 = unlimited)
    limit_bps: Arc<AtomicU64>,
    /// Available tokens (current bucket capacity in bytes)
    tokens: Arc<AtomicU64>,
    /// Last refill timestamp (nanoseconds since arbitrary epoch)
    last_refill: Arc<AtomicU64>,
}

impl SpeedLimiter {
    /// Create a new SpeedLimiter with the specified limit
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - Speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```
    /// use usenet_dl::speed_limiter::SpeedLimiter;
    ///
    /// // 10 MB/s limit
    /// let limiter = SpeedLimiter::new(Some(10 * 1024 * 1024));
    ///
    /// // Unlimited
    /// let unlimited = SpeedLimiter::new(None);
    /// ```
    #[must_use]
    pub fn new(limit_bps: Option<u64>) -> Self {
        let limit = limit_bps.unwrap_or(0);
        let now = Self::now_nanos();

        Self {
            limit_bps: Arc::new(AtomicU64::new(limit)),
            tokens: Arc::new(AtomicU64::new(limit)),
            last_refill: Arc::new(AtomicU64::new(now)),
        }
    }

    /// Set a new speed limit
    ///
    /// This takes effect immediately. If increasing the limit, tokens are
    /// refilled to the new capacity. If decreasing, excess tokens remain
    /// until consumed.
    ///
    /// # Arguments
    ///
    /// * `limit_bps` - New speed limit in bytes per second (None = unlimited)
    ///
    /// # Examples
    ///
    /// ```
    /// use usenet_dl::speed_limiter::SpeedLimiter;
    ///
    /// let limiter = SpeedLimiter::new(Some(5_000_000)); // 5 MB/s
    ///
    /// // Increase to 10 MB/s
    /// limiter.set_limit(Some(10_000_000));
    ///
    /// // Remove limit
    /// limiter.set_limit(None);
    /// ```
    pub fn set_limit(&self, limit_bps: Option<u64>) {
        let new_limit = limit_bps.unwrap_or(0);
        let old_limit = self.limit_bps.swap(new_limit, Ordering::SeqCst);

        // If increasing limit, add extra tokens to bucket
        if new_limit > old_limit {
            let extra_tokens = new_limit - old_limit;
            self.tokens.fetch_add(extra_tokens, Ordering::SeqCst);
        }
    }

    /// Get the current speed limit
    ///
    /// Returns None if unlimited, otherwise the limit in bytes per second.
    pub fn get_limit(&self) -> Option<u64> {
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 { None } else { Some(limit) }
    }

    /// Acquire permission to transfer the specified number of bytes
    ///
    /// This method blocks until sufficient tokens are available. For unlimited
    /// speed (limit = 0), this returns immediately.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Number of bytes to transfer
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use usenet_dl::speed_limiter::SpeedLimiter;
    ///
    /// # async fn example() {
    /// let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s
    ///
    /// // Before downloading each chunk
    /// let chunk_size = 8192;
    /// limiter.acquire(chunk_size).await;
    /// // ... perform download ...
    /// # }
    /// ```
    pub async fn acquire(&self, bytes: u64) {
        // Fast path: unlimited speed
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 {
            return;
        }

        loop {
            // Refill tokens based on elapsed time
            self.refill_tokens();

            // Try to acquire tokens
            let current_tokens = self.tokens.load(Ordering::SeqCst);
            if current_tokens >= bytes {
                // Sufficient tokens available - try to consume them atomically
                let new_tokens = current_tokens - bytes;
                if self
                    .tokens
                    .compare_exchange(
                        current_tokens,
                        new_tokens,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    // Successfully acquired tokens
                    return;
                }
                // CAS failed, another download consumed tokens - retry
                continue;
            }

            // Insufficient tokens - calculate wait time
            let deficit = bytes.saturating_sub(current_tokens);
            let wait_ms = (deficit as f64 / limit as f64 * 1000.0) as u64;

            // Sleep for a short time to allow token refill
            // Use max of 10ms to avoid busy-waiting
            tokio::time::sleep(Duration::from_millis(wait_ms.max(10))).await;
        }
    }

    /// Refill tokens based on elapsed time since last refill
    ///
    /// This is called automatically by acquire(), but can be called manually
    /// for testing or monitoring purposes.
    fn refill_tokens(&self) {
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 {
            return; // Unlimited
        }

        let now = Self::now_nanos();
        let last = self.last_refill.load(Ordering::SeqCst);

        // Calculate elapsed time in seconds
        let elapsed_nanos = now.saturating_sub(last);
        let elapsed_secs = elapsed_nanos as f64 / 1_000_000_000.0;

        // Calculate tokens to add (bytes per second * seconds elapsed)
        let tokens_to_add = (limit as f64 * elapsed_secs) as u64;

        if tokens_to_add > 0 {
            // Try to update last_refill timestamp atomically
            if self
                .last_refill
                .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                // Add tokens, but cap at limit (bucket capacity)
                let current_tokens = self.tokens.load(Ordering::SeqCst);
                let new_tokens = (current_tokens + tokens_to_add).min(limit);
                self.tokens.store(new_tokens, Ordering::SeqCst);
            }
        }
    }

    /// Get current monotonic time in nanoseconds
    ///
    /// Uses a monotonic clock that is not affected by system time changes.
    /// The epoch is arbitrary but consistent within a process lifetime.
    fn now_nanos() -> u64 {
        // Use Instant for monotonic time measurement
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        start.elapsed().as_nanos() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speed_limiter_new_unlimited() {
        let limiter = SpeedLimiter::new(None);
        assert_eq!(limiter.get_limit(), None);
        assert_eq!(limiter.limit_bps.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_speed_limiter_new_with_limit() {
        let limit = 10_000_000; // 10 MB/s
        let limiter = SpeedLimiter::new(Some(limit));
        assert_eq!(limiter.get_limit(), Some(limit));
        assert_eq!(limiter.limit_bps.load(Ordering::Relaxed), limit);
        // Initial tokens should equal limit
        assert_eq!(limiter.tokens.load(Ordering::Relaxed), limit);
    }

    #[test]
    fn test_set_limit_increase() {
        let limiter = SpeedLimiter::new(Some(5_000_000)); // 5 MB/s
        let old_tokens = limiter.tokens.load(Ordering::Relaxed);

        limiter.set_limit(Some(10_000_000)); // 10 MB/s

        assert_eq!(limiter.get_limit(), Some(10_000_000));
        // Tokens should increase by 5 MB
        let new_tokens = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(new_tokens, old_tokens + 5_000_000);
    }

    #[test]
    fn test_set_limit_decrease() {
        let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s
        let old_tokens = limiter.tokens.load(Ordering::Relaxed);

        limiter.set_limit(Some(5_000_000)); // 5 MB/s

        assert_eq!(limiter.get_limit(), Some(5_000_000));
        // Tokens should remain the same (not decreased)
        let new_tokens = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(new_tokens, old_tokens);
    }

    #[test]
    fn test_set_limit_to_unlimited() {
        let limiter = SpeedLimiter::new(Some(10_000_000));
        limiter.set_limit(None);
        assert_eq!(limiter.get_limit(), None);
        assert_eq!(limiter.limit_bps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_acquire_unlimited() {
        let limiter = SpeedLimiter::new(None);

        // Should return immediately for unlimited speed
        let start = Instant::now();
        limiter.acquire(1_000_000).await;
        let elapsed = start.elapsed();

        // Should complete in under 10ms
        assert!(elapsed < Duration::from_millis(10));
    }

    #[tokio::test]
    async fn test_acquire_with_sufficient_tokens() {
        let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s

        // Acquire less than available tokens
        let start = Instant::now();
        limiter.acquire(1_000_000).await; // 1 MB
        let elapsed = start.elapsed();

        // Should complete immediately (tokens available)
        assert!(elapsed < Duration::from_millis(50));

        // Check tokens were consumed
        let remaining = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(remaining, 9_000_000);
    }

    #[tokio::test]
    async fn test_acquire_multiple_small_chunks() {
        let limiter = SpeedLimiter::new(Some(10_000_000)); // 10 MB/s

        // Acquire multiple small chunks
        for _ in 0..10 {
            limiter.acquire(100_000).await; // 100 KB each
        }

        // Total: 1 MB consumed
        let remaining = limiter.tokens.load(Ordering::Relaxed);
        assert!(
            (8_999_000..=9_001_000).contains(&remaining),
            "expected ~9_000_000 tokens remaining, got {remaining}"
        );
    }

    #[tokio::test]
    async fn test_token_refill() {
        let limiter = SpeedLimiter::new(Some(1_000_000)); // 1 MB/s

        // Consume all tokens
        limiter.acquire(1_000_000).await;
        let tokens_after_acquire = limiter.tokens.load(Ordering::Relaxed);
        assert_eq!(tokens_after_acquire, 0);

        // Wait for refill (1 second should refill all tokens)
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Force refill by attempting to acquire
        limiter.refill_tokens();
        let tokens_after_refill = limiter.tokens.load(Ordering::Relaxed);

        // Should have refilled close to full capacity (within 10% tolerance)
        assert!(
            tokens_after_refill >= 900_000,
            "tokens_after_refill = {}",
            tokens_after_refill
        );
    }

    #[tokio::test]
    async fn test_concurrent_acquires() {
        let limiter = Arc::new(SpeedLimiter::new(Some(10_000_000))); // 10 MB/s

        // Spawn 5 concurrent downloads, each trying to acquire 3 MB
        let mut handles = vec![];
        for _ in 0..5 {
            let limiter_clone = Arc::clone(&limiter);
            let handle = tokio::spawn(async move {
                limiter_clone.acquire(3_000_000).await;
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Total acquired: 15 MB, but capacity is 10 MB
        // Some downloads should have waited for token refill
        // This test mainly verifies no deadlocks/panics occur
    }

    #[test]
    fn test_now_nanos_monotonic() {
        let t1 = SpeedLimiter::now_nanos();
        std::thread::sleep(Duration::from_millis(10));
        let t2 = SpeedLimiter::now_nanos();

        // Time should always increase
        assert!(t2 > t1);
        // Difference should be roughly 10ms (10_000_000 nanoseconds)
        let diff = t2 - t1;
        assert!(diff >= 10_000_000, "diff = {}", diff);
    }

    #[tokio::test]
    async fn test_speed_limiting_enforced() {
        let limiter = SpeedLimiter::new(Some(1_000_000)); // 1 MB/s

        let start = Instant::now();

        // Try to acquire 2 MB (should take ~2 seconds)
        limiter.acquire(2_000_000).await;

        let elapsed = start.elapsed();

        // Should take at least 1 second (tokens refill at 1 MB/s)
        // Allow some tolerance for test timing
        assert!(
            elapsed >= Duration::from_millis(800),
            "elapsed = {:?} (expected >= 800ms)",
            elapsed
        );
    }
}
