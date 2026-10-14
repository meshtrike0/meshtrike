//! Link layer (sections 5, 6, 8, 11): Noise_NN per connection, beacons,
//! summary-vector exchange, envelope pull, and High-Risk cover slots.

use super::*;
use mesh_abuse::{MalformedTracker, RateLimiter};
use mesh_crypto::noise::{Handshake, Pattern, Transport};
use mesh_wire::*;
use std::collections::VecDeque;

const HANDSHAKE_TIMEOUT_MS: u64 = 10_000;
/// Send at most 18 frames/s (receivers enforce 20/s).
const SEND_RATE: f64 = 18.0;
const SEND_BURST: f64 = 18.0;
/// Wi-Fi peer-to-peer links carry 1–10 MB/s (section 9).
const WIFI_FRAMES_PER_SEC: u32 = 2_000;
const WIFI_BYTES_PER_SEC: u32 = 12 * 1024 * 1024;
const WIFI_FRAME_MTU: u16 = 16_384;
/// Request a Wi-Fi link when this much is waiting for a neighbour.
const WIFI_THRESHOLD_BYTES: usize = 64 * 1024;
const WIFI_IDLE_MS: u64 = 60_000;
/// Periodic summary re-sync per link.
const RESYNC_MS: u64 = 30_000;

pub struct Link {
    pub handle: PeerHandle,
    pub kind: TransportKind,
    pub initiator: bool,
    pub opened_ms: u64,
    hs: Option<Handshake>,
    tr: Option<Transport>,
    pub mtu: u16,
    reasm: Reassembler,
    next_stream: u16,
    pub beacon: Option<Beacon>,
    /// Recognised contact (presence tag matched).
    pub peer: Option<Id16>,
    /// Message IDs the neighbour has or was sent.
    pub known: HashSet<MessageId>,
    /// Message IDs the neighbour certainly has (it offered them, or we sent them).
    has: HashSet<MessageId>,
    last_resync: u64,
    out: VecDeque<LinkFrame>,
    high_risk: bool,
    limiter: RateLimiter,
    malformed: MalformedTracker,
    pub rssi: i16,
    pub listen_only: bool,
    offers: Vec<SummaryEntry>,
    offer_due: Option<u64>,
    sends: Vec<(u64, MessageId)>,
    last_slot: u64,
    /// Sender-side pacing below the neighbour's rate limit (section 8).
    tokens: f64,
    last_refill: u64,
    /// Noise handshake hash: both ends derive Wi-Fi credentials from it.
    hash: Vec<u8>,
    wifi_requested: bool,
    pub last_activity: u64,
}

impl Link {
    /// (queued frames, scheduled envelope sends, offers pending) for diagnostics.
    pub fn queues(&self) -> (usize, usize, usize) {
        (self.out.len(), self.sends.len(), self.offers.len())
    }

    pub fn is_up(&self) -> bool {
        self.tr.is_some() && self.beacon.is_some()
    }
}

impl Engine {
    pub fn on_peer_found(
        &mut self,
        p: PeerHandle,
        adv_uuids: Vec<[u8; 16]>,
        rssi: i16,
        kind: TransportKind,
    ) {
        if self.relay.is_none() {
            return;
        }
        let now = self.now();
        let pairing = self.msg.as_ref().is_some_and(|ms| {
            let wanted = ms.pairings.scanning_uuids(now);
            adv_uuids.iter().any(|u| wanted.contains(u))
        });
        self.discovered.insert(
            p,
            Discovered {
                rssi,
                last_seen: now,
                kind,
            },
        );
        if let Some(l) = self.links.get_mut(&p) {
            l.rssi = rssi;
            return;
        }
        if self.connecting.contains_key(&p) {
            return;
        }
        if let Some(e) = self.dup_of.get(&p).copied() {
            let still = self
                .links
                .values()
                .any(|l| l.beacon.as_ref().is_some_and(|b| b.ephemeral_id == e));
            if still {
                return;
            }
            self.dup_of.remove(&p);
        }
        let at_capacity = self.links.len() + self.connecting.len() >= self.cfg.max_links;
        let listen_only = self.effective_mode() == RadioMode::ListenOnly;
        if pairing || (!at_capacity && !listen_only) {
            self.connect_to(p);
        }
    }

