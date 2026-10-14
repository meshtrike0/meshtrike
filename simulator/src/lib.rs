//! Headless multi-node mesh simulator (section 20): real engines connected by
//! a virtual radio with range, mobility, latency, frame loss and jamming.
//! Time is virtual and fully controlled, so runs are reproducible.
#![forbid(unsafe_code)]

use mesh_api::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const STEP_MS: u64 = 50;
const WIFI_FLAG: usize = 1 << 40;

#[derive(Default)]
pub struct SimClock(pub AtomicU64);

impl Clock for SimClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub enum Cmd {
    Advertise(Vec<[u8; 16]>),
    StopAdvertise,
    Scan(Vec<[u8; 16]>),
    StopScan,
    Connect(PeerHandle),
    Frame(PeerHandle, Vec<u8>),
    Disconnect(PeerHandle),
    Lora(Vec<u8>),
    Wifi(PeerHandle, bool),
    /// Internet route: run (true) or stop it.
    NetConfig(bool),
    /// Internet route: envelope to a node's mixnet address.
    NetSend(String, Vec<u8>),
}

/// TransportHost that queues commands for the world to execute.
#[derive(Default)]
pub struct SimTransport {
    pub cmds: Mutex<Vec<Cmd>>,
    pub lora: bool,
    pub wifi: bool,
    /// The internet route is connected (set by the world).
    pub net_online: std::sync::atomic::AtomicBool,
    /// Call audio this phone played (decrypted bundles, in order).
    pub heard: Mutex<Vec<Vec<u8>>>,
}

impl TransportHost for SimTransport {
    fn start_advertising(&self, uuids: Vec<[u8; 16]>, _beacon: Vec<u8>, _p: TxPower) {
        self.cmds.lock().unwrap().push(Cmd::Advertise(uuids));
    }
    fn stop_advertising(&self) {
        self.cmds.lock().unwrap().push(Cmd::StopAdvertise);
    }
    fn start_scanning(&self, uuids: Vec<[u8; 16]>) {
        self.cmds.lock().unwrap().push(Cmd::Scan(uuids));
    }
    fn stop_scanning(&self) {
        self.cmds.lock().unwrap().push(Cmd::StopScan);
    }
    fn connect(&self, p: PeerHandle, _t: TransportKind) {
        self.cmds.lock().unwrap().push(Cmd::Connect(p));
    }
    fn send_frame(&self, p: PeerHandle, bytes: Vec<u8>) -> bool {
        self.cmds.lock().unwrap().push(Cmd::Frame(p, bytes));
        true
    }
    fn disconnect(&self, p: PeerHandle) {
        self.cmds.lock().unwrap().push(Cmd::Disconnect(p));
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            wifi_p2p: self.wifi,
            lora_paired: self.lora,
            tx_power_control: true,
        }
    }
    fn lora_send(&self, bytes: Vec<u8>) -> bool {
        self.cmds.lock().unwrap().push(Cmd::Lora(bytes));
        self.lora
    }
    fn wifi_link(
        &self,
        peer: PeerHandle,
        group_owner: bool,
        _name: String,
        _pass: String,
        _auth: [u8; 32],
    ) {
        self.cmds.lock().unwrap().push(Cmd::Wifi(peer, group_owner));
    }
    fn internet_config(&self, enabled: bool, _config_json: String) {
        self.cmds.lock().unwrap().push(Cmd::NetConfig(enabled));
    }
    fn call_audio(&self, data: Vec<u8>) {
        self.heard.lock().unwrap().push(data);
    }
    fn internet_send(&self, address: String, envelope: Vec<u8>) -> bool {
        if !self.net_online.load(Ordering::Relaxed) {
            return false;
        }
        self.cmds
            .lock()
            .unwrap()
            .push(Cmd::NetSend(address, envelope));
        true
    }
}

/// A node's address on the simulated mixnet.
pub fn net_address_of(node: usize) -> String {
    format!("sim{node}.enc@gateway")
}

