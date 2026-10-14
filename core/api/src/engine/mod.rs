//! The core engine (D29/D36): one deterministic state machine driven by host
//! calls and `tick()`. It owns every key and all plaintext; the host sees
//! only radio frames, UI events and the data it displays.

mod calls;
mod extras;
mod groups;
mod inbound;
mod internet;
mod link;
mod messaging;
mod pairing;

pub use link::Link;

use crate::host::*;
use crate::records::{self, t, *};
use crate::types::*;
use mesh_constants::*;
use mesh_countermeasures::{InterferenceDetector, ListenOnlyPolicy, TxPowerController};
use mesh_crypto::vodozemac::megolm::{GroupSession, InboundGroupSession};
use mesh_crypto::vodozemac::olm::Session;
use mesh_crypto::{random_array, SymKey};
use mesh_discovery::{Beacon, ClockSkew};
use mesh_identity::LocalIdentity;
use mesh_routing::RelayStore;
use mesh_storage::{Kv, Op};
use mesh_wire::{Envelope, Tag};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub argon2_m_kib: u32,
    pub argon2_t: u32,
    /// Seed for routing randomness (simulator); None = OS random.
    pub rng_seed: Option<u64>,
    pub max_links: usize,
    /// Override broadcast PoW difficulty (simulator only).
    pub pow_bits_override: Option<u32>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            argon2_m_kib: ARGON2_MEMORY_KIB,
            argon2_t: ARGON2_ITERATIONS,
            rng_seed: None,
            max_links: 8,
            pow_bits_override: None,
        }
    }
}

pub struct Host {
    pub tx: Arc<dyn TransportHost>,
    pub ks: Arc<dyn Keystore>,
    pub ev: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    pub storage: Arc<dyn StorageProvider>,
}

/// Small PRNG for routing decisions (delays, TTL, rebroadcast). Key
/// material always comes from the OS generator instead.
pub(crate) struct SplitMix(u64);