    pub(crate) fn connect_to(&mut self, p: PeerHandle) {
        if self.links.contains_key(&p) || self.connecting.contains_key(&p) {
            return;
        }
        let kind = self
            .discovered
            .get(&p)
            .map(|d| d.kind)
            .unwrap_or(TransportKind::Ble);
        self.connecting.insert(p, self.now());
        self.connect_attempts += 1;
        self.host.tx.connect(p, kind);
    }

    /// `outgoing` = we are the central / connection initiator (Noise initiator).
    pub fn on_connected(&mut self, p: PeerHandle, mtu: u16, outgoing: bool) {
        self.on_connected_kind(p, mtu, outgoing, None)
    }

    pub fn on_connected_kind(
        &mut self,
        p: PeerHandle,
        mtu: u16,
        outgoing: bool,
        kind: Option<TransportKind>,
    ) {
        if self.relay.is_none() {
            self.host.tx.disconnect(p);
            return;
        }
        let now = self.now();
        self.connecting.remove(&p);
        let initiator = outgoing;
        let kind = kind
            .or_else(|| self.discovered.get(&p).map(|d| d.kind))
            .unwrap_or(TransportKind::Ble);
        let hs = if initiator {
            Handshake::initiator(Pattern::NN, None, None)
        } else {
            Handshake::responder(Pattern::NN, None)
        };
        let Ok(mut hs) = hs else { return };
        let first = if initiator { hs.write(&[]).ok() } else { None };
        let link = Link {
            handle: p,
            kind,
            initiator,
            opened_ms: now,
            hs: Some(hs),
            tr: None,
            mtu: mtu.max(23),
            reasm: Reassembler::default(),
            next_stream: 0,
            beacon: None,
            peer: None,
            known: HashSet::new(),
            has: HashSet::new(),
            last_resync: now,
            out: VecDeque::new(),
            high_risk: self.settings().high_risk,
            limiter: if kind == TransportKind::WifiP2p {
                RateLimiter::with_limits(now, WIFI_FRAMES_PER_SEC, WIFI_BYTES_PER_SEC)
            } else {
                RateLimiter::new(now)
            },
            malformed: MalformedTracker::default(),
            rssi: self.discovered.get(&p).map(|d| d.rssi).unwrap_or(-60),
            listen_only: self.effective_mode() == RadioMode::ListenOnly,
            offers: Vec::new(),
            offer_due: None,
            sends: Vec::new(),
            last_slot: now,
            tokens: SEND_BURST,
            last_refill: now,
            hash: Vec::new(),
            wifi_requested: false,
            last_activity: now,
        };
        self.links.insert(p, link);
        let who = self.log.peer(p);
        self.log(format!(
            "{who} connected ({}, mtu {mtu})",
            if initiator { "outgoing" } else { "incoming" }
        ));
        self.tx_power.on_neighbour_connected(now);
        if let Some(m) = first {
            self.host.tx.send_frame(p, m);
        }
    }

    pub fn on_disconnected(&mut self, p: PeerHandle) {
        if self.connecting.remove(&p).is_some() {
            self.connect_failures += 1;
        }
        if let Some(l) = self.links.remove(&p) {
            let who = self.log.peer(p);
            self.log(format!("{who} disconnected"));
            self.contact_link_changed(l.peer);
            self.emit_network();
        }
    }

    /// A recognised contact's link appeared or went away: its `nearby`/`link`
    /// changed, so tell the UI (contact rows, chat link pill).
    fn contact_link_changed(&self, peer: Option<Id16>) {
        if let Some(c) = peer.and_then(|id| self.contact_view(&id)) {
            self.emit(Event::ContactChanged { contact: c });
        }
    }

    fn drop_link(&mut self, p: PeerHandle) {
        self.drop_link_why(p, "closed")
    }

    fn drop_link_why(&mut self, p: PeerHandle, why: &str) {
        if self.links.contains_key(&p) {
            let who = self.log.peer(p);
            self.log(format!("{who} dropped: {why}"));
        }
        if let Some(l) = self.links.remove(&p) {
            self.contact_link_changed(l.peer);
            self.emit_network();
        }
        self.host.tx.disconnect(p);
    }