fn net_node_of(addr: &str) -> Option<usize> {
    addr.strip_prefix("sim")?.split('.').next()?.parse().ok()
}

pub struct Node {
    pub engine: Engine,
    pub tx: Arc<SimTransport>,
    pub events: Arc<VecSink>,
    pub storage: Arc<MemoryStorage>,
    pub keystore: Arc<SoftwareKeystore>,
    pub pos: (f64, f64),
    pub vel: (f64, f64),
    pub advertising: Vec<[u8; 16]>,
    pub scanning: Vec<[u8; 16]>,
    pub powered: bool,
    /// How many times the host was asked to (re)start advertising.
    pub advertise_starts: u64,
    pub frames_sent: u64,
    pub bytes_sent: u64,
}

/// Handle a node uses for another node: other index + 1 (0 is never used).
pub fn handle_of(other: usize) -> PeerHandle {
    other as PeerHandle + 1
}

/// Wi-Fi P2P links use a second handle per neighbour.
pub const WIFI_HANDLE_BASE: PeerHandle = 100_000;

pub fn wifi_handle_of(other: usize) -> PeerHandle {
    WIFI_HANDLE_BASE + other as PeerHandle + 1
}

fn node_of(h: PeerHandle) -> usize {
    if h > WIFI_HANDLE_BASE {
        (h - WIFI_HANDLE_BASE - 1) as usize
    } else {
        (h - 1) as usize
    }
}

pub struct World {
    pub clock: Arc<SimClock>,
    pub nodes: Vec<Node>,
    pub range_m: f64,
    pub area: (f64, f64),
    pub connected: HashSet<(usize, usize)>,
    pending_connect: Vec<(usize, usize, u64)>,
    in_flight: Vec<(u64, usize, usize, Vec<u8>)>,
    pub latency_ms: u64,
    pub loss: f64,
    /// Jammed circle: (x, y, radius).
    pub jammers: Vec<(f64, f64, f64)>,
    rng: u64,
    pub collect_events: bool,
    last_discovery: u64,
    pub lora_range_m: f64,
    pub wifi_range_m: f64,
    /// Wi-Fi P2P links (unordered pair) and pending requests (from, to).
    pub wifi_links: HashSet<(usize, usize)>,
    wifi_requests: HashSet<(usize, usize)>,
    pub wifi_frames: u64,
    /// Internet route: nodes whose route is switched on, nodes with no
    /// connectivity right now, envelopes in transit (due, to, bytes), and the
    /// gateway mailbox holding envelopes for offline nodes.
    pub net_enabled: HashSet<usize>,
    pub net_cut: HashSet<usize>,
    net_in_flight: Vec<(u64, usize, Vec<u8>)>,
    pub net_mailbox: HashMap<usize, Vec<Vec<u8>>>,
    pub net_latency_ms: u64,
    pub net_sent: u64,
}

pub fn fast_config(seed: u64) -> CoreConfig {
    CoreConfig {
        argon2_m_kib: 64,
        argon2_t: 1,
        rng_seed: Some(seed),
        max_links: 8,
        pow_bits_override: Some(8),
    }
}

impl World {
    pub fn new(start_ms: u64, seed: u64) -> Self {
        let clock = Arc::new(SimClock::default());
        clock.0.store(start_ms, Ordering::Relaxed);
        Self {
            clock,
            nodes: Vec::new(),
            range_m: 30.0,
            area: (200.0, 200.0),
            connected: HashSet::new(),
            pending_connect: Vec::new(),
            in_flight: Vec::new(),
            latency_ms: 20,
            loss: 0.0,
            jammers: Vec::new(),
            rng: seed | 1,
            collect_events: true,
            last_discovery: 0,
            lora_range_m: 5_000.0,
            wifi_range_m: 60.0,
            wifi_links: HashSet::new(),
            wifi_requests: HashSet::new(),
            wifi_frames: 0,
            net_enabled: HashSet::new(),
            net_cut: HashSet::new(),
            net_in_flight: Vec::new(),
            net_mailbox: HashMap::new(),
            net_latency_ms: 900,
            net_sent: 0,
        }
    }

    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    pub fn add_node(&mut self, pos: (f64, f64)) -> usize {
        self.add_node_with(pos, false)
    }

