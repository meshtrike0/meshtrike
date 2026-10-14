//! Nearby voice calls over a live Bluetooth or Wi-Fi Direct link.
//!
//! - Signalling (offer, answer, end) travels as ordinary sealed direct
//!   messages (Olm), so only the paired contact can ring or answer. Signals
//!   older than a minute are ignored: a call never rings late.
//! - Each call has fresh X25519 keys from both sides; audio is sealed with
//!   keys derived from them (one per direction), so a recording of the radio
//!   cannot be opened later, even with the phones' long-term keys.
//! - Audio rides the link as `CallMedia` frames, ahead of queued envelopes,
//!   and stale audio is dropped rather than queued. It never goes into the
//!   relay store, the mesh or the internet route.
//! - High-Risk mode refuses calls (a call keeps the radio transmitting).

use super::*;
use mesh_wire::{CallSignalKind, Content, FrameType};

/// How long a call rings before it counts as missed.
pub const CALL_RING_MS: u64 = 45_000;
/// Call signals older than this are ignored.
const SIGNAL_MAX_AGE_MS: u64 = 60_000;
/// No audio and no link this long ends an active call.
const CALL_LOST_MS: u64 = 20_000;
/// Largest audio bundle accepted (Opus packets for about 100 ms).
pub const MAX_CALL_BUNDLE: usize = 1_000;
/// Replay window for audio sequence numbers.
const REPLAY_WINDOW: u64 = 64;
/// No audio this long marks an active call as reconnecting. A muted phone
/// still sends empty bundles, so silence here means the link.
const CALL_QUIET_MS: u64 = 2_500;
/// How often call quality is measured.
const QUALITY_EVERY_MS: u64 = 1_000;

pub mod reason {
    pub const HANGUP: u8 = 0;
    pub const DECLINED: u8 = 1;
    pub const BUSY: u8 = 2;
    pub const NO_ANSWER: u8 = 3;
    pub const LOST: u8 = 4;
    pub const UNAVAILABLE: u8 = 5;
}

pub(crate) struct Call {
    id: [u8; 8],
    peer: Id16,
    outgoing: bool,
    state: CallState,
    my_secret: [u8; 32],
    their_eph: Option<[u8; 32]>,
    send_key: Option<SymKey>,
    recv_key: Option<SymKey>,
    send_seq: u64,
    recv_top: u64,
    recv_seen: u64,
    started_ms: u64,
    answered_ms: Option<u64>,
    last_media_ms: u64,
    /// Bundles received, and the counters at the last quality sample.
    recv_got: u64,
    sample_top: u64,
    sample_got: u64,
    sample_ms: u64,
    /// Share of audio that got through in the last two samples (0–100).
    shares: [u8; 2],
    quality: u8,
    reconnecting: bool,
    /// The link carrying the call when last looked at.
    link: Option<TransportKind>,
}

impl Call {
    fn new(id: [u8; 8], peer: Id16, outgoing: bool, state: CallState, my_secret: [u8; 32], their_eph: Option<[u8; 32]>, now: u64) -> Self {
        Call {
            id,
            peer,
            outgoing,
            state,
            my_secret,
            their_eph,
            send_key: None,
            recv_key: None,
            send_seq: 0,
            recv_top: 0,
            recv_seen: 0,
            started_ms: now,
            answered_ms: None,
            last_media_ms: now,
            recv_got: 0,
            sample_top: 0,
            sample_got: 0,
            sample_ms: now,
            shares: [100, 100],
            quality: 4,
            reconnecting: false,
            link: None,
        }
    }

    /// Answered: measuring starts now.
    fn activate(&mut self, now: u64) {
        self.state = CallState::Active;
        self.answered_ms = Some(now);
        self.last_media_ms = now;
        self.sample_ms = now;
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.my_secret.zeroize();
    }
}