    fn malformed(&mut self, p: PeerHandle) {
        let now = self.now();
        let blocked = self
            .links
            .get_mut(&p)
            .map(|l| l.malformed.record(now))
            .unwrap_or(false);
        if blocked {
            let who = self.log.peer(p);
            self.log(format!("{who} blocked for 10 min (malformed frames)"));
            if let Some(b) = self.links.get(&p).and_then(|l| l.beacon.clone()) {
                self.blocked_eph.insert(b.ephemeral_id);
            }
            self.drop_link(p);
        }
    }

    pub fn on_frame(&mut self, p: PeerHandle, bytes: Vec<u8>) {
        let now = self.now();
        let Some(link) = self.links.get_mut(&p) else {
            return;
        };
        if link.malformed.is_blocked(now) {
            return;
        }
        if let Some(mut hs) = link.hs.take() {
            let ok = (|| -> Result<(), mesh_crypto::CryptoError> {
                hs.read(&bytes)?;
                if !link.initiator {
                    let m2 = hs.write(&[])?;
                    self.host.tx.send_frame(p, m2);
                }
                Ok(())
            })();
            if ok.is_err() || !hs.is_finished() {
                let who = self.log.peer(p);
                self.log(format!("{who} link handshake failed"));
                self.drop_link(p);
                return;
            }
            link.hash = hs.handshake_hash();
            match hs.into_transport() {
                Ok(tr) => link.tr = Some(tr),
                Err(_) => {
                    self.drop_link(p);
                    return;
                }
            }
            let beacon = self.beacon().encode().to_vec();
            self.send_link_message(p, FrameType::Beacon, &beacon);
            return;
        }
        let Some(tr) = link.tr.as_mut() else { return };
        let plain = match tr.decrypt(&bytes) {
            Ok(p) => p,
            Err(_) => {
                // Noise nonces are now out of sync; the link cannot recover.
                self.drop_link_why(p, "noise decrypt failed");
                return;
            }
        };
        link.last_activity = now;
        // Rate limit after decryption so Noise nonces stay in step.
        if !link.limiter.allow(bytes.len(), now) {
            // Closing the link is enough: a fast sender is not a malformed
            // one, and blocking would lock a contact out for 10 minutes.
            self.drop_link_why(p, "sent faster than allowed");
            return;
        }
        let frame = match LinkFrame::decode(&plain) {
            Ok(f) => f,
            Err(_) => return self.malformed(p),
        };
        match link.reasm.push(frame, now) {
            Ok(Some((ft, data))) => self.on_link_message(p, ft, data),
            Ok(None) => {}
            Err(_) => self.malformed(p),
        }
    }

    fn frame_capacity(link: &Link) -> usize {
        if link.kind == TransportKind::WifiP2p {
            return frame_capacity(WIFI_FRAME_MTU as usize);
        }
        if link.high_risk {
            frame_capacity(COVER_FRAME_LEN)
        } else {
            frame_capacity(noise_msg_len_for_mtu(link.mtu))
        }
    }

    pub(crate) fn send_link_message(&mut self, p: PeerHandle, ft: FrameType, data: &[u8]) {
        let Some(link) = self.links.get_mut(&p) else {
            return;
        };
        let stream = link.next_stream;
        link.next_stream = link.next_stream.wrapping_add(1);
        let Ok(frames) = fragment(ft, stream, data, Self::frame_capacity(link)) else {
            return;
        };
        link.out.extend(frames);
        if !link.high_risk {
            let now = self.now();
            self.flush_link(p, now);
        }
    }

    /// Live call audio: one frame, ahead of everything queued; older audio
    /// still waiting is dropped (late audio is useless).
    pub(crate) fn send_realtime(&mut self, p: PeerHandle, ft: FrameType, data: &[u8]) {
        let now = self.now();
        let Some(link) = self.links.get_mut(&p) else {
            return;
        };
        let stream = link.next_stream;
        link.next_stream = link.next_stream.wrapping_add(1);
        let Ok(mut frames) = fragment(ft, stream, data, Self::frame_capacity(link)) else {
            return;
        };
        if frames.len() != 1 {
            return;
        }
        link.out.retain(|f| f.ftype != ft);
        link.out.push_front(frames.remove(0));
        if !link.high_risk {
            self.flush_link(p, now);
        }
    }