impl SplitMix {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: u64, hi_inclusive: u64) -> u64 {
        lo + self.next() % (hi_inclusive - lo + 1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    wrapped_relay: Vec<u8>,
    wrapped_msg_hw: Vec<u8>,
    salt: [u8; 16],
    duress: Option<Vec<u8>>,
    biometric: bool,
}

/// State available after first unlock: routing works while the app is locked.
/// Key under `t::SETTINGS` holding the Wi-Fi Direct switch (bool, true = off).
const WIFI_OFF_KEY: &[u8] = b"wifi_off";

pub(crate) struct RelayState {
    pub kv: Box<dyn Kv>,
    pub store: RelayStore,
    pub tagkeys: HashMap<Id16, TagKeyRec>,
    pub presence: HashMap<Id16, [u8; 32]>,
    pub me: RelayMe,
    pub communities: BTreeMap<Id16, CommunityRec>,
    pub settings: Settings,
    /// The user switched Wi-Fi Direct off (Settings). Stored on its own key
    /// so the postcard-encoded `Settings` record keeps its layout.
    pub wifi_off: bool,
    /// expiry → (tag → tagkey record id), built lazily.
    pub tag_cache: HashMap<u32, HashMap<Tag, Id16>>,
}

pub(crate) struct OlmEntry {
    pub id: Id16,
    pub session: Session,
    pub last_decrypt_ms: u64,
    pub created_ms: u64,
}

pub(crate) struct GroupEntry {
    pub rec: GroupRec,
    pub outbound: GroupSession,
}

pub(crate) struct InMegolm {
    pub rec: InMegolmRec,
    pub session: InboundGroupSession,
}

/// State available only after PIN unlock.
pub(crate) struct MsgState {
    pub kv: Box<dyn Kv>,
    pub pickle_key: Zeroizing<[u8; 32]>,
    pub me: LocalIdentity,
    pub peers: HashMap<Id16, PeerRec>,
    pub sessions: HashMap<Id16, Vec<OlmEntry>>,
    pub convs: HashMap<Id16, ConvRec>,
    pub groups: HashMap<Id16, GroupEntry>,
    pub megolm_in: HashMap<[u8; 32], InMegolm>,
    pub channels: HashMap<Id16, ChannelRec>,
    pub pairings: pairing::Pairings,
}

pub struct Engine {
    pub(crate) cfg: CoreConfig,
    pub(crate) host: Host,
    pub(crate) rng: SplitMix,
    meta: Option<Meta>,
    pub(crate) relay: Option<RelayState>,
    pub(crate) msg: Option<MsgState>,
    pub(crate) links: HashMap<PeerHandle, Link>,
    pub(crate) connecting: HashMap<PeerHandle, u64>,
    pub(crate) discovered: HashMap<PeerHandle, Discovered>,
    /// Handles we connected out to that turned out to be a second link to a
    /// phone we already had (its ephemeral ID): not dialled again while that
    /// link lasts, so two phones do not keep re-opening the losing link.
    pub(crate) dup_of: HashMap<PeerHandle, [u8; 16]>,
    /// The one nearby voice call, ringing or in progress.
    pub(crate) call: Option<calls::Call>,
    /// Outgoing messages already past "Queued", so later envelopes of the
    /// same message (a file's many parts) skip reloading the whole record.
    pub(crate) sent_marked: HashSet<Id16>,
    /// Envelopes of an outgoing message still awaiting their ack: counted
    /// down per ack so a file's parts do not each rewrite the whole record.
    pub(crate) pending_left: HashMap<Id16, usize>,
    pub(crate) eph_id: [u8; 16],
    pub(crate) eph_epoch: u64,
    /// What the radio advertises now: service UUIDs and power. The beacon is
    /// not part of it: it travels over the link, so a beacon-only change (a
    /// capability bit, for instance) must never restart advertising, which on
    /// Android can also change the Bluetooth address and break connections.
    last_adv: Option<(Vec<[u8; 16]>, TxPower)>,
    last_scan: Option<Vec<[u8; 16]>>,
    last_slow_tick: u64,
    last_community_slot: u64,
    pub(crate) tx_power: TxPowerController,
    pub(crate) interference: InterferenceDetector,
    pub(crate) listen_only: ListenOnlyPolicy,
    pub(crate) broadcast_rate: mesh_abuse::BroadcastRate,
    pub(crate) clock_skew: ClockSkew,
    pub(crate) crypto_failures: u32,
    /// Carrier of the envelope being ingested right now (recorded on the
    /// message it creates).
    pub(crate) ingest_via: Option<crate::types::Via>,
    pub(crate) connect_attempts: u32,
    pub(crate) connect_failures: u32,
    pub(crate) last_delivery_ms: Option<u64>,
    /// Our Nym address while the internet route is connected.
    pub(crate) net_address: Option<String>,
    pub(crate) net_last_flush: u64,
    net_last_reach: u64,
    /// Envelopes that just left over the internet; their messages record the
    /// carrier on the next slow tick (the envelope -> message map is written
    /// after sending).
    pub(crate) net_sent_pending: Vec<mesh_wire::MessageId>,
    pub(crate) handoff: extras::HandoffState,
    pub(crate) background_since: Option<u64>,
    pub(crate) pairing_uuid: Option<([u8; 16], u64)>,
    pub(crate) blocked_eph: HashSet<[u8; 16]>,
    pub(crate) direct_only: HashMap<mesh_wire::MessageId, Id16>,
    pub(crate) last_expire_ms: u64,
    pub(crate) log: crate::log::DebugLog,
}

#[derive(Debug, Clone)]
pub(crate) struct Discovered {
    pub rssi: i16,
    pub last_seen: u64,
    pub kind: TransportKind,
}

fn storage_err(_: mesh_storage::StorageError) -> CoreError {
    CoreError::Internal(10)
}

impl Engine {
    /// `core_open` (section 13). Opens the relay store if an identity exists
    /// so relaying works before the PIN is entered.
    pub fn open(cfg: CoreConfig, host: Host) -> CoreResult<Self> {
        let now = host.clock.now_ms();
        let seed = cfg
            .rng_seed
            .unwrap_or_else(|| u64::from_be_bytes(random_array()));
        let mut e = Self {
            cfg,
            host,
            rng: SplitMix(seed),
            meta: None,
            relay: None,
            msg: None,
            links: HashMap::new(),
            connecting: HashMap::new(),
            discovered: HashMap::new(),
            dup_of: HashMap::new(),
            call: None,
            sent_marked: HashSet::new(),
            pending_left: HashMap::new(),
            eph_id: random_array(),
            eph_epoch: mesh_discovery::epoch(now),
            last_adv: None,
            last_scan: None,
            last_slow_tick: now,
            last_community_slot: 0,
            tx_power: TxPowerController::new(now),
            interference: InterferenceDetector::default(),
            listen_only: ListenOnlyPolicy::new(now),
            broadcast_rate: Default::default(),
            clock_skew: ClockSkew::default(),
            crypto_failures: 0,
            ingest_via: None,
            connect_attempts: 0,
            connect_failures: 0,
            last_delivery_ms: None,
            net_address: None,
            net_last_flush: 0,
            net_last_reach: 0,
            net_sent_pending: Vec::new(),
            handoff: Default::default(),
            background_since: None,
            pairing_uuid: None,
            blocked_eph: HashSet::new(),
            direct_only: HashMap::new(),
            last_expire_ms: 0,
            log: Default::default(),
        };
        if let Some(meta) = e
            .host
            .storage
            .read_meta()
            .and_then(|b| records::dec::<Meta>(&b))
        {
            let relay_key = e.unwrap_key(KeySlot::RelayDb, &meta.wrapped_relay)?;
            let kv = e
                .host
                .storage
                .open_relay(&relay_key, false)
                .map_err(|_| CoreError::Keystore)?;
            e.meta = Some(meta);
            e.relay = Some(Self::load_relay(kv, now));
            e.refresh_radio();
            e.push_net_config();
        }
        Ok(e)
    }

    pub(crate) fn now(&self) -> u64 {
        self.host.clock.now_ms()
    }

    /// Append to the debug log (no identifiers or content, section 19).
    pub(crate) fn log(&mut self, line: impl Into<String>) {
        let now = self.host.clock.now_ms();
        self.log.push(now, line);
    }