fn reason_text(r: u8) -> &'static str {
    match r {
        reason::DECLINED => "declined",
        reason::BUSY => "busy",
        reason::NO_ANSWER => "no answer",
        reason::LOST => "connection lost",
        reason::UNAVAILABLE => "unavailable",
        _ => "ended",
    }
}

impl Engine {
    // ===== Public API =====

    /// Ring a contact who is in radio range right now.
    pub fn start_call(&mut self, contact: Id16) -> CoreResult<String> {
        let now = self.now();
        if self.settings().high_risk {
            return Err(CoreError::InvalidInput(
                "Calls are off in High-Risk mode: a call keeps your radio transmitting".into(),
            ));
        }
        self.ms()?;
        if self.call.is_some() {
            return Err(CoreError::InvalidInput("already in a call".into()));
        }
        if self.call_link(&contact).is_none() {
            return Err(CoreError::InvalidInput(
                "Calls work only when you are near each other".into(),
            ));
        }
        let id: [u8; 8] = random_array();
        let secret: [u8; 32] = random_array();
        let eph = mesh_crypto::x25519_public(&secret);
        let mut c = Call::new(id, contact, true, CallState::Calling, secret, None, now);
        c.link = self.call_link_kind(&contact);
        self.call = Some(c);
        self.send_call_signal(&contact, CallSignalKind::Offer, id, eph, 0);
        self.log("call: ringing a contact");
        self.emit_call();
        Ok(hex::encode(id))
    }

    /// Pick up the ringing call.
    pub fn answer_call(&mut self, id: &str) -> CoreResult<()> {
        let now = self.now();
        let c = self.call.as_mut().ok_or(CoreError::NotFound)?;
        if hex::encode(c.id) != id || c.outgoing || c.state != CallState::Ringing {
            return Err(CoreError::NotFound);
        }
        let their = c.their_eph.ok_or(CoreError::NotFound)?;
        Self::derive_call_keys(c, &their)?;
        c.activate(now);
        let (peer, cid, eph) = (c.peer, c.id, mesh_crypto::x25519_public(&c.my_secret));
        self.send_call_signal(&peer, CallSignalKind::Answer, cid, eph, 0);
        self.log("call: answered");
        self.emit_call();
        Ok(())
    }

    /// Hang up, decline or cancel.
    pub fn end_call(&mut self, id: &str) -> CoreResult<()> {
        let c = self.call.as_ref().ok_or(CoreError::NotFound)?;
        if hex::encode(c.id) != id {
            return Err(CoreError::NotFound);
        }
        let r = if c.state == CallState::Ringing { reason::DECLINED } else { reason::HANGUP };
        let (peer, cid) = (c.peer, c.id);
        self.send_call_signal(&peer, CallSignalKind::End, cid, [0; 32], r);
        self.finish_call(r, true);
        Ok(())
    }

    pub fn current_call(&self) -> Option<CallView> {
        self.call.as_ref().map(|c| self.call_view(c, None))
    }

    /// One bundle of microphone audio (Opus packets) for the other phone.
    /// Dropped quietly when the call is not active or no link is up.
    pub fn call_send_audio(&mut self, data: &[u8]) {
        if data.is_empty() || data.len() > MAX_CALL_BUNDLE {
            return;
        }
        let Some(c) = self.call.as_mut() else { return };
        if c.state != CallState::Active {
            return;
        }
        let Some(key) = c.send_key.clone() else { return };
        c.send_seq += 1;
        let seq = c.send_seq;
        let (peer, cid, dir) = (c.peer, c.id, c.outgoing as u8);
        let ct = mesh_crypto::seal(&key, &media_nonce(&cid, seq), &[dir], data);
        let mut frame = Vec::with_capacity(16 + ct.len());
        frame.extend_from_slice(&cid);
        frame.extend_from_slice(&seq.to_be_bytes());
        frame.extend_from_slice(&ct);
        if let Some(h) = self.call_link(&peer) {
            self.send_realtime(h, FrameType::CallMedia, &frame);
        }
    }

