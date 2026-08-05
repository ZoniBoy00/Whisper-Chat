//! Per-IP token-bucket rate limiting.
//!
//! Envelope throughput, pre-key traffic, profile mutations, presence queries
//! and group operations are all throttled per source IP. The generic limiter
//! keys separate buckets (`prekey:<ip>`, `presence:<ip>`, `group:<ip>`) so one
//! storm cannot starve the others; the profile limiter is a second instance
//! with its own (smaller) budget. Both are built from environment overrides in
//! the relay core.

use std::collections::HashMap;

/// Default per-IP token bucket: burst of 60 envelopes, refilled at 1/sec
/// (~60 envelopes per minute).
const DEFAULT_RATE_BURST: f64 = 60.0;
const DEFAULT_RATE_REFILL_PER_SEC: f64 = 1.0;

/// Default per-IP profile token bucket: 30 mutations, refilled at 30/hour.
/// Registration, search and profile lookups all draw from it. Generous enough
/// for normal use (avatar/display-name tweaks) while still limiting username
/// squatting spam.
const DEFAULT_PROFILE_RATE_BURST: f64 = 30.0;
const DEFAULT_PROFILE_RATE_REFILL_PER_SEC: f64 = 30.0 / 3600.0;

/// Default per-IP contact token bucket: 20 friend-request/contact mutations,
/// refilled at 20/hour. Tight enough to block friend-request spam — the
/// exact vector the contact system exists to stop — while a normal user adds
/// friends as they meet them.
const DEFAULT_CONTACTS_RATE_BURST: f64 = 20.0;
const DEFAULT_CONTACTS_RATE_REFILL_PER_SEC: f64 = 20.0 / 3600.0;

/// Per-IP token bucket. Each accepted envelope consumes one token; tokens are
/// refilled continuously up to the burst capacity.
///
/// Pre-key traffic shares the same limiter but keys its buckets as
/// `prekey:<ip>`, so a pre-key storm can neither starve envelope routing nor
/// leak into the envelope budget (and vice versa).
pub(crate) struct RateLimiter {
    buckets: std::sync::Mutex<HashMap<String, Bucket>>,
    burst: f64,
    refill_per_sec: f64,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: std::time::Instant,
}

impl RateLimiter {
    /// Create a limiter with an explicit bucket size and refill rate.
    pub(crate) fn new(burst: f64, refill_per_sec: f64) -> Self {
        Self {
            buckets: std::sync::Mutex::new(HashMap::new()),
            burst,
            refill_per_sec,
        }
    }

    /// Build a limiter from environment overrides:
    /// `WHISPER_RATE_BURST` (max burst) and `WHISPER_RATE_REFILL` (tokens/sec).
    pub(crate) fn from_env() -> Self {
        let burst = std::env::var("WHISPER_RATE_BURST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(DEFAULT_RATE_BURST);
        let refill = std::env::var("WHISPER_RATE_REFILL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(DEFAULT_RATE_REFILL_PER_SEC);
        Self::new(burst, refill)
    }

    /// Build the profile limiter (see [`DEFAULT_PROFILE_RATE_BURST`]).
    ///
    /// Burst/refill are overridable via `WHISPER_PROFILE_RATE_BURST` and
    /// `WHISPER_PROFILE_RATE_REFILL`; when those are unset the generic
    /// `WHISPER_RATE_BURST` / `WHISPER_RATE_REFILL` overrides apply, so a
    /// single smoke-test configuration can bound every bucket.
    pub(crate) fn from_profile_env() -> Self {
        let burst = std::env::var("WHISPER_PROFILE_RATE_BURST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                std::env::var("WHISPER_RATE_BURST")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .unwrap_or(DEFAULT_PROFILE_RATE_BURST);
        let refill = std::env::var("WHISPER_PROFILE_RATE_REFILL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                std::env::var("WHISPER_RATE_REFILL")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .unwrap_or(DEFAULT_PROFILE_RATE_REFILL_PER_SEC);
        Self::new(burst, refill)
    }

    /// Build the contacts limiter (see [`DEFAULT_CONTACTS_RATE_BURST`]).
    ///
    /// Burst/refill are overridable via `WHISPER_CONTACTS_RATE_BURST` and
    /// `WHISPER_CONTACTS_RATE_REFILL`; when those are unset the generic
    /// `WHISPER_RATE_BURST` / `WHISPER_RATE_REFILL` overrides apply, so a
    /// single smoke-test configuration can bound every bucket.
    pub(crate) fn from_contacts_env() -> Self {
        let burst = std::env::var("WHISPER_CONTACTS_RATE_BURST")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                std::env::var("WHISPER_RATE_BURST")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .unwrap_or(DEFAULT_CONTACTS_RATE_BURST);
        let refill = std::env::var("WHISPER_CONTACTS_RATE_REFILL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .or_else(|| {
                std::env::var("WHISPER_RATE_REFILL")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .unwrap_or(DEFAULT_CONTACTS_RATE_REFILL_PER_SEC);
        Self::new(burst, refill)
    }

    /// Try to consume one token for `key`. Returns `false` when the bucket is
    /// exhausted (rate limit hit).
    pub(crate) fn try_take(&self, key: &str) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let now = std::time::Instant::now();
        let bucket = buckets.entry(key.to_string()).or_insert_with(|| Bucket {
            tokens: self.burst,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_allows_burst_then_rejects() {
        let l = RateLimiter::new(2.0, 0.0);
        assert!(l.try_take("ip-a"));
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"));
    }

    #[test]
    fn limiter_is_per_key() {
        let l = RateLimiter::new(1.0, 0.0);
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"), "ip-a must be exhausted");
        assert!(l.try_take("ip-b"), "ip-b has its own bucket");
    }

    #[test]
    fn limiter_refills_over_time() {
        let l = RateLimiter::new(2.0, 1000.0); // 1000 tokens/sec
        assert!(l.try_take("ip-a"));
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(l.try_take("ip-a"), "tokens must refill over time");
    }
}