    pub fn debug_log(&self) -> String {
        self.log.text()
    }

    pub(crate) fn emit(&self, ev: Event) {
        self.host.ev.emit(ev);
    }

    fn unwrap_key(&self, slot: KeySlot, wrapped: &[u8]) -> CoreResult<Zeroizing<[u8; 32]>> {
        let k = self
            .host
            .ks
            .unwrap(slot, wrapped.to_vec())
            .map_err(|_| CoreError::Keystore)?;
        let arr: [u8; 32] = k.as_slice().try_into().map_err(|_| CoreError::Keystore)?;
        Ok(Zeroizing::new(arr))
    }

    fn load_relay(kv: Box<dyn Kv>, now: u64) -> RelayState {
        let mut store = RelayStore::default();
        let mut stale = Vec::new();
        for (k, rec) in records::load_all::<EnvRec>(&*kv, t::ENV) {
            match Envelope::decode(&rec.raw) {
                Ok(env) => {
                    store.insert(
                        mesh_routing::Stored {
                            raw_len: rec.raw.len(),
                            env,
                            received_ms: rec.received_ms,
                            mine: rec.mine,
                            for_me: rec.for_me,
                            offer: rec.offer,
                        },
                        now,
                    );
                }
                Err(_) => stale.push(k),
            }
        }
        for (k, exp) in records::load_all::<u32>(&*kv, t::ACK) {
            if let Ok(id) = k.as_slice().try_into() {
                store.restore_ack(id, exp);
            }
        }
        let tagkeys = records::load_all::<TagKeyRec>(&*kv, t::TAGKEY)
            .into_iter()
            .filter_map(|(k, v)| Some((k.as_slice().try_into().ok()?, v)))
            .collect();
        let presence = records::load_all::<[u8; 32]>(&*kv, t::PRESENCE)
            .into_iter()
            .filter_map(|(k, v)| Some((k.as_slice().try_into().ok()?, v)))
            .collect();
        let communities = records::load_all::<CommunityRec>(&*kv, t::COMMUNITY)
            .into_iter()
            .filter_map(|(k, v)| Some((k.as_slice().try_into().ok()?, v)))
            .collect();
        let me = records::load::<RelayMe>(&*kv, t::RELAY_ME, b"me").unwrap_or(RelayMe {
            presence_key: random_array(),
            has_identity: false,
        });
        let settings = records::load::<Settings>(&*kv, t::SETTINGS, b"s").unwrap_or_default();
        let wifi_off = records::load::<bool>(&*kv, t::SETTINGS, WIFI_OFF_KEY).unwrap_or(false);
        let mut kv = kv;
        if !stale.is_empty() {
            let _ = kv.apply(stale.into_iter().map(|k| Op::Delete(t::ENV, k)).collect());
        }
        RelayState {
            kv,
            store,
            tagkeys,
            presence,
            me,
            communities,
            settings,
            wifi_off,
            tag_cache: HashMap::new(),
        }
    }

    // ===== Lifecycle =====

    pub fn has_identity(&self) -> bool {
        self.relay.as_ref().is_some_and(|r| r.me.has_identity)
    }

    pub fn is_unlocked(&self) -> bool {
        self.msg.is_some()
    }

    fn stretch(&self, pin: &str, salt: &[u8; 16]) -> CoreResult<Zeroizing<[u8; 32]>> {
        mesh_lock::stretch_pin_with(pin, salt, self.cfg.argon2_m_kib, self.cfg.argon2_t)
            .map_err(|_| CoreError::InvalidInput("pin".into()))
    }

