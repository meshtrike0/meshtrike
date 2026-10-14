//! Countermeasures (sections 11, 12): adaptive transmit power, interference
//! detection, Listen-only connection policy, forwarding delays, cover slots.
#![forbid(unsafe_code)]

use mesh_constants::*;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TxPower {
    UltraLow = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

/// Adaptive BLE advertising power (12.1 item 3).
#[derive(Debug, Clone)]
pub struct TxPowerController {
    pub level: TxPower,
    last_connect_ms: u64,
    last_step_ms: u64,
    last_down_check_ms: u64,
}

impl TxPowerController {
    pub fn new(now_ms: u64) -> Self {
        Self {
            level: TxPower::UltraLow,
            last_connect_ms: now_ms,
            last_step_ms: now_ms,
            last_down_check_ms: now_ms,
        }
    }

    pub fn on_neighbour_connected(&mut self, now_ms: u64) {
        self.last_connect_ms = now_ms;
    }

    /// Re-evaluate; `strong_neighbours` = neighbours heard above −70 dBm.
    /// Returns the new level when it changed.
    pub fn tick(
        &mut self,
        now_ms: u64,
        public_mode: bool,
        strong_neighbours: usize,
    ) -> Option<TxPower> {
        let before = self.level;
        let max = if public_mode {
            TxPower::High
        } else {
            TxPower::Medium
        };
        if self.level > max {
            self.level = max;
        }
        if now_ms.saturating_sub(self.last_connect_ms) >= TX_STEP_UP_MS
            && now_ms.saturating_sub(self.last_step_ms) >= TX_STEP_UP_MS
            && self.level < max
        {
            self.level = match self.level {
                TxPower::UltraLow => TxPower::Low,
                TxPower::Low => TxPower::Medium,
                _ => TxPower::High,
            };
            self.last_step_ms = now_ms;
        }
        if now_ms.saturating_sub(self.last_down_check_ms) >= TX_STEP_DOWN_MS {
            self.last_down_check_ms = now_ms;
            if strong_neighbours >= TX_STEP_DOWN_NEIGHBOURS && self.level > TxPower::UltraLow {
                self.level = match self.level {
                    TxPower::High => TxPower::Medium,
                    TxPower::Medium => TxPower::Low,
                    _ => TxPower::UltraLow,
                };
                self.last_step_ms = now_ms;
            }
        }
        (self.level != before).then_some(self.level)
    }
}

/// Interference detector (12.2).
#[derive(Debug, Clone, Default)]
pub struct InterferenceDetector {
    /// (time, adverts seen in the last interval)
    samples: VecDeque<(u64, u32)>,
    /// (time, attempts, failures)
    connects: VecDeque<(u64, u32, u32)>,
    stationary_since: Option<u64>,
    low_run: usize,
    clear_since: Option<u64>,
    pub suspected: bool,
}

impl InterferenceDetector {
    fn baseline(&self) -> u32 {
        let mut v: Vec<u32> = self.samples.iter().map(|s| s.1).collect();
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        v[v.len() / 2]
    }

    /// Feed one 10-second sample. Returns Some(new state) when it changes.
    pub fn sample(
        &mut self,
        now_ms: u64,
        adverts: u32,
        connect_attempts: u32,
        connect_failures: u32,
        stationary: bool,
    ) -> Option<bool> {
        let baseline = self.baseline();
        while self
            .samples
            .front()
            .is_some_and(|s| now_ms.saturating_sub(s.0) > INTERFERENCE_BASELINE_MS)
        {
            self.samples.pop_front();
        }
        while self
            .connects
            .front()
            .is_some_and(|s| now_ms.saturating_sub(s.0) > 120_000)
        {
            self.connects.pop_front();
        }
        self.connects
            .push_back((now_ms, connect_attempts, connect_failures));
        if stationary {
            self.stationary_since.get_or_insert(now_ms);
        } else {
            self.stationary_since = None;
        }
        let stationary_60s = self
            .stationary_since
            .is_some_and(|t| now_ms.saturating_sub(t) >= 60_000);
        let low = baseline >= INTERFERENCE_MIN_BASELINE
            && adverts * 100 < baseline * INTERFERENCE_LOW_PERCENT;
        self.low_run = if low && stationary_60s {
            self.low_run + 1
        } else {
            0
        };
        let (att, fail) = self
            .connects
            .iter()
            .fold((0, 0), |a, c| (a.0 + c.1, a.1 + c.2));
        let connect_trigger = att >= INTERFERENCE_CONNECT_MIN
            && fail * 100 >= att * INTERFERENCE_CONNECT_FAIL_PERCENT;
        let before = self.suspected;
        if !self.suspected {
            if self.low_run >= INTERFERENCE_LOW_SAMPLES || connect_trigger {
                self.suspected = true;
                self.clear_since = None;
            }
            // Only learn the baseline from un-jammed samples.
            self.samples.push_back((now_ms, adverts));
        } else {
            let recovered = baseline > 0
                && adverts * 100 > baseline * INTERFERENCE_CLEAR_PERCENT
                && !connect_trigger;
            if recovered {
                let since = *self.clear_since.get_or_insert(now_ms);
                if now_ms.saturating_sub(since) >= INTERFERENCE_CLEAR_MS {
                    self.suspected = false;
                    self.low_run = 0;
                    self.samples.push_back((now_ms, adverts));
                }
            } else {
                self.clear_since = None;
            }
        }
        (before != self.suspected).then_some(self.suspected)
    }
}

/// Listen-only connection policy (12.1 item 2).
#[derive(Debug, Clone)]
pub struct ListenOnlyPolicy {
    last_summary_ms: u64,
}

impl ListenOnlyPolicy {
    pub fn new(now_ms: u64) -> Self {
        Self {
            last_summary_ms: now_ms,
        }
    }
    /// Should we connect out now?
    pub fn should_connect(&mut self, now_ms: u64, has_queued: bool) -> bool {
        if has_queued || now_ms.saturating_sub(self.last_summary_ms) >= LISTEN_ONLY_SUMMARY_MS {
            self.last_summary_ms = now_ms;
            true
        } else {
            false
        }
    }
    /// Connections opened in Listen-only mode close after 30 s.
    pub fn must_close(opened_ms: u64, now_ms: u64) -> bool {
        now_ms.saturating_sub(opened_ms) >= LISTEN_ONLY_CONNECTION_MS
    }
}

/// Random forwarding delay (section 11) from a uniform draw `r` in [0, 1).
pub fn forwarding_delay_ms(high_risk: bool, r: f64) -> u64 {
    let (lo, hi) = if high_risk {
        FORWARD_DELAY_HIGH_RISK_MS
    } else {
        FORWARD_DELAY_MS
    };
    lo + ((hi - lo) as f64 * r) as u64
}

/// Delayed-send options (12.1 item 4); `None` = hold until released.
pub fn delayed_send_release(option: u8, now_ms: u64) -> Option<Option<u64>> {
    match option {
        0 => Some(Some(now_ms)),
        1 => Some(Some(now_ms + 10 * 60_000)),
        2 => Some(Some(now_ms + 30 * 60_000)),
        3 => Some(Some(now_ms + 2 * 3_600_000)),
        4 => Some(None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_power_steps_up_and_down() {
        let mut t = TxPowerController::new(0);
        assert_eq!(t.tick(59_000, false, 0), None);
        assert_eq!(t.tick(60_000, false, 0), Some(TxPower::Low));
        assert_eq!(t.tick(120_000, false, 0), Some(TxPower::Medium));
        assert_eq!(t.tick(180_000, false, 0), None); // High only in Public
        t.on_neighbour_connected(200_000);
        assert_eq!(t.tick(300_000, false, 3), Some(TxPower::Low));
    }

    #[test]
    fn interference_detects_and_clears() {
        let mut d = InterferenceDetector::default();
        let mut t = 0;
        for _ in 0..60 {
            t += 10_000;
            assert_eq!(d.sample(t, 40, 0, 0, true), None);
        }
        let mut fired = None;
        for _ in 0..8 {
            t += 10_000;
            if let Some(s) = d.sample(t, 2, 0, 0, true) {
                fired = Some(s);
            }
        }
        assert_eq!(fired, Some(true));
        let mut cleared = None;
        for _ in 0..8 {
            t += 10_000;
            if let Some(s) = d.sample(t, 40, 0, 0, true) {
                cleared = Some(s);
            }
        }
        assert_eq!(cleared, Some(false));
    }

    #[test]
    fn interference_needs_stationary() {
        let mut d = InterferenceDetector::default();
        let mut t = 0;
        for _ in 0..60 {
            t += 10_000;
            d.sample(t, 40, 0, 0, false);
        }
        for _ in 0..10 {
            t += 10_000;
            assert_eq!(d.sample(t, 1, 0, 0, false), None);
        }
    }

    #[test]
    fn interference_connect_failures() {
        let mut d = InterferenceDetector::default();
        assert_eq!(d.sample(10_000, 5, 6, 5, false), None);
        assert_eq!(d.sample(20_000, 5, 6, 5, false), Some(true));
    }

    #[test]
    fn delays() {
        assert_eq!(forwarding_delay_ms(false, 0.0), 50);
        assert!(forwarding_delay_ms(true, 0.999) <= 5000);
        assert_eq!(delayed_send_release(4, 0), Some(None));
    }
}
