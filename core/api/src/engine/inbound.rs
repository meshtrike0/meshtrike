//! Envelope ingest (sections 7, 8, 15.4): tag recognition, relay storage,
//! acks, and dispatch of envelopes addressed to us.

use super::*;
use mesh_crypto::{direct_env_key, direct_tag, group_env_key, group_tag};
use mesh_routing::{AckOutcome, Insert, Stored};
use mesh_wire::*;

impl Engine {
    // ===== Tag keys =====

    pub(crate) fn add_tagkey(&mut self, id: Id16, rec: TagKeyRec) {
        if let Some(r) = self.relay.as_mut() {
            let _ = r.kv.apply(vec![records::put(t::TAGKEY, &id, &rec)]);
            r.tagkeys.insert(id, rec);
            r.tag_cache.clear();
        }
    }

    pub(crate) fn remove_tagkey(&mut self, id: &Id16) {
        if let Some(r) = self.relay.as_mut() {
            let _ = r.kv.delete(t::TAGKEY, id);
            r.tagkeys.remove(id);
            r.tag_cache.clear();
        }
    }

    /// Which of our tag keys produced `tag` for this expiry hour?
    pub(crate) fn lookup_tag(&mut self, tag: &Tag, expiry: u32) -> Option<(Id16, TagKeyRec)> {
        let r = self.relay.as_mut()?;
        if let Some((id, rec)) = r.tagkeys.iter().find(|(_, rec)| {
            matches!(
                rec.kind,
                TagKind::Channel | TagKind::Reply | TagKind::Return | TagKind::PairingIk
            ) && &rec.tag == tag
        }) {
            return Some((*id, rec.clone()));
        }
        let cache = r.tag_cache.entry(expiry).or_insert_with(|| {
            let mut m = HashMap::new();
            for (id, rec) in &r.tagkeys {
                let k = SymKey(rec.key);
                match rec.kind {
                    TagKind::Pair => {
                        m.insert(direct_tag(&k, rec.inbound_dir, expiry), *id);
                    }
                    TagKind::Group => {
                        m.insert(group_tag(&k, expiry), *id);
                    }
                    _ => {}
                }
            }
            m
        });
        let id = *cache.get(tag)?;
        r.tagkeys.get(&id).map(|rec| (id, rec.clone()))
    }

    pub(crate) fn tag_is_ours(&mut self, tag: &Tag, expiry: u32) -> bool {
        self.lookup_tag(tag, expiry).is_some()
    }

    pub(crate) fn env_key(rec: &TagKeyRec, inbound: bool) -> SymKey {
        let k = SymKey(rec.key);
        match rec.kind {
            TagKind::Pair => direct_env_key(
                &k,
                if inbound {
                    rec.inbound_dir
                } else {
                    rec.outbound_dir
                },
            ),
            TagKind::Group => group_env_key(&k),
            _ => k,
        }
    }

    // ===== Storage =====

    pub(crate) fn store_envelope(
        &mut self,
        env: Envelope,
        mine: bool,
        for_me: bool,
        offer: bool,
    ) -> bool {
        let now = self.now();
        let Some(r) = self.relay.as_mut() else {
            return false;
        };
        let raw = env.encode();
        let rec = EnvRec {
            raw: raw.clone(),
            received_ms: now,
            mine,
            for_me,
            offer,
        };
        let id = env.message_id;
        match r.store.insert(
            Stored {
                raw_len: raw.len(),
                env,
                received_ms: now,
                mine,
                for_me,
                offer,
            },
            now,
        ) {
            Insert::Stored { evicted } => {
                let mut ops = vec![records::put(t::ENV, &id, &rec)];
                ops.extend(evicted.into_iter().map(|e| Op::Delete(t::ENV, e.to_vec())));
                let _ = r.kv.apply(ops);
                true
            }
            Insert::Full => {
                if mine {
                    self.emit(Event::Warning {
                        warning: Warning::StorageFull,
                    });
                }
                false
            }
            _ => false,
        }
    }

    pub(crate) fn update_stored(&mut self, id: &MessageId, for_me: bool, offer: bool) {
        let Some(r) = self.relay.as_mut() else { return };
        let Some(s) = r.store.get_mut(id) else { return };
        s.for_me = for_me;
        s.offer = offer;
        let rec = EnvRec {
            raw: s.env.encode(),
            received_ms: s.received_ms,
            mine: s.mine,
            for_me,
            offer,
        };
        let _ = r.kv.apply(vec![records::put(t::ENV, id, &rec)]);
    }

    pub(crate) fn remove_stored(&mut self, id: &MessageId, mark_acked: bool) {
        let Some(r) = self.relay.as_mut() else { return };
        if let Some(s) = r.store.remove(id) {
            let mut ops = vec![Op::Delete(t::ENV, id.to_vec())];
            if mark_acked {
                r.store.restore_ack(*id, s.env.expiry);
                ops.push(records::put(t::ACK, id, &s.env.expiry));
            }
            let _ = r.kv.apply(ops);
        }
    }