    pub fn create_identity(
        &mut self,
        nickname: String,
        pin: String,
        duress_pin: Option<String>,
    ) -> CoreResult<IdentityInfo> {
        if self.has_identity() {
            return Err(CoreError::InvalidInput("identity exists".into()));
        }
        mesh_identity::validate_nickname(&nickname)
            .map_err(|_| CoreError::InvalidInput("nickname".into()))?;
        mesh_lock::validate_pin(&pin).map_err(|_| CoreError::InvalidInput("pin".into()))?;
        if let Some(d) = &duress_pin {
            mesh_lock::validate_pin(d).map_err(|_| CoreError::InvalidInput("duress_pin".into()))?;
            if *d == pin {
                return Err(CoreError::InvalidInput("duress_pin".into()));
            }
        }
        let now = self.now();
        self.host.storage.wipe_all();
        let relay_key = Zeroizing::new(random_array::<32>());
        let hw_key = Zeroizing::new(random_array::<32>());
        let salt: [u8; 16] = random_array();
        let ks = self.host.ks.clone();
        let wrapped_relay = ks
            .wrap(KeySlot::RelayDb, relay_key.to_vec())
            .map_err(|_| CoreError::Keystore)?;
        let wrapped_msg_hw = ks
            .wrap(KeySlot::MessageDb, hw_key.to_vec())
            .map_err(|_| CoreError::Keystore)?;
        let p = self.stretch(&pin, &salt)?;
        let duress = match &duress_pin {
            Some(d) => {
                let dp = self.stretch(d, &salt)?;
                Some(
                    ks.hmac(KeySlot::Duress, dp.to_vec())
                        .map_err(|_| CoreError::Keystore)?,
                )
            }
            None => None,
        };
        let msg_key = mesh_lock::message_store_key(&hw_key, &p);
        let relay_kv = self
            .host
            .storage
            .open_relay(&relay_key, true)
            .map_err(storage_err)?;
        let msg_kv = self
            .host
            .storage
            .open_message(&msg_key, true)
            .map_err(storage_err)?;
        let me = LocalIdentity::generate(&nickname, now)
            .map_err(|_| CoreError::InvalidInput("nickname".into()))?;
        let meta = Meta {
            wrapped_relay,
            wrapped_msg_hw,
            salt,
            duress,
            biometric: false,
        };
        self.host.storage.write_meta(&records::enc(&meta));
        ks.counter_set(0);
        self.meta = Some(meta);
        let mut relay = Self::load_relay(relay_kv, now);
        relay.me = RelayMe {
            presence_key: *me.presence_key,
            has_identity: true,
        };
        let _ = relay.kv.apply(vec![
            records::put(t::RELAY_ME, b"me", &relay.me),
            records::put(t::SETTINGS, b"s", &relay.settings),
        ]);
        self.relay = Some(relay);
        let pickle_key = Zeroizing::new(mesh_crypto::hkdf32(&*msg_key, &[b"pickle-v1"]));
        let mut ms = MsgState {
            kv: msg_kv,
            pickle_key,
            me,
            peers: HashMap::new(),
            sessions: HashMap::new(),
            convs: HashMap::new(),
            groups: HashMap::new(),
            megolm_in: HashMap::new(),
            channels: HashMap::new(),
            pairings: Default::default(),
        };
        let id_bytes = ms.me.to_bytes(&ms.pickle_key);
        ms.kv
            .put(t::IDENTITY, b"me", &id_bytes)
            .map_err(storage_err)?;
        let info = IdentityInfo {
            nickname: ms.me.nickname.clone(),
            identity_key: hex::encode(ms.me.ed25519()),
        };
        self.msg = Some(ms);
        self.log("identity created");
        self.refresh_radio();
        Ok(info)
    }

    pub fn unlock(&mut self, pin: String) -> CoreResult<UnlockResult> {
        let meta = self.meta.clone().ok_or(CoreError::NotFound)?;
        if self.msg.is_some() {
            return Ok(UnlockResult::Ok);
        }
        if mesh_lock::validate_pin(&pin).is_err() {
            return Err(CoreError::InvalidInput("pin".into()));
        }
        let ks = self.host.ks.clone();
        let counter = ks.counter_get() + 1;
        ks.counter_set(counter);
        let p = self.stretch(&pin, &meta.salt)?;
        // Always compute the duress HMAC so timing does not depend on it.
        let d = ks
            .hmac(KeySlot::Duress, p.to_vec())
            .map_err(|_| CoreError::Keystore)?;
        if meta
            .duress
            .as_ref()
            .is_some_and(|v| mesh_crypto::ct_eq(v, &d))
        {
            self.panic_wipe();
            return Ok(UnlockResult::Wiped);
        }
        self.unlock_with_p(&meta, &p, counter)
    }

    fn unlock_with_p(
        &mut self,
        meta: &Meta,
        p: &[u8; 32],
        counter: u32,
    ) -> CoreResult<UnlockResult> {
        let hw = self.unwrap_key(KeySlot::MessageDb, &meta.wrapped_msg_hw)?;
        let msg_key = mesh_lock::message_store_key(&hw, p);
        match self.host.storage.open_message(&msg_key, false) {
            Ok(kv) => {
                self.host.ks.counter_set(0);
                self.log("unlocked");
                self.load_msg_state(kv, &msg_key)?;
                Ok(UnlockResult::Ok)
            }
            Err(mesh_storage::StorageError::WrongKey) => match mesh_lock::after_failure(counter) {
                mesh_lock::FailedAttempt::Wipe => {
                    self.panic_wipe();
                    Ok(UnlockResult::Wiped)
                }
                mesh_lock::FailedAttempt::Remaining(r) => Ok(UnlockResult::WrongPin {
                    remaining: mesh_lock::show_remaining(counter).then_some(r),
                }),
            },
            Err(_) => Err(CoreError::Keystore),
        }
    }

    pub fn enable_biometric(&mut self, pin: String) -> CoreResult<()> {
        let mut meta = self.meta.clone().ok_or(CoreError::NotFound)?;
        self.require_unlocked()?;
        let p = self.stretch(&pin, &meta.salt)?;
        let hw = self.unwrap_key(KeySlot::MessageDb, &meta.wrapped_msg_hw)?;
        let msg_key = mesh_lock::message_store_key(&hw, &p);
        self.host
            .storage
            .open_message(&msg_key, false)
            .map_err(|_| CoreError::InvalidInput("pin".into()))?;
        self.host
            .ks
            .store_biometric(p.to_vec())
            .map_err(|_| CoreError::Keystore)?;
        meta.biometric = true;
        self.host.storage.write_meta(&records::enc(&meta));
        self.meta = Some(meta);
        Ok(())
    }