    // ===== Internals =====

    fn call_link_kind(&self, peer: &Id16) -> Option<TransportKind> {
        self.call_link(peer).and_then(|h| self.links.get(&h)).map(|l| l.kind)
    }

    /// The best live link to `peer`: Wi-Fi Direct first, then Bluetooth.
    pub(crate) fn call_link(&self, peer: &Id16) -> Option<PeerHandle> {
        let mut best: Option<(PeerHandle, bool)> = None;
        for l in self.links.values() {
            if l.peer.as_ref() == Some(peer) && l.is_up() {
                let wifi = l.kind == TransportKind::WifiP2p;
                if best.is_none_or(|(_, w)| wifi && !w) {
                    best = Some((l.handle, wifi));
                }
            }
        }
        best.map(|(h, _)| h)
    }

    fn derive_call_keys(c: &mut Call, their: &[u8; 32]) -> CoreResult<()> {
        let shared = mesh_crypto::x25519_shared(&c.my_secret, their).map_err(|_| CoreError::Crypto)?;
        let k_caller = mesh_crypto::hkdf32(&shared, &[b"meshtrike-call-v1", &c.id, b"caller"]);
        let k_callee = mesh_crypto::hkdf32(&shared, &[b"meshtrike-call-v1", &c.id, b"callee"]);
        let (send, recv) = if c.outgoing { (k_caller, k_callee) } else { (k_callee, k_caller) };
        c.send_key = Some(SymKey(send));
        c.recv_key = Some(SymKey(recv));
        Ok(())
    }

    fn send_call_signal(&mut self, peer: &Id16, kind: CallSignalKind, id: [u8; 8], eph: [u8; 32], r: u8) {
        let now = self.now();
        let _ = self.send_control(
            peer,
            Content::CallSignal {
                kind,
                call_id: id,
                eph,
                reason: r,
            },
            now,
        );
    }

    /// A call signal from paired contact `peer` (already decrypted by Olm).
    pub(crate) fn on_call_signal(&mut self, peer: &Id16, sent_ms: u64, kind: CallSignalKind, id: [u8; 8], eph: [u8; 32], r: u8) {
        let now = self.now();
        if now.saturating_sub(sent_ms) > SIGNAL_MAX_AGE_MS && sent_ms < now {
            // Too old to ring (e.g. it waited while Meshtrike was locked).
            if kind == CallSignalKind::Offer {
                if let Ok(conv) = self.ensure_direct_conv(peer) {
                    self.add_call_note(conv, "Missed call", true);
                }
            }
            return;
        }
        match kind {
            CallSignalKind::Offer => {
                if self.settings().high_risk {
                    self.send_call_signal(peer, CallSignalKind::End, id, [0; 32], reason::UNAVAILABLE);
                    return;
                }
                if let Some(c) = &self.call {
                    if c.id != id {
                        self.send_call_signal(peer, CallSignalKind::End, id, [0; 32], reason::BUSY);
                    }
                    return;
                }
                let mut c = Call::new(id, *peer, false, CallState::Ringing, random_array(), Some(eph), now);
                c.link = self.call_link_kind(peer);
                self.call = Some(c);
                self.log("call: incoming");
                self.emit_call();
            }
            CallSignalKind::Answer => {
                let Some(c) = self.call.as_mut() else { return };
                if c.id != id || c.peer != *peer || !c.outgoing || c.state != CallState::Calling {
                    return;
                }
                if Self::derive_call_keys(c, &eph).is_err() {
                    let (p, cid) = (c.peer, c.id);
                    self.send_call_signal(&p, CallSignalKind::End, cid, [0; 32], reason::LOST);
                    self.finish_call(reason::LOST, true);
                    return;
                }
                c.activate(now);
                self.log("call: connected");
                self.emit_call();
            }
            CallSignalKind::End => {
                let Some(c) = self.call.as_ref() else { return };
                if c.id != id || c.peer != *peer {
                    return;
                }
                self.finish_call(r, false);
            }
        }
    }

