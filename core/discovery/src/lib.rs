//! Private discovery (section 5): epochs, beacons, presence tags, advertising
//! plans for Public / Community / Listen-only modes and pairing UUIDs.
#![forbid(unsafe_code)]

use mesh_constants::*;
use mesh_crypto::{community_uuid, ct_eq, presence_tag};

pub type Uuid = [u8; 16];

pub fn epoch(unix_ms: u64) -> u64 {
    unix_ms / 1000 / EPOCH_SECS
}

pub mod caps {
    pub const WIFI_P2P: u32 = 1 << 0;
    pub const LORA_BRIDGE: u32 = 1 << 1;
    pub const RELAY_ON: u32 = 1 << 2;
    /// Bits 8..16 carry the highest protocol version supported.
    pub const VERSION_SHIFT: u32 = 8;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beacon {
    pub ephemeral_id: [u8; 16],
    pub presence_tag: [u8; 16],
    pub capabilities: u32,
}

impl Beacon {
    pub fn build(
        presence_key: &[u8; 32],
        epoch: u64,
        ephemeral_id: [u8; 16],
        capabilities: u32,
    ) -> Self {
        Self {
            ephemeral_id,
            presence_tag: presence_tag(presence_key, epoch, &ephemeral_id),
            capabilities,
        }
    }

    pub fn encode(&self) -> [u8; BEACON_LEN] {
        let mut b = [0u8; BEACON_LEN];
        b[..16].copy_from_slice(&self.ephemeral_id);
        b[16..32].copy_from_slice(&self.presence_tag);
        b[32..36].copy_from_slice(&self.capabilities.to_be_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != BEACON_LEN || b[36..40] != [0; 4] {
            return None;
        }
        Some(Self {
            ephemeral_id: b[..16].try_into().unwrap(),
            presence_tag: b[16..32].try_into().unwrap(),
            capabilities: u32::from_be_bytes(b[32..36].try_into().unwrap()),
        })
    }

    pub fn protocol_version(&self) -> u8 {
        ((self.capabilities >> caps::VERSION_SHIFT) & 0xFF) as u8
    }

    /// Index of the contact presence key that produced this beacon, checking
    /// the current epoch and one either side; also returns the epoch offset.
    pub fn recognise(&self, presence_keys: &[[u8; 32]], epoch: u64) -> Option<(usize, i64)> {
        for (i, k) in presence_keys.iter().enumerate() {
            for d in -EPOCH_TOLERANCE..=EPOCH_TOLERANCE {
                let e = epoch as i64 + d;
                if e < 0 {
                    continue;
                }
                if ct_eq(
                    &presence_tag(k, e as u64, &self.ephemeral_id),
                    &self.presence_tag,
                ) {
                    return Some((i, d));
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioMode {
    Public,
    Community,
    ListenOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisingPlan {
    /// UUIDs to advertise now (empty = do not advertise).
    pub advertise: Vec<Uuid>,
    /// UUIDs to scan for.
    pub scan: Vec<Uuid>,
}

/// Compute what to advertise and scan for. `community_keys` holds at most 4
/// keys; Community mode advertises one at a time, rotating every 2 s.
/// `base_mode` is the mode Listen-only falls back to for scanning.
pub fn plan(
    mode: RadioMode,
    base_mode: RadioMode,
    community_keys: &[[u8; 32]],
    pairing_uuid: Option<Uuid>,
    now_ms: u64,
) -> AdvertisingPlan {
    let ep = epoch(now_ms);
    let current: Vec<Uuid> = community_keys
        .iter()
        .map(|k| community_uuid(k, ep))
        .collect();
    let next: Vec<Uuid> = community_keys
        .iter()
        .map(|k| community_uuid(k, ep + 1))
        .collect();
    let mut scan: Vec<Uuid> = Vec::new();
    let scan_mode = if mode == RadioMode::ListenOnly {
        base_mode
    } else {
        mode
    };
    if scan_mode == RadioMode::Public {
        scan.push(PUBLIC_SERVICE_UUID);
    }
    scan.extend(current.iter().copied());
    scan.extend(next.iter().copied());
    // Previous epoch too, for peers whose clocks lag.
    if ep > 0 {
        scan.extend(community_keys.iter().map(|k| community_uuid(k, ep - 1)));
    }
    let mut advertise = match mode {
        RadioMode::Public => vec![PUBLIC_SERVICE_UUID],
        RadioMode::Community if !current.is_empty() => {
            let slot = (now_ms / COMMUNITY_ROTATION_MS) as usize % current.len();
            vec![current[slot]]
        }
        RadioMode::Community => vec![],
        RadioMode::ListenOnly => vec![],
    };
    if let Some(p) = pairing_uuid {
        // Pairing is advertised in every mode, including Listen-only (15.1).
        advertise.insert(0, p);
    }
    AdvertisingPlan { advertise, scan }
}

/// Clock-skew detector (section 19): warn when 3 peers within 10 minutes
/// report epochs differing from ours by 2 or more.
#[derive(Default)]
pub struct ClockSkew {
    hits: Vec<u64>,
}

impl ClockSkew {
    pub fn observe(&mut self, epoch_diff: i64, now_ms: u64) -> bool {
        self.hits
            .retain(|t| now_ms.saturating_sub(*t) < CLOCK_SKEW_WINDOW_MS);
        if epoch_diff.abs() >= CLOCK_SKEW_EPOCHS {
            self.hits.push(now_ms);
        }
        self.hits.len() >= CLOCK_SKEW_PEERS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_roundtrip_and_recognition() {
        let k = [7u8; 32];
        let ep = 1_900_000;
        let b = Beacon::build(&k, ep, [1; 16], caps::RELAY_ON | (2 << caps::VERSION_SHIFT));
        let d = Beacon::decode(&b.encode()).unwrap();
        assert_eq!(d, b);
        assert_eq!(d.protocol_version(), 2);
        assert_eq!(d.recognise(&[[1; 32], k], ep), Some((1, 0)));
        assert_eq!(d.recognise(&[k], ep + 1), Some((0, -1)));
        assert_eq!(d.recognise(&[k], ep + 2), None);
        assert_eq!(d.recognise(&[[2; 32]], ep), None);
    }

    #[test]
    fn plans() {
        let now = 1_700_000_000_000;
        let p = plan(RadioMode::Public, RadioMode::Public, &[], None, now);
        assert_eq!(p.advertise, vec![PUBLIC_SERVICE_UUID]);
        let keys = [[1u8; 32], [2u8; 32]];
        let c1 = plan(RadioMode::Community, RadioMode::Public, &keys, None, now);
        let c2 = plan(
            RadioMode::Community,
            RadioMode::Public,
            &keys,
            None,
            now + COMMUNITY_ROTATION_MS,
        );
        assert_eq!(c1.advertise.len(), 1);
        assert_ne!(c1.advertise, c2.advertise);
        assert!(!c1.scan.contains(&PUBLIC_SERVICE_UUID));
        let l = plan(
            RadioMode::ListenOnly,
            RadioMode::Public,
            &keys,
            Some([9; 16]),
            now,
        );
        assert_eq!(l.advertise, vec![[9; 16]]);
        assert!(l.scan.contains(&PUBLIC_SERVICE_UUID));
    }

    #[test]
    fn clock_skew() {
        let mut c = ClockSkew::default();
        assert!(!c.observe(2, 0));
        assert!(!c.observe(0, 1));
        assert!(!c.observe(-3, 2));
        assert!(c.observe(2, 3));
    }
}