    pub fn add_user_lora(&mut self, name: &str, pos: (f64, f64), lora: bool) -> usize {
        let i = self.add_node_with(pos, lora);
        self.nodes[i]
            .engine
            .create_identity(name.into(), "123456".into(), None)
            .expect("identity");
        i
    }

    pub fn add_user_wifi(&mut self, name: &str, pos: (f64, f64)) -> usize {
        let i = self.add_node_full(pos, false, true);
        self.nodes[i]
            .engine
            .create_identity(name.into(), "123456".into(), None)
            .expect("identity");
        i
    }

    pub fn add_node_with(&mut self, pos: (f64, f64), lora: bool) -> usize {
        self.add_node_full(pos, lora, false)
    }

    pub fn add_node_full(&mut self, pos: (f64, f64), lora: bool, wifi: bool) -> usize {
        let i = self.nodes.len();
        let tx = Arc::new(SimTransport {
            lora,
            wifi,
            ..Default::default()
        });
        let events = Arc::new(VecSink::default());
        let storage = Arc::new(MemoryStorage::default());
        let keystore = Arc::new(SoftwareKeystore::default());
        let host = Host {
            tx: tx.clone(),
            ks: keystore.clone(),
            ev: events.clone(),
            clock: self.clock.clone(),
            storage: storage.clone(),
        };
        let engine = Engine::open(fast_config(i as u64 * 7919 + 1), host).expect("open");
        self.nodes.push(Node {
            engine,
            tx,
            events,
            storage,
            keystore,
            pos,
            vel: (0.0, 0.0),
            advertising: vec![],
            scanning: vec![],
            powered: true,
            advertise_starts: 0,
            frames_sent: 0,
            bytes_sent: 0,
        });
        i
    }

    /// Add a node and create its identity (PIN 123456).
    pub fn add_user(&mut self, name: &str, pos: (f64, f64)) -> usize {
        let i = self.add_node(pos);
        self.nodes[i]
            .engine
            .create_identity(name.into(), "123456".into(), None)
            .expect("identity");
        i
    }

    pub fn dist(&self, a: usize, b: usize) -> f64 {
        let (pa, pb) = (self.nodes[a].pos, self.nodes[b].pos);
        ((pa.0 - pb.0).powi(2) + (pa.1 - pb.1).powi(2)).sqrt()
    }

    fn jammed(&self, i: usize) -> bool {
        let p = self.nodes[i].pos;
        self.jammers
            .iter()
            .any(|(x, y, r)| ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt() <= *r)
    }

    pub fn in_range(&self, a: usize, b: usize) -> bool {
        self.nodes[a].powered
            && self.nodes[b].powered
            && self.dist(a, b) <= self.range_m
            && !self.jammed(a)
            && !self.jammed(b)
    }

    fn pair(a: usize, b: usize) -> (usize, usize) {
        (a.min(b), a.max(b))
    }

    fn break_link(&mut self, a: usize, b: usize) {
        if self.connected.remove(&Self::pair(a, b)) {
            self.nodes[a].engine.on_disconnected(handle_of(b));
            self.nodes[b].engine.on_disconnected(handle_of(a));
        }
    }