    /// A `CallMedia` frame from link `p`.
    pub(crate) fn on_call_media(&mut self, p: PeerHandle, data: &[u8]) {
        let now = self.now();
        if data.len() < 16 + 16 {
            return;
        }
        let from = self.links.get(&p).and_then(|l| l.peer);
        let Some(c) = self.call.as_mut() else { return };
        if c.state != CallState::Active || from != Some(c.peer) || data[..8] != c.id {
            return;
        }
        let seq = u64::from_be_bytes(data[8..16].try_into().unwrap_or([0; 8]));
        // Replay protection: newer than the window and not seen in it.
        if seq + REPLAY_WINDOW <= c.recv_top {
            return;
        }
        if seq <= c.recv_top && c.recv_seen & (1u64 << (c.recv_top - seq)) != 0 {
            return;
        }
        let Some(key) = c.recv_key.clone() else { return };
        let dir = (!c.outgoing) as u8;
        let Ok(audio) = mesh_crypto::open(&key, &media_nonce(&c.id, seq), &[dir], &data[16..]) else {
            return;
        };
        if seq > c.recv_top {
            let shift = seq - c.recv_top;
            c.recv_seen = if shift >= 64 { 0 } else { c.recv_seen << shift };
            c.recv_top = seq;
        }
        c.recv_seen |= 1u64 << (c.recv_top - seq);
        c.last_media_ms = now;
        c.recv_got += 1;
        // An empty bundle (a single 0: no packets) only says "still here,
        // muted".
        if audio.as_slice() != [0] {
            self.host.tx.call_audio(audio);
        }
    }

    /// Ring timeout and lost calls.
    pub(crate) fn tick_call(&mut self, now: u64) {
        let Some(c) = self.call.as_ref() else { return };
        match c.state {
            CallState::Calling | CallState::Ringing if now.saturating_sub(c.started_ms) > CALL_RING_MS => {
                let (p, cid, out) = (c.peer, c.id, c.outgoing);
                if out {
                    self.send_call_signal(&p, CallSignalKind::End, cid, [0; 32], reason::NO_ANSWER);
                }
                self.finish_call(reason::NO_ANSWER, out);
            }
            CallState::Active
                if now.saturating_sub(c.last_media_ms) > CALL_LOST_MS && self.call_link(&c.peer).is_none() =>
            {
                let (p, cid) = (c.peer, c.id);
                self.send_call_signal(&p, CallSignalKind::End, cid, [0; 32], reason::LOST);
                self.finish_call(reason::LOST, true);
            }
            CallState::Active => self.measure_call(now),
            // Ringing: follow the link (the screen shows which one).
            _ => {
                let link = self.call_link_kind(&c.peer);
                if let Some(c) = self.call.as_mut().filter(|c| c.link != link && link.is_some()) {
                    c.link = link;
                    self.emit_call();
                }
            }
        }
    }

    /// Quality, recovery and link of an active call; the screen hears of
    /// any change.
    fn measure_call(&mut self, now: u64) {
        let link = {
            let Some(c) = self.call.as_ref() else { return };
            self.call_link_kind(&c.peer)
        };
        let Some(c) = self.call.as_mut() else { return };
        let mut changed = false;
        if link.is_some() && c.link != link {
            c.link = link;
            changed = true;
        }
        let quiet = now.saturating_sub(c.last_media_ms) > CALL_QUIET_MS;
        if quiet != c.reconnecting {
            c.reconnecting = quiet;
            changed = true;
        }
        if now.saturating_sub(c.sample_ms) >= QUALITY_EVERY_MS {
            // Sequence numbers say how many bundles were sent; the count
            // says how many arrived.
            let sent = c.recv_top.saturating_sub(c.sample_top);
            let got = c.recv_got.saturating_sub(c.sample_got);
            let share = if quiet || sent == 0 { 0 } else { (got.min(sent) * 100 / sent) as u8 };
            c.shares = [c.shares[1], share];
            c.sample_top = c.recv_top;
            c.sample_got = c.recv_got;
            c.sample_ms = now;
            let avg = (c.shares[0] as u16 + c.shares[1] as u16) / 2;
            let q = match avg {
                92.. => 4,
                75.. => 3,
                45.. => 2,
                1.. => 1,
                _ => 0,
            };
            if q != c.quality {
                c.quality = q;
                changed = true;
            }
        }
        if changed {
            self.emit_call();
        }
    }