    pub fn unlock_biometric(&mut self) -> CoreResult<UnlockResult> {
        let meta = self.meta.clone().ok_or(CoreError::NotFound)?;
        if !meta.biometric {
            return Err(CoreError::NotFound);
        }
        let p = self
            .host
            .ks
            .load_biometric()
            .map_err(|_| CoreError::Keystore)?;
        let p: [u8; 32] = p.as_slice().try_into().map_err(|_| CoreError::Keystore)?;
        let counter = self.host.ks.counter_get();
        self.unlock_with_p(&meta, &p, counter)
    }

    fn load_msg_state(&mut self, kv: Box<dyn Kv>, msg_key: &[u8; 32]) -> CoreResult<()> {
        let pickle_key = Zeroizing::new(mesh_crypto::hkdf32(msg_key, &[b"pickle-v1"]));
        let id_bytes = kv
            .get(t::IDENTITY, b"me")
            .map_err(storage_err)?
            .ok_or(CoreError::Internal(11))?;
        let me = LocalIdentity::from_bytes(&id_bytes, &pickle_key)
            .map_err(|_| CoreError::Internal(12))?;
        let peers = records::load_all::<PeerRec>(&*kv, t::PEER)
            .into_iter()
            .filter_map(|(k, v)| Some((k.as_slice().try_into().ok()?, v)))
            .collect();
        let mut sessions: HashMap<Id16, Vec<OlmEntry>> = HashMap::new();
        for (k, rec) in records::load_all::<OlmRec>(&*kv, t::OLM) {
            if k.len() != 32 {
                continue;
            }
            let peer: Id16 = k[..16].try_into().unwrap();
            let id: Id16 = k[16..].try_into().unwrap();
            if let Ok(p) =
                mesh_crypto::vodozemac::olm::SessionPickle::from_encrypted(&rec.pickle, &pickle_key)
            {
                sessions.entry(peer).or_default().push(OlmEntry {
                    id,
                    session: Session::from_pickle(p),
                    last_decrypt_ms: rec.last_decrypt_ms,
                    created_ms: rec.created_ms,
                });
            }
        }
        let convs = records::load_all::<ConvRec>(&*kv, t::CONV)
            .into_iter()
            .filter_map(|(k, v)| Some((k.as_slice().try_into().ok()?, v)))
            .collect();
        let mut groups = HashMap::new();
        for (k, rec) in records::load_all::<GroupRec>(&*kv, t::GROUP) {
            let Ok(id) = k.as_slice().try_into() else {
                continue;
            };
            if let Ok(p) = mesh_crypto::vodozemac::megolm::GroupSessionPickle::from_encrypted(
                &rec.outbound_pickle,
                &pickle_key,
            ) {
                groups.insert(
                    id,
                    GroupEntry {
                        outbound: GroupSession::from_pickle(p),
                        rec,
                    },
                );
            }
        }
        let mut megolm_in = HashMap::new();
        for (k, rec) in records::load_all::<InMegolmRec>(&*kv, t::MEGOLM_IN) {
            let Ok(sid) = k.as_slice().try_into() else {
                continue;
            };
            if let Ok(p) = mesh_crypto::vodozemac::megolm::InboundGroupSessionPickle::from_encrypted(
                &rec.pickle,
                &pickle_key,
            ) {
                megolm_in.insert(
                    sid,
                    InMegolm {
                        session: InboundGroupSession::from_pickle(p),
                        rec,
                    },
                );
            }
        }
        let channels = records::load_all::<ChannelRec>(&*kv, t::CHANNEL)
            .into_iter()
            .filter_map(|(k, v)| Some((k.as_slice().try_into().ok()?, v)))
            .collect();
        self.msg = Some(MsgState {
            kv,
            pickle_key,
            me,
            peers,
            sessions,
            convs,
            groups,
            megolm_in,
            channels,
            pairings: Default::default(),
        });
        self.background_since = None;
        // Acks that arrived while locked; envelopes for us that arrived while locked.
        self.process_delivered_while_locked();
        self.process_for_me();
        self.release_due_held();
        Ok(())
    }

    pub fn lock(&mut self) {
        if self.msg.take().is_some() {
            self.log("locked");
            self.emit(Event::Locked);
        }
    }

    /// Destroy wrapping keys and both databases (section 10).
    pub fn panic_wipe(&mut self) {
        for (h, _) in std::mem::take(&mut self.links) {
            self.host.tx.disconnect(h);
        }
        self.host.tx.stop_advertising();
        self.host.tx.stop_scanning();
        self.host.ks.destroy_all();
        self.host.storage.wipe_all();
        self.msg = None;
        self.relay = None;
        self.meta = None;
        self.last_adv = None;
        self.last_scan = None;
        self.handoff = Default::default();
        self.emit(Event::Wiped);
    }

    /// App moved to background / foreground (auto-lock, section 12.3).
    pub fn set_foreground(&mut self, foreground: bool) {
        let now = self.now();
        if foreground {
            if let Some(since) = self.background_since.take() {
                if now.saturating_sub(since) >= self.auto_lock_ms() {
                    self.lock();
                }
            }
        } else {
            self.background_since = Some(now);
            if self.settings().high_risk {
                self.lock();
            }
        }
    }