    /// Advance one step: move, discover, run commands, deliver frames, tick.
    pub fn step(&mut self) {
        let now = self.now() + STEP_MS;
        self.clock.0.store(now, Ordering::Relaxed);
        let n = self.nodes.len();
        // Mobility (bounce inside the area).
        for node in &mut self.nodes {
            node.pos.0 += node.vel.0 * STEP_MS as f64 / 1000.0;
            node.pos.1 += node.vel.1 * STEP_MS as f64 / 1000.0;
            if node.pos.0 < 0.0 || node.pos.0 > self.area.0 {
                node.vel.0 = -node.vel.0;
                node.pos.0 = node.pos.0.clamp(0.0, self.area.0);
            }
            if node.pos.1 < 0.0 || node.pos.1 > self.area.1 {
                node.vel.1 = -node.vel.1;
                node.pos.1 = node.pos.1.clamp(0.0, self.area.1);
            }
        }
        // Links out of range break.
        let links: Vec<(usize, usize)> = self.connected.iter().copied().collect();
        for (a, b) in links {
            if !self.in_range(a, b) {
                self.break_link(a, b);
            }
        }
        let wl: Vec<(usize, usize)> = self.wifi_links.iter().copied().collect();
        for (a, b) in wl {
            if self.dist(a, b) > self.wifi_range_m
                || !self.nodes[a].powered
                || !self.nodes[b].powered
            {
                self.wifi_links.remove(&(a, b));
                self.nodes[a].engine.on_disconnected(wifi_handle_of(b));
                self.nodes[b].engine.on_disconnected(wifi_handle_of(a));
            }
        }
        // Discovery every 500 ms: scanners see advertisers in range.
        if now.saturating_sub(self.last_discovery) >= 500 {
            self.last_discovery = now;
            for a in 0..n {
                if self.nodes[a].scanning.is_empty() {
                    continue;
                }
                for b in 0..n {
                    if a == b || !self.in_range(a, b) || self.connected.contains(&Self::pair(a, b))
                    {
                        continue;
                    }
                    let adv = &self.nodes[b].advertising;
                    if adv.iter().any(|u| self.nodes[a].scanning.contains(u)) {
                        let uuids = adv.clone();
                        let rssi = (-40.0 - self.dist(a, b)) as i16;
                        self.nodes[a].engine.on_peer_found(
                            handle_of(b),
                            uuids,
                            rssi,
                            TransportKind::Ble,
                        );
                    }
                }
            }
        }
        // Pending connections complete after 100 ms.
        let (ready, later): (Vec<_>, Vec<_>) = self
            .pending_connect
            .drain(..)
            .partition(|(_, _, t)| *t <= now);
        self.pending_connect = later;
        for (a, b, _) in ready {
            if self.connected.contains(&Self::pair(a, b)) {
                continue; // the other direction won; one link per pair
            }
            if self.in_range(a, b) && !self.nodes[b].advertising.is_empty() {
                self.connected.insert(Self::pair(a, b));
                self.nodes[b].engine.on_connected(handle_of(a), 512, false);
                self.nodes[a].engine.on_connected(handle_of(b), 512, true);
            } else {
                self.nodes[a].engine.on_disconnected(handle_of(b));
            }
        }
        // Execute commands.
        for i in 0..n {
            let cmds = std::mem::take(&mut *self.nodes[i].tx.cmds.lock().unwrap());
            for c in cmds {
                match c {
                    Cmd::Advertise(u) => {
                        self.nodes[i].advertise_starts += 1;
                        self.nodes[i].advertising = u;
                    }
                    Cmd::StopAdvertise => self.nodes[i].advertising.clear(),
                    Cmd::Scan(u) => self.nodes[i].scanning = u,
                    Cmd::StopScan => self.nodes[i].scanning.clear(),
                    Cmd::Connect(h) => {
                        let b = node_of(h);
                        if b < n {
                            self.pending_connect.push((i, b, now + 100));
                        }
                    }
                    Cmd::Frame(h, bytes) if h > WIFI_HANDLE_BASE => {
                        let b = node_of(h);
                        self.wifi_frames += 1;
                        if self.wifi_links.contains(&Self::pair(i, b)) {
                            self.in_flight.push((now + 2, i, b + WIFI_FLAG, bytes));
                        }
                    }
                    Cmd::Frame(h, bytes) => {
                        let b = node_of(h);
                        self.nodes[i].frames_sent += 1;
                        self.nodes[i].bytes_sent += bytes.len() as u64;
                        if self.connected.contains(&Self::pair(i, b)) {
                            let lost = self.loss > 0.0 && self.rand() < self.loss;
                            if !lost {
                                self.in_flight.push((now + self.latency_ms, i, b, bytes));
                            }
                        }
                    }
                    Cmd::Disconnect(h) if h > WIFI_HANDLE_BASE => {
                        let b = node_of(h);
                        if self.wifi_links.remove(&Self::pair(i, b)) {
                            self.nodes[b].engine.on_disconnected(wifi_handle_of(i));
                        }
                    }
                    Cmd::Wifi(h, _go) => {
                        let b = node_of(h);
                        self.wifi_requests.insert((i, b));
                        if self.wifi_requests.contains(&(b, i))
                            && self.dist(i, b) <= self.wifi_range_m
                            && !self.wifi_links.contains(&Self::pair(i, b))
                        {
                            self.wifi_requests.remove(&(i, b));
                            self.wifi_requests.remove(&(b, i));
                            self.wifi_links.insert(Self::pair(i, b));
                            let (lo, hi) = (i.min(b), i.max(b));
                            self.nodes[hi].engine.on_connected_kind(
                                wifi_handle_of(lo),
                                16_384,
                                false,
                                Some(TransportKind::WifiP2p),
                            );
                            self.nodes[lo].engine.on_connected_kind(
                                wifi_handle_of(hi),
                                16_384,
                                true,
                                Some(TransportKind::WifiP2p),
                            );
                        }
                    }
                    Cmd::Disconnect(h) => {
                        let b = node_of(h);
                        if self.connected.remove(&Self::pair(i, b)) {
                            self.nodes[b].engine.on_disconnected(handle_of(i));
                        }
                    }
                    Cmd::NetConfig(on) => {
                        if on {
                            self.net_enabled.insert(i);
                        } else {
                            self.net_enabled.remove(&i);
                        }
                        self.net_refresh(i);
                    }
                    Cmd::NetSend(addr, bytes) => {
                        if let Some(to) = net_node_of(&addr).filter(|t| *t < n) {
                            self.net_sent += 1;
                            self.net_in_flight
                                .push((now + self.net_latency_ms, to, bytes));
                        }
                    }
                    Cmd::Lora(bytes) => {
                        for b in 0..n {
                            if b != i
                                && self.nodes[b].tx.lora
                                && self.dist(i, b) <= self.lora_range_m
                            {
                                self.nodes[b].engine.on_lora_packet(bytes.clone());
                            }
                        }
                    }
                }
            }
        }
        // Deliver frames in order.
        let (due, later): (Vec<_>, Vec<_>) =
            self.in_flight.drain(..).partition(|(t, ..)| *t <= now);
        self.in_flight = later;
        for (_, from, to, bytes) in due {
            if to >= WIFI_FLAG {
                let to = to - WIFI_FLAG;
                if self.wifi_links.contains(&Self::pair(from, to)) {
                    self.nodes[to].engine.on_frame(wifi_handle_of(from), bytes);
                }
            } else if self.connected.contains(&Self::pair(from, to)) {
                self.nodes[to].engine.on_frame(handle_of(from), bytes);
            }
        }
        // Internet route: deliver to connected nodes, else the recipient's
        // gateway holds it (as a Nym gateway does) until they reconnect.
        let (due, later): (Vec<_>, Vec<_>) =
            self.net_in_flight.drain(..).partition(|(t, ..)| *t <= now);
        self.net_in_flight = later;
        for (_, to, bytes) in due {
            if self.net_is_up(to) {
                self.nodes[to].engine.on_internet_envelope(bytes);
            } else {
                self.net_mailbox.entry(to).or_default().push(bytes);
            }
        }
        for i in 0..n {
            if self.nodes[i].powered {
                self.nodes[i].engine.tick();
            }
            if !self.collect_events {
                self.nodes[i].events.drain();
            }
        }
    }