    /// End the call: tell the UI, and note it in the chat.
    fn finish_call(&mut self, r: u8, by_us: bool) {
        let Some(c) = self.call.take() else { return };
        let now = self.now();
        let view = CallView {
            state: CallState::Ended,
            ..self.call_view(&c, Some(r))
        };
        let note = match (c.outgoing, c.answered_ms) {
            (_, Some(a)) => {
                let secs = now.saturating_sub(a) / 1000;
                let over = match c.link {
                    Some(TransportKind::WifiP2p) => " · over Wi-Fi",
                    Some(TransportKind::Ble) => " · over Bluetooth",
                    _ => "",
                };
                format!("Call · {}:{:02}{over}", secs / 60, secs % 60)
            }
            (true, None) => format!("Call not answered · {}", reason_text(r)),
            (false, None) if by_us && r == reason::DECLINED => "Call declined".to_string(),
            (false, None) => "Missed call".to_string(),
        };
        self.log(format!("call: ended ({})", reason_text(r)));
        self.emit(Event::CallChanged { call: view });
        if let Ok(conv) = self.ensure_direct_conv(&c.peer) {
            self.add_call_note(conv, &note, !c.outgoing && c.answered_ms.is_none());
        }
    }

    fn call_view(&self, c: &Call, r: Option<u8>) -> CallView {
        CallView {
            id: hex::encode(c.id),
            contact: c.peer,
            outgoing: c.outgoing,
            state: c.state,
            started_at: c.started_ms,
            answered_at: c.answered_ms,
            reason: r.map(|r| reason_text(r).to_string()),
            link: self.call_link_kind(&c.peer),
            quality: if c.state == CallState::Active { c.quality } else { 0 },
            reconnecting: c.state == CallState::Active && c.reconnecting,
        }
    }

    pub(crate) fn emit_call(&self) {
        if let Some(c) = &self.call {
            self.emit(Event::CallChanged {
                call: self.call_view(c, None),
            });
        }
    }

    /// A call note in a chat: shown as a centred pill, never sent.
    pub(crate) fn add_call_note(&mut self, conv: Id16, text: &str, unread: bool) {
        let now = self.now();
        let rec = MsgRec {
            uuid: random_array(),
            conversation: conv,
            outgoing: false,
            sender_peer: None,
            sender_pseudonym: None,
            kind: MessageKind::Call,
            body: text.as_bytes().to_vec(),
            file_name: None,
            total_size: None,
            chunks_have: 1,
            chunks_total: 1,
            state: DeliveryState::Received,
            release_at: None,
            sent_at: now,
            received_at: Some(now),
            expires_at: None,
            disappear_secs: 0,
            reply_tag: None,
            read: !unread,
            pending_envs: vec![],
            lora_sent: false,
        };
        self.store_msg(&rec);
        let view = self.view(&rec);
        self.emit(Event::MessageAdded {
            conversation: conv,
            message: view,
        });
        self.touch_conv(&conv, text, unread);
    }
}

fn media_nonce(id: &[u8; 8], seq: u64) -> [u8; 16] {
    let mut n = [0u8; 16];
    n[..8].copy_from_slice(id);
    n[8..].copy_from_slice(&seq.to_be_bytes());
    n
}