    fn auto_lock_ms(&self) -> u64 {
        if self.settings().high_risk {
            0
        } else {
            self.settings().auto_lock_ms
        }
    }

    pub(crate) fn require_unlocked(&self) -> CoreResult<()> {
        if self.msg.is_some() {
            Ok(())
        } else {
            Err(CoreError::Locked)
        }
    }

    pub(crate) fn settings(&self) -> Settings {
        self.relay
            .as_ref()
            .map(|r| r.settings.clone())
            .unwrap_or_default()
    }

    pub fn save_settings_pub(&mut self, s: Settings) {
        self.save_settings(s)
    }

    pub(crate) fn save_settings(&mut self, s: Settings) {
        if let Some(r) = self.relay.as_mut() {
            r.settings = s;
            let _ =
                r.kv.apply(vec![records::put(t::SETTINGS, b"s", &r.settings)]);
        }
        self.refresh_radio();
        self.emit_network();
    }

    // ===== Radio =====

    pub(crate) fn beacon(&self) -> Beacon {
        let pk = self
            .relay
            .as_ref()
            .map(|r| r.me.presence_key)
            .unwrap_or([0; 32]);
        let caps = self.host.tx.capabilities();
        let mut c = mesh_discovery::caps::RELAY_ON
            | ((PROTOCOL_VERSION as u32) << mesh_discovery::caps::VERSION_SHIFT);
        if self.wifi_direct_usable() {
            c |= mesh_discovery::caps::WIFI_P2P;
        }
        if caps.lora_paired && !self.settings().high_risk {
            c |= mesh_discovery::caps::LORA_BRIDGE;
        }
        Beacon::build(&pk, self.eph_epoch, self.eph_id, c)
    }

    pub(crate) fn effective_mode(&self) -> RadioMode {
        let s = self.settings();
        let has_community = self
            .relay
            .as_ref()
            .is_some_and(|r| !r.communities.is_empty());
        if s.high_risk && has_community && s.radio_mode == RadioMode::Public {
            RadioMode::Community
        } else if s.radio_mode == RadioMode::Community && !has_community {
            RadioMode::Public
        } else {
            s.radio_mode
        }
    }

    /// Re-send the advertising / scanning plan (after permissions change or
    /// Bluetooth is switched on).
    pub fn restart_radio(&mut self) {
        self.last_adv = None;
        self.last_scan = None;
        self.refresh_radio();
    }

    pub(crate) fn refresh_radio(&mut self) {
        let Some(relay) = self.relay.as_ref() else {
            return;
        };
        let now = self.now();
        let keys: Vec<[u8; 32]> = relay.communities.values().map(|c| c.key).collect();
        let to_d = |m: RadioMode| match m {
            RadioMode::Public => mesh_discovery::RadioMode::Public,
            RadioMode::Community => mesh_discovery::RadioMode::Community,
            RadioMode::ListenOnly => mesh_discovery::RadioMode::ListenOnly,
        };
        let mode = self.effective_mode();
        let base = if mode == RadioMode::ListenOnly {
            if keys.is_empty() || !self.settings().high_risk {
                RadioMode::Public
            } else {
                RadioMode::Community
            }
        } else {
            mode
        };
        let pairing = self
            .pairing_uuid
            .filter(|(_, until)| *until > now)
            .map(|(u, _)| u);
        let mut plan = mesh_discovery::plan(to_d(mode), to_d(base), &keys, pairing, now);
        // Scanning also covers pairing UUIDs we are looking for.
        if let Some(ms) = &self.msg {
            plan.scan.extend(ms.pairings.scanning_uuids(now));
        }
        let power = match self.tx_power.level {
            mesh_countermeasures::TxPower::UltraLow => TxPower::UltraLow,
            mesh_countermeasures::TxPower::Low => TxPower::Low,
            mesh_countermeasures::TxPower::Medium => TxPower::Medium,
            mesh_countermeasures::TxPower::High => TxPower::High,
        };
        let adv = (plan.advertise.clone(), power);
        if self.last_adv.as_ref() != Some(&adv) {
            let beacon = self.beacon().encode().to_vec();
            if plan.advertise.is_empty() {
                self.host.tx.stop_advertising();
            } else {
                self.host
                    .tx
                    .start_advertising(plan.advertise, beacon, power);
            }
            self.last_adv = Some(adv);
        }
        if self.last_scan.as_ref() != Some(&plan.scan) {
            self.host.tx.start_scanning(plan.scan.clone());
            self.last_scan = Some(plan.scan);
        }
    }