    /// Create, store and announce one of our own envelopes.
    pub(crate) fn queue_own(&mut self, env: Envelope) -> bool {
        let id = env.message_id;
        if let Some(r) = self.relay.as_mut() {
            r.store.seen.insert(id, self.host.clock.now_ms());
        }
        let ok = self.store_envelope(env, true, false, true);
        if ok {
            self.announce(id, None);
            if self.effective_mode() == RadioMode::ListenOnly && self.links.is_empty() {
                let now = self.now();
                if self.listen_only.should_connect(now, true) {
                    let c: Vec<PeerHandle> = self.discovered.keys().copied().take(2).collect();
                    c.into_iter().for_each(|p| self.connect_to(p));
                }
            }
        }
        ok
    }

    // ===== Ingest =====

    /// An envelope arrived from a neighbour (`from`), LoRa or QR handoff.
    pub(crate) fn ingest(&mut self, env: Envelope, raw_len: usize, from: Option<PeerHandle>) {
        // Remember the carrier while this envelope is processed; a message it
        // creates records it. LoRa/QR callers set it before calling.
        if let Some(h) = from {
            self.ingest_via = self.links.get(&h).map(|l| crate::types::Via::from(l.kind));
        }
        self.ingest_inner(env, raw_len, from);
        self.ingest_via = None;
    }

    fn ingest_inner(&mut self, env: Envelope, _raw_len: usize, from: Option<PeerHandle>) {
        let now = self.now();
        let id = env.message_id;
        {
            let Some(r) = self.relay.as_mut() else { return };
            if r.store.contains(&id) || r.store.seen.contains(&id, now) {
                return;
            }
            if env.expiry as u64 * 1000 <= now
                || env.expiry as u64 > now / 1000 + EXPIRY_MAX_SECS + 3600
            {
                return;
            }
        }
        match env.typ {
            EnvType::Cover => return,
            EnvType::Ack => return self.ingest_ack(env, from),
            EnvType::Broadcast => {
                let bits = match self.cfg.pow_bits_override {
                    Some(b) => b,
                    None => self.broadcast_rate.required_bits(now),
                };
                let nonce = u64::from_be_bytes(env.proof);
                if !mesh_crypto::pow::pow_check(&env.pow_prefix(), nonce, &env.payload, bits) {
                    return;
                }
                self.broadcast_rate.record(now);
            }
            _ => {}
        }
        let hit = self.lookup_tag(&env.tag, env.expiry);
        let for_me = hit
            .as_ref()
            .is_some_and(|(_, rec)| open_payload(&env, &Self::env_key(rec, true)).is_ok());
        if let Some(r) = self.relay.as_mut() {
            r.store.seen.insert(id, now);
        }
        if !for_me && env.ttl <= 1 {
            return; // direct-neighbour content is never stored on relays
        }
        if !for_me && !self.settings().relay_enabled {
            return;
        }
        let neighbours = self.links.values().filter(|l| l.is_up()).count();
        let offer = match env.typ {
            EnvType::Broadcast => {
                mesh_routing::should_rebroadcast(neighbours, (self.rng.next() % 100) as u32)
            }
            EnvType::Direct | EnvType::Handshake if for_me => false,
            _ => env.ttl > 1,
        };
        let typ = env.typ;
        let hit_peer = hit.as_ref().map(|h| h.0);
        let rec = hit.map(|h| h.1);
        let env_copy = env.clone();
        if !self.store_envelope(env, false, for_me, offer) {
            return;
        }
        let kind = format!("{typ:?}").to_lowercase();
        let src = match (from, self.ingest_via) {
            (Some(h), _) => self.log.peer(h),
            (None, Some(crate::types::Via::Internet)) => "the internet route".into(),
            (None, Some(crate::types::Via::Lora)) => "LoRa".into(),
            (None, Some(crate::types::Via::Qr)) => "QR handoff".into(),
            (None, _) => "lora/handoff".into(),
        };
        self.log(format!(
            "stored {kind} envelope from {src}{}",
            if for_me { " (for us)" } else { "" }
        ));
        if for_me && matches!(typ, EnvType::Direct | EnvType::Group) && self.msg.is_none() {
            // Locked: we cannot tell content from control traffic; the host
            // throttles this to a generic "New message" notification.
            self.emit(Event::IncomingMessage);
        }
        if for_me && typ == EnvType::Direct {
            if let (Some(rec), Some(peer)) = (&rec, hit_peer) {
                self.send_ack(&env_copy, rec, &peer);
            }
        }
        if offer {
            self.announce(id, from);
        }
        if for_me && self.msg.is_some() {
            self.handle_for_me(&id);
        }
    }

