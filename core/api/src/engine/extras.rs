//! Countermeasure features (section 12): communities, interference response,
//! QR handoff, device-integrity report.

use super::*;
use mesh_handoff::{HandoffReceiver, HandoffSender, QrMatrix};
use mesh_wire::*;

#[derive(Default)]
pub(crate) struct HandoffState {
    pub sender: Option<HandoffSender>,
    pub receiver: Option<HandoffReceiver>,
    pub delivered: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffProgress {
    pub received: u32,
    pub needed: u32,
    pub done: bool,
    pub envelopes: u32,
    pub error: Option<String>,
}

impl Engine {
    pub(crate) fn direct_only_map(&mut self) -> &mut HashMap<MessageId, Id16> {
        &mut self.direct_only
    }

    pub(crate) fn direct_only_target(&self, id: &MessageId) -> Option<Id16> {
        self.direct_only.get(id).copied()
    }

    // ===== Communities (12.1) =====

    pub fn communities(&self) -> Vec<Community> {
        self.relay
            .as_ref()
            .map(|r| {
                r.communities
                    .iter()
                    .map(|(id, c)| Community {
                        id: *id,
                        name: c.name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn add_community(&mut self, key: [u8; 32], name: String) -> CoreResult<Id16> {
        let now = self.now();
        let r = self.relay.as_mut().ok_or(CoreError::Locked)?;
        let id = mesh_crypto::hmac16(&key, &[b"community-id"]);
        if !r.communities.contains_key(&id) && r.communities.len() >= MAX_COMMUNITIES {
            return Err(CoreError::InvalidInput("max 4 communities".into()));
        }
        let rec = CommunityRec {
            name,
            key,
            joined_ms: now,
        };
        let _ = r.kv.apply(vec![records::put(t::COMMUNITY, &id, &rec)]);
        r.communities.insert(id, rec);
        self.refresh_radio();
        Ok(id)
    }

    pub fn create_community(&mut self, name: String) -> CoreResult<Id16> {
        self.require_unlocked()?;
        if name.is_empty() || name.len() > 64 {
            return Err(CoreError::InvalidInput("name".into()));
        }
        self.add_community(random_array(), name)
    }

    pub fn community_qr(&self, id: Id16) -> CoreResult<Vec<u8>> {
        self.require_unlocked()?;
        let c = self
            .relay
            .as_ref()
            .and_then(|r| r.communities.get(&id))
            .ok_or(CoreError::NotFound)?;
        Ok(mesh_identity::encode_community_qr(&c.key, &c.name))
    }

    pub fn join_community(&mut self, payload: Vec<u8>) -> CoreResult<Id16> {
        self.require_unlocked()?;
        let (key, name) = mesh_identity::decode_community_qr(&payload)
            .map_err(|_| CoreError::InvalidInput("not a community code".into()))?;
        self.add_community(key, name)
    }

    pub fn leave_community(&mut self, id: Id16) -> CoreResult<()> {
        let r = self.relay.as_mut().ok_or(CoreError::Locked)?;
        r.communities.remove(&id).ok_or(CoreError::NotFound)?;
        let _ = r.kv.delete(t::COMMUNITY, &id);
        if r.communities.is_empty() && r.settings.radio_mode == RadioMode::Community {
            r.settings.radio_mode = RadioMode::Public;
            let _ =
                r.kv.apply(vec![records::put(t::SETTINGS, b"s", &r.settings)]);
        }
        self.refresh_radio();
        Ok(())
    }

    // ===== Interference (12.2) =====

    pub fn on_radio_sample(&mut self, adverts_seen: u32, failed_connects: u32, stationary: bool) {
        let now = self.now();
        let attempts = std::mem::take(&mut self.connect_attempts).max(failed_connects);
        let failures = std::mem::take(&mut self.connect_failures).max(failed_connects);
        if let Some(state) =
            self.interference
                .sample(now, adverts_seen, attempts, failures, stationary)
        {
            self.emit(Event::Interference { suspected: state });
            if state {
                self.extend_own_queued();
            }
            self.emit_network();
        }
    }

    /// Re-seal our own queued envelopes with a 72-hour expiry under new
    /// message IDs (amendment A.5).
    pub(crate) fn extend_own_queued(&mut self) {
        let now = self.now();
        let new_expiry = self.expiry_for(now, EXPIRY_MAX_SECS);
        let own: Vec<Envelope> = self
            .relay
            .as_ref()
            .map(|r| {
                r.store
                    .iter()
                    .filter(|s| {
                        s.mine
                            && matches!(s.env.typ, EnvType::Direct | EnvType::Group)
                            && s.env.expiry < new_expiry
                    })
                    .map(|s| s.env.clone())
                    .collect()
            })
            .unwrap_or_default();
        for old in own {
            let Some(r) = self.relay.as_ref() else { return };
            // Find our outbound tag key for this envelope by trying each.
            let found = r.tagkeys.values().find_map(|rec| {
                let key = Self::env_key(rec, false);
                let tag_ok = match rec.kind {
                    TagKind::Pair => {
                        mesh_crypto::direct_tag(&SymKey(rec.key), rec.outbound_dir, old.expiry)
                            == old.tag
                    }
                    TagKind::Group => {
                        mesh_crypto::group_tag(&SymKey(rec.key), old.expiry) == old.tag
                    }
                    _ => false,
                };
                if !tag_ok {
                    return None;
                }
                open_payload(&old, &key)
                    .ok()
                    .map(|inner| (rec.clone(), key, inner))
            });
            let Some((rec, key, inner)) = found else {
                continue;
            };
            let message_id: MessageId = random_array();
            let mut env = Envelope {
                message_id,
                expiry: new_expiry,
                payload: vec![],
                ..old.clone()
            };
            match rec.kind {
                TagKind::Pair => {
                    env.tag =
                        mesh_crypto::direct_tag(&SymKey(rec.key), rec.outbound_dir, new_expiry);
                    env.proof = mesh_crypto::ack_proof(&mesh_crypto::ack_secret(&key, &message_id));
                }
                _ => env.tag = mesh_crypto::group_tag(&SymKey(rec.key), new_expiry),
            }
            if seal_into(&mut env, &key, &inner).is_err() {
                continue;
            }
            if let Some(uuid) = self
                .msg
                .as_ref()
                .and_then(|m| m.kv.get(t::OUTENV, &old.message_id).ok().flatten())
            {
                if let Some(ms) = self.msg.as_mut() {
                    let _ = ms.kv.apply(vec![
                        Op::Delete(t::OUTENV, old.message_id.to_vec()),
                        Op::Put(t::OUTENV, message_id.to_vec(), uuid.clone()),
                    ]);
                }
                if let Ok(u) = <[u8; 16]>::try_from(uuid.as_slice()) {
                    if let Some((_, mut m)) = self.load_msg(&u) {
                        m.pending_envs.retain(|e| *e != old.message_id);
                        m.pending_envs.push(message_id);
                        self.store_msg(&m);
                    }
                }
            }
            if let Some(t) = self.direct_only.remove(&old.message_id) {
                self.direct_only.insert(message_id, t);
            }
            self.remove_stored(&old.message_id, false);
            self.queue_own(env);
        }
    }

    // ===== QR handoff (12.2) =====

    pub fn handoff_start_send(&mut self) -> CoreResult<u32> {
        let now = self.now();
        let r = self.relay.as_ref().ok_or(CoreError::Locked)?;
        let mut items: Vec<&mesh_routing::Stored> = r
            .store
            .iter()
            .filter(|s| {
                s.env.expiry as u64 * 1000 > now && s.env.typ != EnvType::Cover && s.env.ttl > 1
                    || s.mine
            })
            .collect();
        items.sort_by(|a, b| b.mine.cmp(&a.mine).then(b.received_ms.cmp(&a.received_ms)));
        let envs: Vec<Vec<u8>> = items.iter().map(|s| s.env.encode()).collect();
        let bundle = mesh_handoff::build_bundle(&envs);
        let sender = HandoffSender::try_new(bundle, u32::from_be_bytes(random_array()))
            .map_err(|_| CoreError::Internal(50))?;
        let n = sender.source_packets() as u32;
        self.handoff.sender = Some(sender);
        Ok(n)
    }

    pub fn handoff_next_frame(&mut self) -> CoreResult<(Vec<u8>, QrMatrix)> {
        let s = self.handoff.sender.as_mut().ok_or(CoreError::NotFound)?;
        let f = s.next_frame();
        let qr = mesh_handoff::qr_encode_handoff(&f).map_err(|_| CoreError::Internal(51))?;
        Ok((f, qr))
    }

    pub fn handoff_start_receive(&mut self) {
        self.handoff.receiver = Some(HandoffReceiver::new());
        self.handoff.delivered = 0;
    }

    pub fn handoff_stop(&mut self) {
        self.handoff.sender = None;
        self.handoff.receiver = None;
    }

    pub fn handoff_push_frame(&mut self, frame: Vec<u8>) -> CoreResult<HandoffProgress> {
        let rx = self.handoff.receiver.as_mut().ok_or(CoreError::NotFound)?;
        let p = rx.push_frame(&frame);
        if let Some(envs) = p.envelopes {
            for e in envs {
                if self.ingest_raw_via(&e, Some(crate::types::Via::Qr)) {
                    self.handoff.delivered += 1;
                }
            }
        }
        Ok(HandoffProgress {
            received: p.received,
            needed: p.needed,
            done: p.done,
            envelopes: self.handoff.delivered,
            error: p.error,
        })
    }

    pub fn handoff_push_luma(
        &mut self,
        width: u32,
        height: u32,
        luma: Vec<u8>,
    ) -> CoreResult<HandoffProgress> {
        let frames = mesh_handoff::qr_decode_luma(width, height, &luma);
        let mut last = None;
        for f in frames {
            last = Some(self.handoff_push_frame(f)?);
        }
        match last {
            Some(p) => Ok(p),
            None => {
                let rx = self.handoff.receiver.as_ref().ok_or(CoreError::NotFound)?;
                let _ = rx;
                Ok(HandoffProgress {
                    received: 0,
                    needed: 0,
                    done: false,
                    envelopes: self.handoff.delivered,
                    error: None,
                })
            }
        }
    }

    /// Decode QR codes (pairing / community codes) from a camera frame in Rust.
    pub fn decode_qr_luma(&self, width: u32, height: u32, luma: Vec<u8>) -> Vec<Vec<u8>> {
        mesh_handoff::qr_decode_luma(width, height, &luma)
    }

    pub fn qr_matrix(&self, data: Vec<u8>) -> CoreResult<QrMatrix> {
        mesh_handoff::qr_encode(&data).map_err(|_| CoreError::InvalidInput("qr".into()))
    }

    // ===== Device integrity (12.3) =====

    /// Android: verify an attestation chain offline (see `integrity` module).
    /// iOS / web: the host passes None and its own jailbreak heuristics.
    pub fn integrity_report(&mut self, attestation_chain: Option<Vec<Vec<u8>>>) -> IntegrityStatus {
        let status = match attestation_chain {
            None => IntegrityStatus::Unknown,
            Some(chain) => crate::integrity::verify_chain(&chain),
        };
        if let IntegrityStatus::Warning(w) = &status {
            self.emit(Event::Warning {
                warning: Warning::DeviceIntegrity(w.clone()),
            });
        }
        status
    }
}

impl Engine {
    /// Diagnostic snapshot (no secrets): counts and short key fingerprints.
    #[doc(hidden)]
    pub fn debug_state(&self) -> String {
        let mut out = String::new();
        if let Some(ms) = &self.msg {
            for (id, g) in &ms.groups {
                out += &format!(
                    "group {} name={} tag={} members={} shared={} admin={} left={}\n",
                    &hex::encode(id)[..6],
                    g.rec.name,
                    &hex::encode(mesh_crypto::sha256(&[&g.rec.tag_key]))[..6],
                    g.rec.members.len(),
                    g.rec.shared_with.len(),
                    &hex::encode(g.rec.admin)[..6],
                    g.rec.left
                );
            }
            out += &format!(
                "megolm_in={} peers={}\n",
                ms.megolm_in.len(),
                ms.peers.len()
            );
        }
        for l in self.links.values() {
            out += &format!(
                "link kind={:?} up={} contact={} known={} caps={:?} queues={:?}\n",
                l.kind,
                l.is_up(),
                l.peer.is_some(),
                l.known.len(),
                l.beacon.as_ref().map(|b| b.capabilities),
                l.queues()
            );
        }
        if let Some(r) = &self.relay {
            for s in r.store.iter() {
                out += &format!(
                    "env {:?} mine={} for_me={} offer={} ttl={} len={}\n",
                    s.env.typ, s.mine, s.for_me, s.offer, s.env.ttl, s.raw_len
                );
            }
        }
        out
    }
}
