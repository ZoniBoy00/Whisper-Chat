//! Per-IP token-bucket rate limiting.
//!
//! Envelope throughput, pre-key traffic, profile mutations, presence queries
//! and group operations are all throttled per source IP. The generic limiter
//! keys separate buckets (`prekey:<ip>`, `presence:<ip>`, `group:<ip>`) so one
//! storm cannot starve the others; the profile limiter is a second instance
//! with its own (smaller) budget. Both are built from environment overrides in
//! the relay core.
//!
//! BUCKET GARBAGE COLLECTION
//! -------------------------
//! The bucket map is keyed by client IP, which is unbounded by nature, so
//! idle buckets are swept periodically: a bucket unused for
//! [`GC_IDLE_TIMEOUT`] is dropped and its tokens forgotten. Sweeping is
//! safe — a dropped bucket is recreated at full burst on the next `try_take`
//! — and keeps the map from growing without limit under IP rotation or
//! spoofed-source floods. Sweeps run at most once per [`GC_INTERVAL`], so the
//! hot path only pays an atomic load.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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

/// A bucket idle for this long is swept (dropped) by the next GC pass.
const GC_IDLE_TIMEOUT: Duration = Duration::from_secs(600); // 10 minutes

/// Minimum time between GC sweeps, so a busy relay does not rescan the whole
/// map on every token draw.
const GC_INTERVAL: Duration = Duration::from_secs(60);

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
    /// Wall-clock millis of the last GC sweep (0 = never yet, so the first
    /// `try_take` always sweeps). Atomic so `try_take` checks it without
    /// locking the bucket map.
    last_gc_ms: AtomicU64,
    /// Idle timeout after which a bucket is eligible for sweeping.
    gc_idle: Duration,
    /// Minimum time between sweeps.
    gc_interval: Duration,
    /// Cumulative token-bucket rejections (exposed via /metrics).
    rejected: AtomicU64,
}

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: std::time::Instant,
}

impl RateLimiter {
    /// Create a limiter with an explicit bucket size and refill rate.
    pub(crate) fn new(burst: f64, refill_per_sec: f64) -> Self {
        Self::with_gc(burst, refill_per_sec, GC_IDLE_TIMEOUT, GC_INTERVAL)
    }

    /// Create a limiter with explicit GC tuning (tests use short timeouts so
    /// sweeps are observable without long sleeps).
    fn with_gc(burst: f64, refill_per_sec: f64, gc_idle: Duration, gc_interval: Duration) -> Self {
        Self {
            buckets: std::sync::Mutex::new(HashMap::new()),
            burst,
            refill_per_sec,
            last_gc_ms: AtomicU64::new(0),
            gc_idle,
            gc_interval,
            rejected: AtomicU64::new(0),
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
        self.gc_if_due();
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
            self.rejected.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    /// Cumulative number of rejections (rate-limit hits) across all keys.
    pub(crate) fn rejected_count(&self) -> u64 {
        self.rejected.load(Ordering::Relaxed)
    }

    /// Run a GC sweep if the minimum interval has elapsed; returns the number
    /// of buckets swept (0 when the interval has not elapsed).
    fn gc_if_due(&self) -> usize {
        let now_ms = wall_clock_ms();
        let last = self.last_gc_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) < self.gc_interval.as_millis() as u64 {
            return 0;
        }
        // Reserve the sweep slot; a concurrent caller loses the race and
        // skips, so the map is locked at most once per interval.
        if self
            .last_gc_ms
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return 0;
        }
        let mut buckets = self.buckets.lock().unwrap();
        let swept = sweep(&mut buckets, Instant::now(), self.gc_idle);
        if swept > 0 {
            tracing::debug!(swept, "rate-limit buckets swept");
        }
        swept
    }
}

/// Drop every bucket whose `last` activity predates `idle`. Returns the
/// number of buckets removed. Pure, so it is unit-testable without a limiter
/// instance.
fn sweep(buckets: &mut HashMap<String, Bucket>, now: Instant, idle: Duration) -> usize {
    let before = buckets.len();
    buckets.retain(|_, bucket| now.duration_since(bucket.last) < idle);
    before - buckets.len()
}

/// Current wall-clock time in milliseconds since the Unix epoch.
fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

    #[test]
    fn sweep_removes_only_idle_buckets() {
        let mut buckets = HashMap::new();
        let now = Instant::now();
        buckets.insert(
            "stale".into(),
            Bucket {
                tokens: 0.0,
                last: now - Duration::from_secs(600),
            },
        );
        buckets.insert(
            "fresh".into(),
            Bucket {
                tokens: 1.0,
                last: now,
            },
        );

        let swept = sweep(&mut buckets, now, Duration::from_secs(300));

        assert_eq!(swept, 1, "only the idle bucket is swept");
        assert!(buckets.contains_key("fresh"));
        assert!(!buckets.contains_key("stale"));
    }

    #[test]
    fn sweep_empty_map_returns_zero() {
        let mut buckets = HashMap::new();
        assert_eq!(
            sweep(&mut buckets, Instant::now(), Duration::from_secs(1)),
            0
        );
    }

    #[test]
    fn dropped_bucket_restarts_with_full_burst() {
        // 1 ms idle timeout + 1 ms sweep interval: the bucket is swept and
        // recreated between calls, so the exhausted IP gets a fresh burst.
        let l = RateLimiter::with_gc(2.0, 0.0, Duration::from_millis(1), Duration::from_millis(1));
        assert!(l.try_take("ip-a"));
        assert!(l.try_take("ip-a"));
        assert!(!l.try_take("ip-a"), "burst exhausted");
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            l.try_take("ip-a"),
            "idle bucket was swept, so a fresh full-burst bucket is created"
        );
    }

    #[test]
    fn gc_respects_minimum_interval() {
        // Long interval (1h): the first call sweeps (last_gc starts at 0),
        // the immediate second call must not sweep again. The bucket is
        // seeded directly — `try_take` would already consume the first sweep
        // slot while the fresh bucket is not yet idle.
        let l = RateLimiter::with_gc(
            2.0,
            0.0,
            Duration::from_millis(1),
            Duration::from_secs(3600),
        );
        l.buckets.lock().unwrap().insert(
            "ip-a".into(),
            Bucket {
                tokens: 0.0,
                last: Instant::now() - Duration::from_millis(10),
            },
        );
        assert_eq!(l.gc_if_due(), 1, "idle bucket swept on first due pass");
        assert_eq!(
            l.gc_if_due(),
            0,
            "second pass is inside the minimum interval and must skip"
        );
    }
}