    fn ingest_ack(&mut self, env: Envelope, from: Option<PeerHandle>) {
        let Ok((acked, secret)) = parse_ack(&env) else {
            return;
        };
        let now = self.now();
        let Some(r) = self.relay.as_mut() else { return };
        r.store.seen.insert(env.message_id, now);
        match r.store.apply_ack(&acked, &secret) {
            AckOutcome::Deleted { was_mine } => {
                let expiry = env.expiry;
                let _ = r.kv.apply(vec![
                    Op::Delete(t::ENV, acked.to_vec()),
                    records::put(t::ACK, &acked, &expiry),
                ]);
                let ack_id = env.message_id;
                if self.store_envelope(env, false, false, true) {
                    self.announce(ack_id, from);
                }
                if was_mine {
                    self.log("ack received for our message");
                    self.on_delivered(&acked);
                } else {
                    self.log("valid ack: relay copy deleted");
                }
            }
            AckOutcome::NotHeld | AckOutcome::Invalid => {}
        }
    }

    fn send_ack(&mut self, env: &Envelope, rec: &TagKeyRec, peer: &Id16) {
        let key = Self::env_key(rec, true);
        let secret = mesh_crypto::ack_secret(&key, &env.message_id);
        let ack = Envelope {
            typ: EnvType::Ack,
            ttl: self.rand_ttl(),
            flags: 0,
            message_id: random_array(),
            tag: random_array(),
            expiry: env.expiry,
            proof: [0; 8],
            payload: ack_payload(&env.message_id, &secret),
        };
        // The sender may only be reachable over the internet route.
        let net_copy = ack.clone();
        if self.queue_own(ack) {
            self.net_route(&net_copy, &[*peer]);
        }
    }

    /// Decrypt everything addressed to us that arrived while locked.
    pub(crate) fn process_for_me(&mut self) {
        let ids: Vec<MessageId> = self
            .relay
            .as_ref()
            .map(|r| {
                r.store
                    .iter()
                    .filter(|s| s.for_me)
                    .map(|s| s.env.message_id)
                    .collect()
            })
            .unwrap_or_default();
        // Verified contacts first (section 8: local priority only).
        let mut ids = ids;
        ids.sort_by_key(|id| {
            let verified = self
                .relay
                .as_ref()
                .and_then(|r| r.store.get(id))
                .map(|s| s.env.tag)
                .and_then(|tag| {
                    let exp = self.relay.as_ref()?.store.get(id)?.env.expiry;
                    let (_, rec) = self.lookup_tag(&tag, exp)?;
                    let peer = self.msg.as_ref()?.peers.get(&rec.owner)?;
                    Some(peer.trust == Trust::Verified)
                });
            !verified.unwrap_or(false)
        });
        for id in ids {
            self.handle_for_me(&id);
        }
    }

    pub(crate) fn handle_for_me(&mut self, id: &MessageId) {
        let Some(env) = self
            .relay
            .as_ref()
            .and_then(|r| r.store.get(id))
            .map(|s| s.env.clone())
        else {
            return;
        };
        let Some((_, rec)) = self.lookup_tag(&env.tag, env.expiry) else {
            self.update_stored(id, false, false);
            return;
        };
        let Ok(inner) = open_payload(&env, &Self::env_key(&rec, true)) else {
            self.crypto_failures += 1;
            self.remove_stored(id, false);
            return;
        };
        match env.typ {
            EnvType::Direct => {
                self.receive_direct(&rec.owner, &inner, &env);
                self.remove_stored(id, true);
            }
            EnvType::Group => {
                let done = self.receive_group(&rec.owner, &inner);
                let offer = env.ttl > 1;
                self.update_stored(id, !done, offer);
            }
            EnvType::Broadcast => {
                self.receive_post(&rec.owner, &inner, &env);
                let offer = self
                    .relay
                    .as_ref()
                    .and_then(|r| r.store.get(id))
                    .is_some_and(|s| s.offer);
                self.update_stored(id, false, offer);
            }
            EnvType::Handshake => {
                self.receive_handshake(&rec, &inner);
                self.remove_stored(id, false);
            }
            _ => {}
        }
    }

    /// Ingest a raw envelope from LoRa or QR handoff.
    pub fn ingest_raw(&mut self, bytes: &[u8]) -> bool {
        self.ingest_raw_via(bytes, None)
    }

    pub(crate) fn ingest_raw_via(&mut self, bytes: &[u8], via: Option<crate::types::Via>) -> bool {
        self.ingest_via = via;
        match Envelope::decode(bytes) {
            Ok(env) => {
                self.ingest(env, bytes.len(), None);
                true
            }
            Err(_) => false,
        }
    }

    pub fn on_lora_packet(&mut self, bytes: Vec<u8>) {
        if self.settings().high_risk {
            return;
        }
        self.ingest_raw_via(&bytes, Some(crate::types::Via::Lora));
    }
}