    fn net_is_up(&self, i: usize) -> bool {
        self.net_enabled.contains(&i) && !self.net_cut.contains(&i) && self.nodes[i].powered
    }

    /// Bring node `i`'s route up or down to match its switch and connectivity.
    fn net_refresh(&mut self, i: usize) {
        let up = self.net_is_up(i);
        let was = self.nodes[i].tx.net_online.swap(up, Ordering::Relaxed);
        if up && !was {
            let _ = self.nodes[i].engine.on_internet_ready(net_address_of(i));
            for bytes in self.net_mailbox.remove(&i).unwrap_or_default() {
                self.nodes[i].engine.on_internet_envelope(bytes);
            }
        } else if !up && was {
            self.nodes[i].engine.on_internet_down();
        }
    }

    /// Cut or restore node `i`'s internet connectivity (its switch stays as is).
    pub fn set_net_cut(&mut self, i: usize, cut: bool) {
        if cut {
            self.net_cut.insert(i);
        } else {
            self.net_cut.remove(&i);
        }
        self.net_refresh(i);
    }

    pub fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms / STEP_MS {
            self.step();
        }
    }

    /// Run until `cond` holds or `max_ms` passes; returns whether it held.
    pub fn run_until(&mut self, max_ms: u64, mut cond: impl FnMut(&mut World) -> bool) -> bool {
        let end = self.now() + max_ms;
        while self.now() < end {
            if cond(self) {
                return true;
            }
            self.step();
        }
        cond(self)
    }

    pub fn events(&self, i: usize) -> Vec<Event> {
        self.nodes[i].events.drain()
    }

    /// In-person pairing between two nodes in range (QR payload hand-over).
    pub fn pair_in_person(&mut self, bob: usize, alice: usize) -> bool {
        let qr = self.nodes[bob].engine.pairing_qr().expect("qr");
        self.nodes[alice].engine.pair_from_qr(qr).expect("scan");
        let (a_ed, b_ed) = (self.identity_key(alice), self.identity_key(bob));
        self.run_until(20_000, |w| {
            w.contact_by_key(alice, &b_ed).is_some() && w.contact_by_key(bob, &a_ed).is_some()
        })
    }

    pub fn identity_key(&self, i: usize) -> String {
        self.nodes[i]
            .engine
            .identity()
            .map(|x| x.identity_key)
            .unwrap_or_default()
    }

    pub fn contact_by_key(&self, i: usize, key: &str) -> Option<Contact> {
        self.nodes[i]
            .engine
            .contacts()
            .into_iter()
            .find(|c| c.identity_key == key)
    }

    pub fn conv_with(&self, i: usize, other: usize) -> Option<Id16> {
        self.contact_by_key(i, &self.identity_key(other))?
            .conversation
    }

    pub fn traffic(&self) -> (u64, u64) {
        self.nodes
            .iter()
            .fold((0, 0), |a, n| (a.0 + n.frames_sent, a.1 + n.bytes_sent))
    }
}

/// Count of messages with the given text in a node's conversation.
pub fn has_text(w: &mut World, node: usize, conv: Id16, text: &str) -> bool {
    w.nodes[node]
        .engine
        .messages(conv, None, 1000)
        .map(|m| m.iter().any(|m| m.text == text))
        .unwrap_or(false)
}

pub fn state_of(w: &mut World, node: usize, conv: Id16, msg: Id16) -> Option<DeliveryState> {
    w.nodes[node]
        .engine
        .messages(conv, None, 1000)
        .ok()?
        .into_iter()
        .find(|m| m.id == msg)
        .map(|m| m.state)
}

pub fn summarize_links(w: &World) -> HashMap<usize, usize> {
    let mut m = HashMap::new();
    for (a, b) in &w.connected {
        *m.entry(*a).or_insert(0) += 1;
        *m.entry(*b).or_insert(0) += 1;
    }
    m
}