    /// Standard mode: send queued frames as the pacing budget allows.
    fn flush_link(&mut self, p: PeerHandle, now: u64) {
        let Some(link) = self.links.get_mut(&p) else {
            return;
        };
        if link.high_risk {
            return;
        }
        let dt = now.saturating_sub(link.last_refill) as f64 / 1000.0;
        link.last_refill = now;
        let (rate, burst) = if link.kind == TransportKind::WifiP2p {
            (WIFI_FRAMES_PER_SEC as f64 * 0.9, 200.0)
        } else {
            (SEND_RATE, SEND_BURST)
        };
        link.tokens = (link.tokens + dt * rate).min(burst);
        let mut failed = false;
        while link.tokens >= 1.0 {
            let Some(f) = link.out.pop_front() else { break };
            let Some(tr) = link.tr.as_mut() else { return };
            link.tokens -= 1.0;
            match tr.encrypt(&f.encode()) {
                Ok(c) => {
                    if !self.host.tx.send_frame(p, c) {
                        failed = true;
                        break;
                    }
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            self.drop_link(p);
        }
    }

    fn on_link_message(&mut self, p: PeerHandle, ft: FrameType, data: Vec<u8>) {
        let now = self.now();
        match ft {
            FrameType::CallMedia => self.on_call_media(p, &data),
            FrameType::Beacon => {
                let Some(b) = Beacon::decode(&data) else {
                    return self.malformed(p);
                };
                if self.blocked_eph.contains(&b.ephemeral_id) || b.ephemeral_id == self.eph_id {
                    return self.drop_link_why(p, "blocked or self");
                }
                // Duplicate connection between the same two phones: keep the
                // one initiated by the lower ephemeral ID.
                let kind = self.links[&p].kind;
                let dup = self.links.values().find(|l| {
                    l.handle != p
                        && l.kind == kind
                        && l.beacon
                            .as_ref()
                            .is_some_and(|ob| ob.ephemeral_id == b.ephemeral_id)
                });
                if let Some(other) = dup.map(|l| l.handle) {
                    let mine_lower = self.eph_id < b.ephemeral_id;
                    let this_initiator = self.links[&p].initiator;
                    if this_initiator != mine_lower {
                        if this_initiator {
                            self.dup_of.insert(p, b.ephemeral_id);
                        }
                        return self.drop_link_why(p, "duplicate link");
                    } else {
                        if self.links[&other].initiator {
                            self.dup_of.insert(other, b.ephemeral_id);
                        }
                        self.drop_link_why(other, "duplicate link");
                    }
                }
                let recognised = self.recognise(&b, now);
                if let Some(l) = self.links.get_mut(&p) {
                    l.beacon = Some(b);
                    l.peer = recognised;
                }
                if let Some(c) = recognised {
                    if let Some(contact) = self.contact_view(&c) {
                        self.emit(Event::ContactChanged { contact });
                    }
                }
                self.send_summary(p);
                self.emit_network();
            }
            FrameType::Summary => {
                let Ok(entries) = decode_summary(&data) else {
                    return self.malformed(p);
                };
                let mut want = Vec::new();
                for e in entries {
                    if let Some(l) = self.links.get_mut(&p) {
                        l.known.insert(e.message_id);
                        l.has.insert(e.message_id);
                    }
                    let ours = self.tag_is_ours(&e.tag, e.expiry);
                    if let Some(r) = self.relay.as_mut() {
                        if (ours || r.settings.relay_enabled) && r.store.wants(&e, ours, now) {
                            want.push(e.message_id);
                        }
                    }
                }
                if !want.is_empty() {
                    let req = encode_request(&want);
                    self.send_link_message(p, FrameType::Request, &req);
                }
            }
            FrameType::Request => {
                let Ok(ids) = decode_request(&data) else {
                    return self.malformed(p);
                };
                let hr = self.settings().high_risk;
                for id in ids {
                    let delay = mesh_countermeasures::forwarding_delay_ms(hr, self.rng.unit());
                    if let Some(l) = self.links.get_mut(&p) {
                        l.sends.push((now + delay, id));
                    }
                }
            }
            FrameType::Envelope => {
                if let Ok(env) = Envelope::decode(&data) {
                    if let Some(l) = self.links.get_mut(&p) {
                        l.known.insert(env.message_id);
                        l.has.insert(env.message_id);
                    }
                    self.ingest(env, data.len(), Some(p));
                } else {
                    self.malformed(p);
                }
            }
            FrameType::Padding => {}
            FrameType::WifiRequest => {
                if self.wifi_allowed(p) {
                    self.send_link_message(p, FrameType::WifiAccept, &[]);
                    self.start_wifi(p);
                }
            }
            FrameType::WifiAccept => {
                if self.links.get(&p).is_some_and(|l| l.wifi_requested) && self.wifi_allowed(p) {
                    self.start_wifi(p);
                }
            }
        }
    }

    /// Both phones support Wi-Fi P2P, not High-Risk, no Wi-Fi link yet.
    fn wifi_allowed(&self, p: PeerHandle) -> bool {
        let Some(l) = self.links.get(&p) else {
            return false;
        };
        let Some(b) = &l.beacon else { return false };
        let theirs = b.capabilities & mesh_discovery::caps::WIFI_P2P != 0;
        let ours = self.wifi_direct_usable();
        let existing = self.links.values().any(|o| {
            o.kind == TransportKind::WifiP2p
                && o.beacon
                    .as_ref()
                    .is_some_and(|ob| ob.ephemeral_id == b.ephemeral_id)
        });
        l.kind == TransportKind::Ble && theirs && ours && !existing
    }

    /// Credentials derived from the BLE link's Noise handshake hash, so only
    /// the two link ends know them; the lower ephemeral ID owns the group.
    fn start_wifi(&mut self, p: PeerHandle) {
        let Some(l) = self.links.get(&p) else { return };
        let Some(b) = &l.beacon else { return };
        let go = self.eph_id < b.ephemeral_id;
        let ssid = mesh_crypto::hkdf32(&l.hash, &[b"wifi-ssid-v1"]);
        let pass = mesh_crypto::hkdf32(&l.hash, &[b"wifi-pass-v1"]);
        let auth = mesh_crypto::hkdf32(&l.hash, &[b"wifi-auth-v1"]);
        // Wi-Fi Direct requires the "DIRECT-xy" prefix. Every part after it
        // comes from the link secret, so the group name carries no app tag.
        const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let xy: String = ssid[4..6]
            .iter()
            .map(|b| ALNUM[*b as usize % ALNUM.len()] as char)
            .collect();
        let name = format!("DIRECT-{xy}-{}", hex::encode(&ssid[..4]));
        let passphrase = hex::encode(&pass[..12]);
        let who = self.log.peer(p);
        self.log(format!(
            "{who}: opening Wi-Fi peer-to-peer link ({})",
            if go { "group owner" } else { "client" }
        ));
        self.host.tx.wifi_link(p, go, name, passphrase, auth);
    }

    /// Ask for a Wi-Fi link when more than 64 KB waits for this neighbour.
    fn maybe_request_wifi(&mut self, p: PeerHandle, now: u64) {
        if !self.wifi_allowed(p) || self.links.get(&p).is_some_and(|l| l.wifi_requested) {
            return;
        }
        let waiting: usize = {
            let Some(r) = self.relay.as_ref() else { return };
            let l = &self.links[&p];
            r.store
                .summary(now)
                .iter()
                .filter(|e| {
                    !l.known.contains(&e.message_id)
                        || l.sends.iter().any(|(_, id)| id == &e.message_id)
                })
                .filter_map(|e| r.store.get(&e.message_id))
                .map(|s| s.raw_len)
                .sum::<usize>()
                + l.out.iter().map(|f| f.data.len()).sum::<usize>()
        };
        if waiting > WIFI_THRESHOLD_BYTES {
            if let Some(l) = self.links.get_mut(&p) {
                l.wifi_requested = true;
            }
            self.send_link_message(p, FrameType::WifiRequest, &[]);
        }
    }

    /// Re-check live links after presence keys change (new contact, key
    /// rotation), so a neighbour paired while connected is recognised.
    pub(crate) fn rerecognise_links(&mut self) {
        let now = self.now();
        let handles: Vec<PeerHandle> = self.links.keys().copied().collect();
        for h in handles {
            let Some(b) = self.links.get(&h).and_then(|l| l.beacon.clone()) else {
                continue;
            };
            let peer = self.recognise(&b, now);
            if let Some(l) = self.links.get_mut(&h) {
                if l.peer != peer {
                    l.peer = peer;
                    // Direct-only envelopes for this neighbour can now be offered.
                    l.known.clear();
                }
            }
            if self.links.get(&h).is_some_and(|l| l.peer.is_some()) {
                self.send_summary(h);
            }
        }
    }

    fn recognise(&mut self, b: &Beacon, now: u64) -> Option<Id16> {
        let r = self.relay.as_ref()?;
        let (ids, keys): (Vec<Id16>, Vec<[u8; 32]>) =
            r.presence.iter().map(|(k, v)| (*k, *v)).unzip();
        let (i, d) = b.recognise(&keys, mesh_discovery::epoch(now))?;
        if self.clock_skew.observe(d, now) {
            self.emit(Event::Warning {
                warning: Warning::ClockSkew,
            });
        }
        Some(ids[i])
    }

    /// Envelopes we may offer on this link (A.6 summary vector).
    fn summary_for(&self, p: PeerHandle, now: u64) -> Vec<SummaryEntry> {
        let Some(r) = self.relay.as_ref() else {
            return vec![];
        };
        let link = &self.links[&p];
        r.store
            .summary(now)
            .into_iter()
            .filter(|e| !link.known.contains(&e.message_id))
            .filter(|e| self.may_send_on(p, &e.message_id))
            .collect()
    }

    /// TTL and direct-only rules for sending a stored envelope on a link.
    pub(crate) fn may_send_on(&self, p: PeerHandle, id: &MessageId) -> bool {
        let Some(s) = self.relay.as_ref().and_then(|r| r.store.get(id)) else {
            return false;
        };
        let ttl_out = if s.mine {
            s.env.ttl
        } else {
            s.env.ttl.saturating_sub(1)
        };
        if ttl_out == 0 || !s.offer {
            return false;
        }
        if s.env.ttl == 1 && s.mine {
            // Direct-neighbour-only content (files > 256 KB, in-person pairing).
            if let Some(target) = self.direct_only_target(id) {
                return self.links.get(&p).and_then(|l| l.peer) == Some(target);
            }
        }
        true
    }

    fn send_summary(&mut self, p: PeerHandle) {
        let now = self.now();
        let entries = self.summary_for(p, now);
        if let Some(l) = self.links.get_mut(&p) {
            entries.iter().for_each(|e| {
                l.known.insert(e.message_id);
            });
        }
        let data = encode_summary(&entries);
        self.send_link_message(p, FrameType::Summary, &data);
    }

    /// Announce a newly stored envelope to every neighbour after a random
    /// forwarding delay (section 11).
    pub(crate) fn announce(&mut self, id: MessageId, except: Option<PeerHandle>) {
        let now = self.now();
        let Some(s) = self.relay.as_ref().and_then(|r| r.store.get(&id)) else {
            return;
        };
        let entry = SummaryEntry {
            message_id: id,
            tag: s.env.tag,
            expiry: s.env.expiry,
        };
        let hr = self.settings().high_risk;
        let handles: Vec<PeerHandle> = self.links.keys().copied().collect();
        for h in handles {
            if Some(h) == except || !self.may_send_on(h, &id) {
                continue;
            }
            let delay = mesh_countermeasures::forwarding_delay_ms(hr, self.rng.unit());
            let Some(l) = self.links.get_mut(&h) else {
                continue;
            };
            if !l.is_up() || l.known.contains(&id) {
                continue;
            }
            l.offers.push(entry.clone());
            let due = now + delay;
            l.offer_due = Some(l.offer_due.map_or(due, |d| d.min(due)));
        }
    }

    pub(crate) fn tick_links(&mut self, now: u64) {
        let handles: Vec<PeerHandle> = self.links.keys().copied().collect();
        for h in handles {
            let Some(l) = self.links.get_mut(&h) else {
                continue;
            };
            if (l.tr.is_none() || l.beacon.is_none())
                && now.saturating_sub(l.opened_ms) > HANDSHAKE_TIMEOUT_MS
            {
                self.drop_link(h);
                continue;
            }
            if l.listen_only
                && mesh_countermeasures::ListenOnlyPolicy::must_close(l.opened_ms, now)
                && l.sends.is_empty()
            {
                self.drop_link(h);
                continue;
            }
            l.reasm.expire(now);
            if l.kind == TransportKind::WifiP2p
                && l.out.is_empty()
                && l.sends.is_empty()
                && now.saturating_sub(l.last_activity) > WIFI_IDLE_MS
            {
                self.drop_link(h);
                continue;
            }
            let ble_up = l.kind == TransportKind::Ble && l.is_up();
            // Re-offer anything the neighbour has not confirmed (lost offers or
            // requests never answered) every 30 s.
            if l.is_up()
                && now.saturating_sub(l.last_resync) >= RESYNC_MS
                && l.out.is_empty()
                && l.sends.is_empty()
            {
                l.last_resync = now;
                let entries: Vec<SummaryEntry> = self
                    .relay
                    .as_ref()
                    .map(|r| r.store.summary(now))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|e| {
                        !self.links[&h].has.contains(&e.message_id)
                            && self.may_send_on(h, &e.message_id)
                    })
                    .take(100)
                    .collect();
                // Sent even when empty: it doubles as a keepalive, so a frame
                // lost at the end of a burst breaks the Noise stream visibly
                // and the reconnect resynchronises both sides.
                let data = encode_summary(&entries);
                self.send_link_message(h, FrameType::Summary, &data);
            }
            if ble_up && (now / 1000).is_multiple_of(2) {
                self.maybe_request_wifi(h, now);
            }
            let Some(l) = self.links.get_mut(&h) else {
                continue;
            };
            // Delta summaries.
            if l.offer_due.is_some_and(|d| d <= now) {
                l.offer_due = None;
                let offers: Vec<SummaryEntry> = std::mem::take(&mut l.offers)
                    .into_iter()
                    .filter(|e| !l.known.contains(&e.message_id))
                    .collect();
                offers.iter().for_each(|e| {
                    l.known.insert(e.message_id);
                });
                if !offers.is_empty() {
                    let data = encode_summary(&offers);
                    self.send_link_message(h, FrameType::Summary, &data);
                }
            }
            // Due envelope sends.
            let Some(l) = self.links.get_mut(&h) else {
                continue;
            };
            let (due, later): (Vec<_>, Vec<_>) = l.sends.drain(..).partition(|(t, _)| *t <= now);
            l.sends = later;
            for (_, id) in due {
                self.send_envelope_on(h, &id);
            }
            self.flush_link(h, now);
            // High-Risk fixed schedule: exactly 6 × 180-byte frames every 2 s.
            let Some(l) = self.links.get_mut(&h) else {
                continue;
            };
            if l.high_risk && now.saturating_sub(l.last_slot) >= COVER_SLOT_MS {
                let Some(tr) = l.tr.as_mut() else { continue };
                l.last_slot = now;
                let mut ok = true;
                for _ in 0..COVER_FRAMES_PER_SLOT {
                    let f = l.out.pop_front().unwrap_or_else(|| LinkFrame {
                        ftype: FrameType::Padding,
                        stream: 0,
                        index: 0,
                        count: 1,
                        data: random_array::<32>()
                            .repeat(4)
                            .into_iter()
                            .take(frame_capacity(COVER_FRAME_LEN))
                            .collect(),
                    });
                    let bytes = f.encode_fixed(COVER_FRAME_LEN - NOISE_TAG_LEN);
                    match tr.encrypt(&bytes) {
                        Ok(c) => ok &= self.host.tx.send_frame(h, c),
                        Err(_) => ok = false,
                    }
                }
                if !ok {
                    self.drop_link(h);
                }
            }
        }
    }

    fn send_envelope_on(&mut self, h: PeerHandle, id: &MessageId) {
        if !self.may_send_on(h, id) {
            return;
        }
        let Some(s) = self.relay.as_ref().and_then(|r| r.store.get(id)) else {
            return;
        };
        let mut env = s.env.clone();
        let mine = s.mine;
        if !mine {
            env.ttl = env.ttl.saturating_sub(1);
        }
        let bytes = env.encode();
        if let Some(l) = self.links.get_mut(&h) {
            l.known.insert(*id);
            l.has.insert(*id);
        }
        let via = self.links.get(&h).map(|l| crate::types::Via::from(l.kind));
        self.send_link_message(h, FrameType::Envelope, &bytes);
        if mine {
            self.on_envelope_sent(id, via);
        }
    }
}