    /// Drive timers. Hosts call this about every 100 ms.
    pub fn tick(&mut self) {
        let now = self.now();
        if self.relay.is_none() {
            return;
        }
        // Epoch change: new ephemeral ID, drop all links (section 5).
        let ep = mesh_discovery::epoch(now);
        if ep != self.eph_epoch {
            self.eph_epoch = ep;
            self.eph_id = random_array();
            self.log("new epoch: ephemeral ID rotated, links reset");
            for (h, _) in std::mem::take(&mut self.links) {
                self.host.tx.disconnect(h);
            }
            self.connecting.clear();
            self.dup_of.clear();
            self.refresh_radio();
        }
        let slot = now / COMMUNITY_ROTATION_MS;
        if slot != self.last_community_slot {
            self.last_community_slot = slot;
            if self.effective_mode() == RadioMode::Community {
                self.refresh_radio();
            }
            if let Some((_, until)) = self.pairing_uuid {
                if until <= now {
                    self.pairing_uuid = None;
                    self.refresh_radio();
                }
            }
        }
        self.tick_links(now);
        self.tick_call(now);
        if now.saturating_sub(self.last_slow_tick) >= 1_000 {
            self.last_slow_tick = now;
            self.slow_tick(now);
        }
    }

    fn slow_tick(&mut self, now: u64) {
        // Relay store expiry.
        let dead = self
            .relay
            .as_mut()
            .map(|r| r.store.expire(now))
            .unwrap_or_default();
        if !dead.is_empty() {
            if let Some(r) = self.relay.as_mut() {
                let _ = r.kv.apply(
                    dead.iter()
                        .map(|d| Op::Delete(t::ENV, d.to_vec()))
                        .collect(),
                );
                r.tag_cache.retain(|e, _| (*e as u64) * 1000 > now);
            }
        }
        let strong = self
            .links
            .values()
            .filter(|l| l.rssi >= TX_STEP_DOWN_RSSI)
            .count();
        let public = self.effective_mode() == RadioMode::Public;
        if self.tx_power.tick(now, public, strong).is_some() {
            self.last_adv = None;
            self.refresh_radio();
        }
        // Listen-only: connect out when needed.
        if self.effective_mode() == RadioMode::ListenOnly {
            let queued = self
                .relay
                .as_ref()
                .is_some_and(|r| r.store.iter().any(|s| s.mine && s.offer));
            if self.links.is_empty() && self.listen_only.should_connect(now, queued) {
                let cands: Vec<PeerHandle> = self.discovered.keys().copied().collect();
                for p in cands.into_iter().take(2) {
                    self.connect_to(p);
                }
            }
        }
        self.discovered
            .retain(|_, d| now.saturating_sub(d.last_seen) < 60_000);
        self.connecting
            .retain(|_, t0| now.saturating_sub(*t0) < 15_000);
        if self.msg.is_some() {
            for id in std::mem::take(&mut self.net_sent_pending) {
                self.on_envelope_sent(&id, Some(Via::Internet));
            }
            self.messaging_slow_tick(now);
            self.pairing_slow_tick(now);
            self.groups_slow_tick(now);
            // New contacts (or a fresh unlock) learn our internet address.
            if self.net_address.is_some() && now.saturating_sub(self.net_last_reach) >= 60_000 {
                self.net_last_reach = now;
                self.share_reach();
            }
        }
        self.flush_net_outbox();
        if self.handoff.receiver.is_none() && self.handoff.sender.is_none() {
            // nothing
        }
    }

    pub fn network_status(&self) -> NetworkStatus {
        let r = self.relay.as_ref();
        let mut transports: Vec<TransportKind> = self.links.values().map(|l| l.kind).collect();
        transports.sort_by_key(|k| *k as u8);
        transports.dedup();
        NetworkStatus {
            neighbours: self.links.values().filter(|l| l.is_up()).count() as u32,
            contacts_nearby: self.links.values().filter(|l| l.peer.is_some()).count() as u32,
            transports,
            last_delivery_ms: self.last_delivery_ms,
            interference: self.interference.suspected,
            radio_mode: self.effective_mode(),
            high_risk: self.settings().high_risk,
            relay_envelopes: r
                .map(|r| {
                    r.store
                        .iter()
                        .filter(|s| Self::carried_for_others(s))
                        .count() as u32
                })
                .unwrap_or(0),
            relay_bytes: r
                .map(|r| {
                    r.store
                        .iter()
                        .filter(|s| Self::carried_for_others(s))
                        .map(|s| s.raw_len as u64)
                        .sum()
                })
                .unwrap_or(0),
            crypto_failures: self.crypto_failures,
            lora_paired: self.host.tx.capabilities().lora_paired,
            tx_power: match self.tx_power.level {
                mesh_countermeasures::TxPower::UltraLow => TxPower::UltraLow,
                mesh_countermeasures::TxPower::Low => TxPower::Low,
                mesh_countermeasures::TxPower::Medium => TxPower::Medium,
                mesh_countermeasures::TxPower::High => TxPower::High,
            },
            advertising: self.last_adv.as_ref().is_some_and(|a| !a.0.is_empty()),
            wifi_p2p_capable: self.host.tx.capabilities().wifi_p2p,
            wifi_direct_on: !self.wifi_off(),
            internet_on: self.net_on() && !self.settings().high_risk,
            internet_online: self.net_usable(),
            internet_direct: self.net_direct(),
            bridges: self.bridge_links().len() as u32,
        }
    }

    /// A sealed message held for other people: not our own outbound traffic,
    /// not mail for us, and not a delivery ack (acks only delete copies).
    fn carried_for_others(s: &mesh_routing::Stored) -> bool {
        !s.mine && !s.for_me && s.env.typ != mesh_wire::EnvType::Ack
    }

