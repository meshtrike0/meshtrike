//! Abuse resistance (section 8): per-neighbour rate limits, malformed-frame
//! blocking, and the broadcast proof-of-work difficulty tracker.
#![forbid(unsafe_code)]

use mesh_constants::*;
use std::collections::VecDeque;

/// Token bucket over frames and bytes per neighbour.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    frames: f64,
    bytes: f64,
    last_ms: u64,
    rate_frames: f64,
    rate_bytes: f64,
    max_frames: f64,
    max_bytes: f64,
}

impl RateLimiter {
    pub fn new(now_ms: u64) -> Self {
        Self::with_limits(now_ms, RATE_LIMIT_FRAMES_PER_SEC, RATE_LIMIT_BYTES_PER_SEC)
    }

    /// Custom budget (Wi-Fi peer-to-peer links carry far more than BLE).
    /// Refills at the given rate; holds `RATE_LIMIT_BURST_SECS` of it.
    pub fn with_limits(now_ms: u64, frames_per_sec: u32, bytes_per_sec: u32) -> Self {
        let burst = RATE_LIMIT_BURST_SECS as f64;
        Self {
            frames: frames_per_sec as f64 * burst,
            bytes: bytes_per_sec as f64 * burst,
            last_ms: now_ms,
            rate_frames: frames_per_sec as f64,
            rate_bytes: bytes_per_sec as f64,
            max_frames: frames_per_sec as f64 * burst,
            max_bytes: bytes_per_sec as f64 * burst,
        }
    }

    /// Returns false when the neighbour exceeds its budget. The caller
    /// closes the link; going too fast is not proof of malice, so it does
    /// not count towards a block.
    pub fn allow(&mut self, len: usize, now_ms: u64) -> bool {
        let dt = now_ms.saturating_sub(self.last_ms) as f64 / 1000.0;
        self.last_ms = now_ms;
        self.frames = (self.frames + dt * self.rate_frames).min(self.max_frames);
        self.bytes = (self.bytes + dt * self.rate_bytes).min(self.max_bytes);
        if self.frames >= 1.0 && self.bytes >= len as f64 {
            self.frames -= 1.0;
            self.bytes -= len as f64;
            true
        } else {
            false
        }
    }
}

/// Blocks a neighbour for 10 minutes after 5 malformed frames within 1 minute.
#[derive(Debug, Clone, Default)]
pub struct MalformedTracker {
    hits: VecDeque<u64>,
    blocked_until: u64,
}

impl MalformedTracker {
    /// Record a malformed frame; returns true if the neighbour is now blocked.
    pub fn record(&mut self, now_ms: u64) -> bool {
        while self
            .hits
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= MALFORMED_WINDOW_MS)
        {
            self.hits.pop_front();
        }
        self.hits.push_back(now_ms);
        if self.hits.len() as u32 >= MALFORMED_LIMIT {
            self.blocked_until = now_ms + MALFORMED_BLOCK_MS;
            self.hits.clear();
        }
        self.is_blocked(now_ms)
    }

    pub fn is_blocked(&self, now_ms: u64) -> bool {
        now_ms < self.blocked_until
    }
}

/// Observed broadcast volume over the last minute → required PoW bits.
#[derive(Debug, Clone, Default)]
pub struct BroadcastRate {
    seen: VecDeque<u64>,
}

impl BroadcastRate {
    pub fn record(&mut self, now_ms: u64) {
        self.prune(now_ms);
        self.seen.push_back(now_ms);
    }

    fn prune(&mut self, now_ms: u64) {
        while self
            .seen
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= 60_000)
        {
            self.seen.pop_front();
        }
    }

    pub fn per_minute(&mut self, now_ms: u64) -> u32 {
        self.prune(now_ms);
        self.seen.len() as u32
    }

    pub fn required_bits(&mut self, now_ms: u64) -> u32 {
        mesh_crypto::pow::required_bits(self.per_minute(now_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_frames() {
        let mut r = RateLimiter::new(0);
        let burst = (RATE_LIMIT_FRAMES_PER_SEC * RATE_LIMIT_BURST_SECS) as usize;
        let allowed = (0..burst + 50).filter(|_| r.allow(10, 0)).count();
        assert_eq!(allowed, burst);
        assert!(r.allow(10, 100)); // 2 frames refilled after 100 ms
    }

    #[test]
    fn rate_limit_bytes() {
        let mut r = RateLimiter::new(0);
        let burst = (RATE_LIMIT_BYTES_PER_SEC * RATE_LIMIT_BURST_SECS) as usize;
        assert!(r.allow(burst, 0));
        assert!(!r.allow(1, 0));
    }

    #[test]
    fn burst_after_a_stall_is_allowed() {
        // A sender paced at 18 frames/s whose frames sat in the radio queue
        // for 5 s arrive all at once.
        let mut r = RateLimiter::new(0);
        assert!((0..90).all(|i| r.allow(500, 5_000 + i)));
    }

    #[test]
    fn malformed_blocks_after_five() {
        let mut m = MalformedTracker::default();
        for i in 0..4 {
            assert!(!m.record(i * 1000));
        }
        assert!(m.record(5000));
        assert!(m.is_blocked(5000 + MALFORMED_BLOCK_MS - 1));
        assert!(!m.is_blocked(5000 + MALFORMED_BLOCK_MS));
    }

    #[test]
    fn spaced_malformed_does_not_block() {
        let mut m = MalformedTracker::default();
        for i in 0..10 {
            assert!(!m.record(i * 20_000));
        }
    }

    #[test]
    fn broadcast_rate_raises_difficulty() {
        let mut b = BroadcastRate::default();
        for i in 0..70 {
            b.record(i * 100);
        }
        assert_eq!(b.required_bits(7000), 24);
        assert_eq!(b.required_bits(200_000), 20);
    }
}
