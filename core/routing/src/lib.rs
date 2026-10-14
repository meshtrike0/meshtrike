//! Store-and-forward relay logic (section 8): envelope store with fair-share
//! eviction and per-tag quotas, ack verification, summary vectors, the
//! seen-message filter, and the broadcast rebroadcast decision.
#![forbid(unsafe_code)]

use mesh_constants::*;
use mesh_wire::{EnvType, Envelope, MessageId, SummaryEntry, Tag};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct Stored {
    pub env: Envelope,
    pub raw_len: usize,
    pub received_ms: u64,
    /// Our own outbound envelope (evicted last).
    pub mine: bool,
    /// Tag matched one of ours; decrypt after unlock.
    pub for_me: bool,
    /// Offer to neighbours in summaries (false for broadcasts we chose not
    /// to rebroadcast, for direct-only large content, and for delivered items).
    pub offer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Insert {
    Stored { evicted: Vec<MessageId> },
    Duplicate,
    Expired,
    Acked,
    QuotaExceeded,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckOutcome {
    /// Valid: copy deleted (or it was ours: delivered).
    Deleted { was_mine: bool },
    /// We do not hold the original, so we cannot verify; ignore.
    NotHeld,
    /// Preimage does not match the proof field.
    Invalid,
}

/// Seen-message filter: 10,000 IDs for 30 minutes.
#[derive(Default)]
pub struct SeenFilter {
    order: VecDeque<(u64, MessageId)>,
    set: HashSet<MessageId>,
}

impl SeenFilter {
    pub fn insert(&mut self, id: MessageId, now_ms: u64) {
        self.prune(now_ms);
        if self.set.insert(id) {
            self.order.push_back((now_ms, id));
            while self.order.len() > SEEN_FILTER_SIZE {
                let (_, old) = self.order.pop_front().unwrap();
                self.set.remove(&old);
            }
        }
    }
    pub fn contains(&mut self, id: &MessageId, now_ms: u64) -> bool {
        self.prune(now_ms);
        self.set.contains(id)
    }
    fn prune(&mut self, now_ms: u64) {
        while let Some((t, id)) = self.order.front() {
            if now_ms.saturating_sub(*t) >= SEEN_FILTER_MS {
                self.set.remove(id);
                self.order.pop_front();
            } else {
                break;
            }
        }
    }
}

pub struct RelayStore {
    items: HashMap<MessageId, Stored>,
    per_tag: HashMap<Tag, usize>,
    bytes: usize,
    /// Acked message IDs → original expiry (unix secs).
    acked: HashMap<MessageId, u32>,
    pub seen: SeenFilter,
    pub max_items: usize,
    pub max_bytes: usize,
}

impl Default for RelayStore {
    fn default() -> Self {
        Self {
            items: HashMap::new(),
            per_tag: HashMap::new(),
            bytes: 0,
            acked: HashMap::new(),
            seen: SeenFilter::default(),
            max_items: RELAY_MAX_ENVELOPES,
            max_bytes: RELAY_MAX_BYTES,
        }
    }
}

fn now_secs(now_ms: u64) -> u32 {
    (now_ms / 1000) as u32
}

impl RelayStore {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn get(&self, id: &MessageId) -> Option<&Stored> {
        self.items.get(id)
    }
    pub fn get_mut(&mut self, id: &MessageId) -> Option<&mut Stored> {
        self.items.get_mut(id)
    }
    pub fn contains(&self, id: &MessageId) -> bool {
        self.items.contains_key(id)
    }
    pub fn is_acked(&self, id: &MessageId) -> bool {
        self.acked.contains_key(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Stored> {
        self.items.values()
    }
    pub fn acked_entries(&self) -> impl Iterator<Item = (&MessageId, &u32)> {
        self.acked.iter()
    }
    pub fn restore_ack(&mut self, id: MessageId, expiry: u32) {
        self.acked.insert(id, expiry);
    }

    fn remove_internal(&mut self, id: &MessageId) -> Option<Stored> {
        let s = self.items.remove(id)?;
        self.bytes -= s.raw_len;
        if let Some(c) = self.per_tag.get_mut(&s.env.tag) {
            *c -= 1;
            if *c == 0 {
                self.per_tag.remove(&s.env.tag);
            }
        }
        Some(s)
    }

    pub fn remove(&mut self, id: &MessageId) -> Option<Stored> {
        self.remove_internal(id)
    }

    pub fn insert(&mut self, s: Stored, now_ms: u64) -> Insert {
        let id = s.env.message_id;
        if self.items.contains_key(&id) {
            return Insert::Duplicate;
        }
        if s.env.expiry <= now_secs(now_ms) {
            return Insert::Expired;
        }
        if self.acked.contains_key(&id) {
            return Insert::Acked;
        }
        let protected = s.mine || s.for_me;
        if !protected && self.per_tag.get(&s.env.tag).copied().unwrap_or(0) >= PER_TAG_QUOTA {
            return Insert::QuotaExceeded;
        }
        let mut evicted = Vec::new();
        while self.items.len() + 1 > self.max_items || self.bytes + s.raw_len > self.max_bytes {
            match self.pick_victim(now_ms) {
                Some(v) => {
                    self.remove_internal(&v);
                    evicted.push(v);
                }
                None => {
                    if protected {
                        break; // own data may exceed the limit rather than be lost
                    }
                    return Insert::Full;
                }
            }
        }
        self.bytes += s.raw_len;
        *self.per_tag.entry(s.env.tag).or_default() += 1;
        self.items.insert(id, s);
        Insert::Stored { evicted }
    }

    /// Eviction order (section 8): expired; then the oldest envelope of the
    /// recipient tag holding the most envelopes; own outbound last.
    fn pick_victim(&self, now_ms: u64) -> Option<MessageId> {
        let now = now_secs(now_ms);
        if let Some(s) = self.items.values().find(|s| s.env.expiry <= now) {
            return Some(s.env.message_id);
        }
        let candidates = self.items.values().filter(|s| !s.mine && !s.for_me);
        let mut best: Option<&Stored> = None;
        for s in candidates {
            let c = self.per_tag[&s.env.tag];
            best = match best {
                None => Some(s),
                Some(b) => {
                    let bc = self.per_tag[&b.env.tag];
                    if c > bc || (c == bc && s.received_ms < b.received_ms) {
                        Some(s)
                    } else {
                        Some(b)
                    }
                }
            };
        }
        best.map(|s| s.env.message_id).or_else(|| {
            self.items
                .values()
                .filter(|s| s.mine && !s.for_me)
                .min_by_key(|s| s.received_ms)
                .map(|s| s.env.message_id)
        })
    }

    /// Verify and apply an Ack (section 8 steps 3–4).
    pub fn apply_ack(&mut self, acked: &MessageId, secret: &[u8; 16]) -> AckOutcome {
        let Some(s) = self.items.get(acked) else {
            return if self.acked.contains_key(acked) {
                AckOutcome::Deleted { was_mine: false }
            } else {
                AckOutcome::NotHeld
            };
        };
        if s.env.typ != EnvType::Direct || mesh_crypto::ack_proof(secret) != s.env.proof {
            return AckOutcome::Invalid;
        }
        let expiry = s.env.expiry;
        let s = self.remove_internal(acked).unwrap();
        self.acked.insert(*acked, expiry);
        AckOutcome::Deleted { was_mine: s.mine }
    }

    /// Remove expired envelopes and ack records; returns removed IDs.
    pub fn expire(&mut self, now_ms: u64) -> Vec<MessageId> {
        let now = now_secs(now_ms);
        let dead: Vec<MessageId> = self
            .items
            .values()
            .filter(|s| s.env.expiry <= now)
            .map(|s| s.env.message_id)
            .collect();
        for d in &dead {
            self.remove_internal(d);
        }
        self.acked.retain(|_, e| *e > now);
        dead
    }

    /// Up to 500 summary entries: own envelopes first, then newest.
    pub fn summary(&self, now_ms: u64) -> Vec<SummaryEntry> {
        let now = now_secs(now_ms);
        let mut v: Vec<&Stored> = self
            .items
            .values()
            .filter(|s| s.offer && s.env.expiry > now && s.env.typ != EnvType::Cover)
            .collect();
        // Own envelopes oldest first (keeps Olm chain order for bursts), then
        // other envelopes newest first.
        v.sort_by(|a, b| {
            b.mine.cmp(&a.mine).then(if a.mine && b.mine {
                a.received_ms.cmp(&b.received_ms)
            } else {
                b.received_ms.cmp(&a.received_ms)
            })
        });
        v.into_iter()
            .take(SUMMARY_VECTOR_MAX)
            .map(|s| SummaryEntry {
                message_id: s.env.message_id,
                tag: s.env.tag,
                expiry: s.env.expiry,
            })
            .collect()
    }

    /// Should we request this offered envelope? (section 8 step 2)
    pub fn wants(&mut self, e: &SummaryEntry, tag_is_ours: bool, now_ms: u64) -> bool {
        if self.items.contains_key(&e.message_id)
            || self.acked.contains_key(&e.message_id)
            || e.expiry <= now_secs(now_ms)
            || self.seen.contains(&e.message_id, now_ms)
        {
            return false;
        }
        if tag_is_ours {
            return true;
        }
        self.items.len() < self.max_items
            && self.per_tag.get(&e.tag).copied().unwrap_or(0) < PER_TAG_QUOTA
    }

    pub fn tag_count(&self, tag: &Tag) -> usize {
        self.per_tag.get(tag).copied().unwrap_or(0)
    }
}

/// Broadcast flood decision (section 8): `r` uniform in [0, 100).
pub fn should_rebroadcast(neighbours: usize, r: u32) -> bool {
    r < rebroadcast_percent(neighbours)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(id: u8, tag: u8, expiry: u32, typ: EnvType) -> Envelope {
        Envelope {
            typ,
            ttl: 5,
            flags: 0,
            message_id: [id; 16],
            tag: [tag; 16],
            expiry,
            proof: [0; 8],
            payload: vec![0; 10],
        }
    }

    fn stored(e: Envelope, t: u64, mine: bool) -> Stored {
        Stored {
            raw_len: 100,
            env: e,
            received_ms: t,
            mine,
            for_me: false,
            offer: true,
        }
    }

    #[test]
    fn insert_dup_expired() {
        let mut r = RelayStore::default();
        assert_eq!(
            r.insert(stored(env(1, 1, 7200, EnvType::Direct), 0, false), 0),
            Insert::Stored { evicted: vec![] }
        );
        assert_eq!(
            r.insert(stored(env(1, 1, 7200, EnvType::Direct), 0, false), 0),
            Insert::Duplicate
        );
        assert_eq!(
            r.insert(
                stored(env(2, 1, 3600, EnvType::Direct), 0, false),
                3_600_000
            ),
            Insert::Expired
        );
    }

    #[test]
    fn per_tag_quota() {
        let mut r = RelayStore::default();
        for i in 0..PER_TAG_QUOTA {
            let mut e = env(0, 9, 7200, EnvType::Direct);
            e.message_id[..8].copy_from_slice(&(i as u64).to_be_bytes());
            assert!(matches!(
                r.insert(stored(e, 0, false), 0),
                Insert::Stored { .. }
            ));
        }
        assert_eq!(
            r.insert(stored(env(255, 9, 7200, EnvType::Direct), 0, false), 0),
            Insert::QuotaExceeded
        );
        // Own messages bypass the quota.
        assert!(matches!(
            r.insert(stored(env(254, 9, 7200, EnvType::Direct), 0, true), 0),
            Insert::Stored { .. }
        ));
    }

    #[test]
    fn eviction_prefers_biggest_tag_then_oldest_and_spares_mine() {
        let mut r = RelayStore {
            max_items: 4,
            ..Default::default()
        };
        r.insert(stored(env(1, 1, 7200, EnvType::Direct), 5, true), 0);
        r.insert(stored(env(2, 2, 7200, EnvType::Direct), 1, false), 0);
        r.insert(stored(env(3, 2, 7200, EnvType::Direct), 2, false), 0);
        r.insert(stored(env(4, 3, 7200, EnvType::Direct), 0, false), 0);
        let res = r.insert(stored(env(5, 4, 7200, EnvType::Direct), 9, false), 0);
        assert_eq!(
            res,
            Insert::Stored {
                evicted: vec![[2; 16]]
            }
        );
        assert!(r.contains(&[1; 16]));
    }

    #[test]
    fn ack_verification() {
        let mut r = RelayStore::default();
        let secret = [7u8; 16];
        let mut e = env(1, 1, 7200, EnvType::Direct);
        e.proof = mesh_crypto::ack_proof(&secret);
        r.insert(stored(e, 0, true), 0);
        assert_eq!(r.apply_ack(&[1; 16], &[8; 16]), AckOutcome::Invalid);
        assert_eq!(
            r.apply_ack(&[1; 16], &secret),
            AckOutcome::Deleted { was_mine: true }
        );
        assert!(!r.contains(&[1; 16]));
        assert_eq!(
            r.insert(stored(env(1, 1, 7200, EnvType::Direct), 0, false), 0),
            Insert::Acked
        );
        assert_eq!(r.apply_ack(&[9; 16], &secret), AckOutcome::NotHeld);
        r.expire(7_200_000);
        assert!(!r.is_acked(&[1; 16]));
    }

    #[test]
    fn group_envelopes_not_ackable() {
        let mut r = RelayStore::default();
        r.insert(stored(env(1, 1, 7200, EnvType::Group), 0, false), 0);
        assert_eq!(r.apply_ack(&[1; 16], &[0; 16]), AckOutcome::Invalid);
    }

    #[test]
    fn summary_order_and_wants() {
        let mut r = RelayStore::default();
        r.insert(stored(env(1, 1, 7200, EnvType::Direct), 1, false), 0);
        r.insert(stored(env(2, 1, 7200, EnvType::Direct), 5, false), 0);
        r.insert(stored(env(3, 1, 7200, EnvType::Direct), 0, true), 0);
        let s = r.summary(0);
        assert_eq!(
            s.iter().map(|e| e.message_id[0]).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        let offered = SummaryEntry {
            message_id: [9; 16],
            tag: [1; 16],
            expiry: 7200,
        };
        assert!(r.wants(&offered, false, 0));
        r.seen.insert([9; 16], 0);
        assert!(!r.wants(&offered, true, 0));
        assert!(r.wants(&offered, true, SEEN_FILTER_MS));
    }

    #[test]
    fn rebroadcast_probabilities() {
        assert!(should_rebroadcast(2, 99));
        assert!(should_rebroadcast(5, 64));
        assert!(!should_rebroadcast(5, 65));
        assert!(!should_rebroadcast(20, 35));
    }
}