    fn wifi_off(&self) -> bool {
        self.relay.as_ref().is_some_and(|r| r.wifi_off)
    }

    /// Wi-Fi Direct may be offered and used: the host can, the user has not
    /// switched it off, and High-Risk is off.
    pub(crate) fn wifi_direct_usable(&self) -> bool {
        self.host.tx.capabilities().wifi_p2p && !self.wifi_off() && !self.settings().high_risk
    }

    /// Settings switch "Wi-Fi Direct for large files". Off also closes any
    /// open Wi-Fi link; BLE carries everything.
    pub fn set_wifi_direct(&mut self, on: bool) {
        let Some(r) = self.relay.as_mut() else { return };
        r.wifi_off = !on;
        let _ =
            r.kv.apply(vec![records::put(t::SETTINGS, WIFI_OFF_KEY, &!on)]);
        if !on {
            let wifi: Vec<PeerHandle> = self
                .links
                .iter()
                .filter(|(_, l)| l.kind == TransportKind::WifiP2p)
                .map(|(h, _)| *h)
                .collect();
            for h in wifi {
                self.links.remove(&h);
                self.host.tx.disconnect(h);
            }
        }
        // Beacons carry the capability bit: re-advertise.
        self.refresh_radio();
        self.emit_network();
    }

    /// Whether fingerprint unlock was set up (no secret is read).
    pub fn biometric_enabled(&self) -> bool {
        self.meta.as_ref().is_some_and(|m| m.biometric)
    }

    pub(crate) fn emit_network(&self) {
        self.emit(Event::NetworkChanged {
            status: self.network_status(),
        });
    }

    pub fn identity(&self) -> CoreResult<IdentityInfo> {
        let ms = self.msg.as_ref().ok_or(CoreError::Locked)?;
        Ok(IdentityInfo {
            nickname: ms.me.nickname.clone(),
            identity_key: hex::encode(ms.me.ed25519()),
        })
    }

    pub fn get_settings(&self) -> Settings {
        self.settings()
    }

    pub fn set_radio_mode(&mut self, m: RadioMode) -> CoreResult<()> {
        let mut s = self.settings();
        if m == RadioMode::Community && self.relay.as_ref().is_none_or(|r| r.communities.is_empty())
        {
            return Err(CoreError::InvalidInput("no community".into()));
        }
        s.radio_mode = m;
        self.save_settings(s);
        Ok(())
    }

    pub fn set_high_risk(&mut self, on: bool) -> CoreResult<()> {
        let mut s = self.settings();
        s.high_risk = on;
        if on {
            s.disappear_default_secs = DISAPPEARING_HIGH_RISK_SECS;
        } else if s.disappear_default_secs == DISAPPEARING_HIGH_RISK_SECS {
            s.disappear_default_secs = DISAPPEARING_DEFAULT_SECS;
        }
        self.save_settings(s);
        // High-Risk never uses the internet route.
        self.push_net_config();
        // Links switch to fixed frames: rebuild them.
        for (h, _) in std::mem::take(&mut self.links) {
            self.host.tx.disconnect(h);
        }
        Ok(())
    }

    pub fn set_read_receipts(&mut self, on: bool) {
        let mut s = self.settings();
        s.read_receipts = on;
        self.save_settings(s);
    }

    pub fn set_disappear_default(&mut self, secs: u32) -> CoreResult<()> {
        if !(DISAPPEARING_MIN_SECS..=DISAPPEARING_MAX_SECS).contains(&secs) {
            return Err(CoreError::InvalidInput("secs".into()));
        }
        let mut s = self.settings();
        s.disappear_default_secs = secs;
        self.save_settings(s);
        Ok(())
    }

    pub fn set_auto_lock(&mut self, ms: u64) {
        let mut s = self.settings();
        s.auto_lock_ms = ms;
        self.save_settings(s);
    }

    pub fn set_duress_pin(&mut self, pin: String, duress: Option<String>) -> CoreResult<()> {
        self.require_unlocked()?;
        let mut meta = self.meta.clone().ok_or(CoreError::NotFound)?;
        let p = self.stretch(&pin, &meta.salt)?;
        let hw = self.unwrap_key(KeySlot::MessageDb, &meta.wrapped_msg_hw)?;
        self.host
            .storage
            .open_message(&mesh_lock::message_store_key(&hw, &p), false)
            .map_err(|_| CoreError::InvalidInput("pin".into()))?;
        meta.duress = match duress {
            Some(d) => {
                if d == pin {
                    return Err(CoreError::InvalidInput("duress_pin".into()));
                }
                let dp = self.stretch(&d, &meta.salt)?;
                Some(
                    self.host
                        .ks
                        .hmac(KeySlot::Duress, dp.to_vec())
                        .map_err(|_| CoreError::Keystore)?,
                )
            }
            None => None,
        };
        self.host.storage.write_meta(&records::enc(&meta));
        self.meta = Some(meta);
        Ok(())
    }

    pub(crate) fn rand_ttl(&mut self) -> u8 {
        self.rng.range(MIN_INITIAL_TTL as u64, MAX_TTL as u64) as u8
    }
}
