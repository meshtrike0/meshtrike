//! One-to-one messaging (sections 6, 7, 15.3, 15.4): Olm sessions, outgoing
//! dispatch, delivery states, chunked media, disappearing messages, LoRa.

use super::*;
use mesh_crypto::vodozemac::olm::{OlmMessage, SessionConfig};
use mesh_crypto::vodozemac::Curve25519PublicKey;
use mesh_crypto::{direct_env_key, direct_tag};
use mesh_wire::*;

pub(crate) fn conv_id_for_peer(peer: &Id16) -> Id16 {
    mesh_crypto::hmac16(peer, &[b"conv-v1"])
}

impl Engine {
    pub(crate) fn ms(&mut self) -> CoreResult<&mut MsgState> {
        self.msg.as_mut().ok_or(CoreError::Locked)
    }

    // ===== Peers =====

    pub(crate) fn save_peer(&mut self, id: &Id16) {
        let Some(ms) = self.msg.as_mut() else { return };
        if let Some(p) = ms.peers.get(id) {
            let _ = ms.kv.apply(vec![records::put(t::PEER, id, p)]);
        }
    }

    /// Register a peer (contact or group member) and its relay-side keys.
    pub(crate) fn install_peer(&mut self, id: Id16, rec: PeerRec) {
        let Some(ms) = self.msg.as_mut() else { return };
        let my_ed = ms.me.ed25519();
        let inbound_dir = mesh_crypto::direction(&rec.ed25519, &my_ed);
        let outbound_dir = mesh_crypto::direction(&my_ed, &rec.ed25519);
        let listed = rec.listed;
        let presence = rec.presence_key;
        let tag_key = rec.tag_key;
        let _ = ms.kv.apply(vec![records::put(t::PEER, &id, &rec)]);
        ms.peers.insert(id, rec);
        self.add_tagkey(
            id,
            TagKeyRec {
                kind: TagKind::Pair,
                owner: id,
                key: tag_key,
                inbound_dir,
                outbound_dir,
                tag: [0; 16],
                created_ms: self.host.clock.now_ms(),
            },
        );
        if let Some(r) = self.relay.as_mut() {
            if listed {
                let _ = r.kv.apply(vec![records::put(t::PRESENCE, &id, &presence)]);
                r.presence.insert(id, presence);
            } else {
                let _ = r.kv.delete(t::PRESENCE, &id);
                r.presence.remove(&id);
            }
        }
        self.rerecognise_links();
    }

    pub(crate) fn peer_name(&self, id: &Id16) -> String {
        self.msg
            .as_ref()
            .and_then(|m| m.peers.get(id))
            .map(|p| mesh_media::sanitize_text(&p.nickname))
            .unwrap_or_else(|| "Unknown".into())
    }

    pub(crate) fn contact_view(&self, id: &Id16) -> Option<Contact> {
        let ms = self.msg.as_ref()?;
        let p = ms.peers.get(id)?;
        if !p.listed {
            return None;
        }
        Some(Contact {
            id: *id,
            nickname: mesh_media::sanitize_text(&p.nickname),
            trust: p.trust,
            identity_key: hex::encode(p.ed25519),
            nearby: self.links.values().any(|l| l.peer == Some(*id)),
            link: self
                .links
                .values()
                .filter(|l| l.peer == Some(*id) && l.is_up())
                .map(|l| l.kind)
                .max_by_key(|k| match k {
                    TransportKind::WifiP2p => 2,
                    TransportKind::Ble => 1,
                    TransportKind::Lora => 0,
                }),
            internet: self.net_usable() && self.net_address_of(id).is_some(),
            conversation: p.conversation,
        })
    }

    pub fn contacts(&self) -> Vec<Contact> {
        let Some(ms) = self.msg.as_ref() else {
            return vec![];
        };
        let mut v: Vec<Contact> = ms
            .peers
            .keys()
            .filter_map(|id| self.contact_view(id))
            .collect();
        v.sort_by_key(|c| c.nickname.to_lowercase());
        v
    }

    pub fn safety_number(&self, c: Id16) -> CoreResult<String> {
        let ms = self.msg.as_ref().ok_or(CoreError::Locked)?;
        let p = ms.peers.get(&c).ok_or(CoreError::NotFound)?;
        Ok(mesh_crypto::safety_number(&ms.me.ed25519(), &p.ed25519))
    }

    pub fn mark_verified(&mut self, c: Id16) -> CoreResult<()> {
        let ms = self.ms()?;
        let p = ms.peers.get_mut(&c).ok_or(CoreError::NotFound)?;
        if p.trust == Trust::KeyChanged {
            return Err(CoreError::TrustBlocked);
        }
        p.trust = Trust::Verified;
        self.save_peer(&c);
        if let Some(contact) = self.contact_view(&c) {
            self.emit(Event::ContactChanged { contact });
        }
        Ok(())
    }

    /// Remove a contact and rotate our presence key (section 5).
    pub fn remove_contact(&mut self, c: Id16) -> CoreResult<()> {
        self.net_forget_peer(&c);
        let ms = self.ms()?;
        let peer = ms.peers.get(&c).cloned().ok_or(CoreError::NotFound)?;
        let in_group = ms.groups.values().any(|g| g.rec.members.contains(&c));
        if let Some(conv) = peer.conversation {
            ms.convs.remove(&conv);
            let mut ops = vec![Op::Delete(t::CONV, conv.to_vec())];
            for (k, _) in ms.kv.scan_prefix(t::MSG, &conv).unwrap_or_default() {
                ops.push(Op::Delete(t::MSG, k));
            }
            let _ = ms.kv.apply(ops);
        }
        if in_group {
            let ms = self.ms()?;
            if let Some(p) = ms.peers.get_mut(&c) {
                p.listed = false;
                p.conversation = None;
            }
            let rec = ms.peers[&c].clone();
            self.install_peer(c, rec);
        } else {
            let ms = self.ms()?;
            ms.peers.remove(&c);
            ms.sessions.remove(&c);
            let mut ops = vec![Op::Delete(t::PEER, c.to_vec())];
            for (k, _) in ms.kv.scan_prefix(t::OLM, &c).unwrap_or_default() {
                ops.push(Op::Delete(t::OLM, k));
            }
            let _ = ms.kv.apply(ops);
            self.remove_tagkey(&c);
            if let Some(r) = self.relay.as_mut() {
                r.presence.remove(&c);
                let _ = r.kv.delete(t::PRESENCE, &c);
            }
        }
        self.rotate_presence_key()?;
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    pub(crate) fn rotate_presence_key(&mut self) -> CoreResult<()> {
        let new: [u8; 32] = random_array();
        let ms = self.ms()?;
        *ms.me.presence_key = new;
        let bytes = ms.me.to_bytes(&ms.pickle_key);
        let _ = ms.kv.put(t::IDENTITY, b"me", &bytes);
        let contacts: Vec<Id16> = ms
            .peers
            .iter()
            .filter(|(_, p)| p.listed)
            .map(|(k, _)| *k)
            .collect();
        if let Some(r) = self.relay.as_mut() {
            r.me.presence_key = new;
            let _ = r.kv.apply(vec![records::put(t::RELAY_ME, b"me", &r.me)]);
        }
        let now = self.now();
        for c in contacts {
            let _ = self.send_control(&c, Content::PresenceKey(new), now);
        }
        self.last_adv = None;
        self.refresh_radio();
        Ok(())
    }

    /// "Reset my sessions" (12.3): new Olm sessions, new Megolm sessions,
    /// new presence key.
    pub fn reset_sessions(&mut self) -> CoreResult<()> {
        let ms = self.ms()?;
        let peers: Vec<Id16> = ms.peers.keys().copied().collect();
        ms.sessions.clear();
        let olm_keys: Vec<Vec<u8>> = ms
            .kv
            .scan(t::OLM)
            .unwrap_or_default()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let _ = ms.kv.apply(
            olm_keys
                .into_iter()
                .map(|k| Op::Delete(t::OLM, k))
                .collect(),
        );
        let now = self.now();
        for p in peers {
            let _ = self.send_control(&p, Content::SessionInit, now);
        }
        let groups: Vec<Id16> = self.ms()?.groups.keys().copied().collect();
        for g in groups {
            self.rotate_megolm(&g, false);
        }
        self.rotate_presence_key()
    }

    // ===== Olm =====

    fn save_session(&mut self, peer: &Id16, idx: usize) {
        let Some(ms) = self.msg.as_mut() else { return };
        let Some(e) = ms.sessions.get(peer).and_then(|v| v.get(idx)) else {
            return;
        };
        let rec = OlmRec {
            pickle: e.session.pickle().encrypt(&ms.pickle_key),
            last_decrypt_ms: e.last_decrypt_ms,
            created_ms: e.created_ms,
        };
        let key = [peer.as_slice(), &e.id].concat();
        let _ = ms.kv.apply(vec![records::put(t::OLM, &key, &rec)]);
    }

    fn save_identity(&mut self) {
        if let Some(ms) = self.msg.as_mut() {
            let b = ms.me.to_bytes(&ms.pickle_key);
            let _ = ms.kv.put(t::IDENTITY, b"me", &b);
        }
    }

    /// Encrypt with the session that most recently decrypted, else the newest;
    /// create an outbound session from a one-time or fallback key if none.
    pub(crate) fn olm_encrypt(
        &mut self,
        peer: &Id16,
        plaintext: &[u8],
    ) -> CoreResult<(u8, Vec<u8>)> {
        let now = self.now();
        let ms = self.ms()?;
        let has = ms.sessions.get(peer).is_some_and(|v| !v.is_empty());
        if !has {
            let p = ms.peers.get_mut(peer).ok_or(CoreError::NotFound)?;
            let otk = if p.remote_otks.is_empty() {
                p.remote_fallback
            } else {
                p.remote_otks.remove(0)
            };
            let session = ms
                .me
                .account
                .create_outbound_session(
                    SessionConfig::version_1(),
                    Curve25519PublicKey::from_bytes(p.curve25519),
                    Curve25519PublicKey::from_bytes(otk),
                )
                .map_err(|_| CoreError::Crypto)?;
            ms.sessions.entry(*peer).or_default().push(OlmEntry {
                id: random_array(),
                session,
                last_decrypt_ms: 0,
                created_ms: now,
            });
            self.save_peer(peer);
        }
        let ms = self.ms()?;
        let v = ms.sessions.get_mut(peer).unwrap();
        let idx = (0..v.len())
            .max_by_key(|&i| (v[i].last_decrypt_ms, v[i].created_ms))
            .unwrap();
        let msg = v[idx]
            .session
            .encrypt(plaintext)
            .map_err(|_| CoreError::Crypto)?;
        self.save_session(peer, idx);
        let (typ, bytes) = msg.to_parts();
        Ok((typ as u8, bytes))
    }

    fn olm_decrypt(&mut self, peer: &Id16, typ: u8, bytes: &[u8]) -> Option<Vec<u8>> {
        let now = self.now();
        let msg = OlmMessage::from_parts(typ as usize, bytes).ok()?;
        let ms = self.msg.as_mut()?;
        let sessions = ms.sessions.entry(*peer).or_default();
        for (i, entry) in sessions.iter_mut().enumerate() {
            if let Ok(pt) = entry.session.decrypt(&msg) {
                entry.last_decrypt_ms = now;
                self.save_session(peer, i);
                return Some(pt);
            }
        }
        let OlmMessage::PreKey(pk) = &msg else {
            self.crypto_failures += 1;
            return None;
        };
        let curve = ms.peers.get(peer)?.curve25519;
        let res = match ms.me.account.create_inbound_session(
            SessionConfig::version_1(),
            Curve25519PublicKey::from_bytes(curve),
            pk,
        ) {
            Ok(r) => r,
            Err(_) => {
                self.crypto_failures += 1;
                return None;
            }
        };
        ms.sessions.entry(*peer).or_default().push(OlmEntry {
            id: random_array(),
            session: res.session,
            last_decrypt_ms: now,
            created_ms: now,
        });
        let idx = ms.sessions[peer].len() - 1;
        // One of our one-time keys was used; replenish when the peer holds < 5.
        let p = ms.peers.get_mut(peer)?;
        p.my_otks_held = p.my_otks_held.saturating_sub(1);
        let replenish = p.my_otks_held < MIN_REMOTE_OTKS as u32;
        self.save_identity();
        self.save_session(peer, idx);
        self.save_peer(peer);
        if replenish {
            let ms = self.msg.as_mut()?;
            let n = REPLENISH_REMOTE_OTKS - ms.peers[peer].my_otks_held as usize;
            let keys = ms.me.fresh_one_time_keys(n);
            ms.peers.get_mut(peer)?.my_otks_held += n as u32;
            self.save_identity();
            self.save_peer(peer);
            let _ = self.send_control(peer, Content::OneTimeKeys(keys), now);
        }
        Some(res.plaintext)
    }

    // ===== Building envelopes =====

    pub(crate) fn expiry_for(&self, now: u64, secs: u64) -> u32 {
        mesh_wire::expiry_hour(now / 1000 + secs.min(EXPIRY_MAX_SECS))
    }

    /// Encrypt one inner plaintext to a peer and queue it (returns envelope ID).
    pub(crate) fn send_direct(
        &mut self,
        peer: &Id16,
        ip: &InnerPlaintext,
        ttl: Option<u8>,
        lora: bool,
    ) -> CoreResult<MessageId> {
        let padded = if lora {
            ip.encode_padded(LORA_INNER_BUCKET)
                .map_err(|_| CoreError::InvalidInput("lora".into()))?
        } else {
            ip.encode_auto()
                .map_err(|_| CoreError::InvalidInput("too large".into()))?
        };
        let (typ, olm) = self.olm_encrypt(peer, &padded)?;
        if lora && typ != 1 {
            return Err(CoreError::InvalidInput(
                "lora needs an established session".into(),
            ));
        }
        self.queue_direct_inner(peer, &direct_inner(typ, &olm), ttl, lora)
    }

    /// Seal an already-built inner message to a peer and queue (or LoRa-send) it.
    pub(crate) fn queue_direct_inner(
        &mut self,
        peer: &Id16,
        inner: &[u8],
        ttl: Option<u8>,
        lora: bool,
    ) -> CoreResult<MessageId> {
        let now = self.now();
        let rec = self
            .relay
            .as_ref()
            .and_then(|r| r.tagkeys.get(peer))
            .cloned()
            .ok_or(CoreError::NotFound)?;
        let key = SymKey(rec.key);
        let expiry = self.expiry_for(now, EXPIRY_DEFAULT_SECS);
        let message_id: MessageId = random_array();
        let env_key = direct_env_key(&key, rec.outbound_dir);
        let mut env = Envelope {
            typ: EnvType::Direct,
            ttl: ttl.unwrap_or_else(|| self.rand_ttl()),
            flags: if lora { FLAG_LORA } else { 0 },
            message_id,
            tag: direct_tag(&key, rec.outbound_dir, expiry),
            expiry,
            proof: mesh_crypto::ack_proof(&mesh_crypto::ack_secret(&env_key, &message_id)),
            payload: vec![],
        };
        seal_into(&mut env, &env_key, inner).map_err(|_| CoreError::Internal(20))?;
        if lora {
            let bytes = env.encode();
            return if self.host.tx.lora_send(bytes) {
                Ok(message_id)
            } else {
                Err(CoreError::Transport("lora".into()))
            };
        }
        let net_copy = env.clone();
        if !self.queue_own(env) {
            return Err(CoreError::StorageFull);
        }
        // The same sealed envelope also goes over the internet route, if the
        // contact shared an address (the receiver drops whichever copy is second).
        self.net_route(&net_copy, &[*peer]);
        Ok(message_id)
    }

    /// Control messages (session init, keys, receipts) to one peer.
    pub(crate) fn send_control(
        &mut self,
        peer: &Id16,
        c: Content,
        now: u64,
    ) -> CoreResult<MessageId> {
        let content = c.encode().map_err(|_| CoreError::Internal(21))?;
        let ip = InnerPlaintext::new(c.kind(), random_array(), now, content);
        self.send_direct(peer, &ip, None, false)
    }

    // ===== Conversations =====

    pub(crate) fn ensure_direct_conv(&mut self, peer: &Id16) -> CoreResult<Id16> {
        let now = self.now();
        let disappear = self.settings().disappear_default_secs;
        let ms = self.ms()?;
        let conv = conv_id_for_peer(peer);
        if !ms.convs.contains_key(&conv) {
            let title = ms
                .peers
                .get(peer)
                .map(|p| p.nickname.clone())
                .unwrap_or_default();
            let rec = ConvRec {
                kind: ConversationKind::Direct,
                peer: Some(*peer),
                title,
                disappear_secs: disappear,
                last_activity: now,
                unread: 0,
                last_preview: String::new(),
            };
            let _ = ms.kv.apply(vec![records::put(t::CONV, &conv, &rec)]);
            ms.convs.insert(conv, rec);
        }
        if let Some(p) = ms.peers.get_mut(peer) {
            if p.conversation != Some(conv) {
                p.conversation = Some(conv);
                self.save_peer(peer);
            }
        }
        Ok(conv)
    }

    pub(crate) fn save_conv(&mut self, id: &Id16) {
        let Some(ms) = self.msg.as_mut() else { return };
        if let Some(c) = ms.convs.get(id) {
            let _ = ms.kv.apply(vec![records::put(t::CONV, id, c)]);
        }
    }

    pub fn conversations(&self) -> Vec<ConversationSummary> {
        let Some(ms) = self.msg.as_ref() else {
            return vec![];
        };
        let my_ed = ms.me.ed25519();
        let mut v: Vec<ConversationSummary> = ms
            .convs
            .iter()
            .map(|(id, c)| ConversationSummary {
                id: *id,
                kind: c.kind,
                title: mesh_media::sanitize_text(&match c.kind {
                    ConversationKind::Direct => c
                        .peer
                        .and_then(|p| ms.peers.get(&p))
                        .map(|p| p.nickname.clone())
                        .unwrap_or(c.title.clone()),
                    // The UI adds the "#".
                    ConversationKind::Channel => c.title.clone(),
                    ConversationKind::Group => c.title.clone(),
                }),
                unread: c.unread,
                last_activity: c.last_activity,
                last_preview: mesh_media::sanitize_text(&c.last_preview),
                trust: c.peer.and_then(|p| ms.peers.get(&p)).map(|p| p.trust),
                disappear_secs: c.disappear_secs,
                is_admin: ms.groups.get(id).is_some_and(|g| g.rec.admin == my_ed),
                my_handle: ms
                    .channels
                    .get(id)
                    .map(|ch| pseudonym_name(&mesh_crypto::Ed25519Seed(ch.pseudonym_seed).public())),
            })
            .collect();
        v.sort_by_key(|c| std::cmp::Reverse(c.last_activity));
        v
    }

    pub fn set_disappearing(&mut self, conv: Id16, secs: u32) -> CoreResult<()> {
        if !(DISAPPEARING_MIN_SECS..=DISAPPEARING_MAX_SECS).contains(&secs) {
            return Err(CoreError::InvalidInput("secs".into()));
        }
        let ms = self.ms()?;
        ms.convs
            .get_mut(&conv)
            .ok_or(CoreError::NotFound)?
            .disappear_secs = secs;
        self.save_conv(&conv);
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    // ===== Message records =====

    pub(crate) fn load_msg(&self, uuid: &Id16) -> Option<(Vec<u8>, MsgRec)> {
        let ms = self.msg.as_ref()?;
        let key = ms.kv.get(t::MSGIDX, uuid).ok()??;
        let rec = records::load::<MsgRec>(&*ms.kv, t::MSG, &key)?;
        Some((key, rec))
    }

    pub(crate) fn store_msg(&mut self, rec: &MsgRec) {
        let Some(ms) = self.msg.as_mut() else { return };
        let key = msg_key(&rec.conversation, rec.sent_at, &rec.uuid);
        let mut ops = vec![
            records::put(t::MSG, &key, rec),
            Op::Put(t::MSGIDX, rec.uuid.to_vec(), key),
        ];
        let pending = matches!(
            rec.state,
            DeliveryState::Held | DeliveryState::Queued | DeliveryState::Sent
        );
        if rec.outgoing && pending {
            ops.push(Op::Put(t::PENDING_OUT, rec.uuid.to_vec(), vec![]));
        } else {
            ops.push(Op::Delete(t::PENDING_OUT, rec.uuid.to_vec()));
        }
        let _ = ms.kv.apply(ops);
    }

    fn delete_msg_rec(&mut self, rec: &MsgRec) {
        let Some(ms) = self.msg.as_mut() else { return };
        let key = msg_key(&rec.conversation, rec.sent_at, &rec.uuid);
        let mut ops = vec![
            Op::Delete(t::MSG, key),
            Op::Delete(t::MSGIDX, rec.uuid.to_vec()),
            Op::Delete(t::PENDING_OUT, rec.uuid.to_vec()),
            Op::Delete(t::MSGVIA, rec.uuid.to_vec()),
            Op::Delete(t::MSGREPLY, rec.uuid.to_vec()),
            Op::Delete(t::MSGVOICE, rec.uuid.to_vec()),
        ];
        for (k, _) in ms.kv.scan_prefix(t::CHUNK, &rec.uuid).unwrap_or_default() {
            ops.push(Op::Delete(t::CHUNK, k));
        }
        for e in &rec.pending_envs {
            ops.push(Op::Delete(t::OUTENV, e.to_vec()));
        }
        let _ = ms.kv.apply(ops);
    }

    pub(crate) fn set_msg_via(&mut self, uuid: &Id16, via: crate::types::Via) {
        let Some(ms) = self.msg.as_mut() else { return };
        if let Ok(b) = serde_json::to_vec(&via) {
            let _ = ms.kv.put(t::MSGVIA, uuid, &b);
        }
    }

    fn msg_via(&self, uuid: &Id16) -> Option<crate::types::Via> {
        let ms = self.msg.as_ref()?;
        let b = ms.kv.get(t::MSGVIA, uuid).ok()??;
        serde_json::from_slice(&b).ok()
    }

    pub(crate) fn view(&self, r: &MsgRec) -> Message {
        let ms = self.msg.as_ref();
        let (sender, verified) = if r.outgoing {
            ("You".to_string(), true)
        } else if let Some(p) = r.sender_peer.and_then(|p| ms.and_then(|m| m.peers.get(&p))) {
            (
                mesh_media::sanitize_text(&p.nickname),
                p.trust == Trust::Verified,
            )
        } else if let Some(pk) = r.sender_pseudonym {
            (pseudonym_name(&pk), false)
        } else {
            ("Unknown".into(), false)
        };
        let text = match r.kind {
            MessageKind::Text | MessageKind::System | MessageKind::Call => {
                mesh_media::sanitize_text(&String::from_utf8_lossy(&r.body))
            }
            MessageKind::File => r.file_name.clone().unwrap_or_else(|| "file".into()),
            MessageKind::Image | MessageKind::Voice => String::new(),
        };
        Message {
            id: r.uuid,
            conversation: r.conversation,
            outgoing: r.outgoing,
            sender,
            sender_verified: verified,
            kind: r.kind,
            text,
            file_size: r.total_size,
            chunks_have: r.chunks_have,
            chunks_total: r.chunks_total,
            state: r.state,
            sent_at: r.sent_at,
            received_at: r.received_at,
            expires_at: r.expires_at,
            release_at: r.release_at,
            can_start_chat: r.reply_tag.is_some() && !r.outgoing,
            via: self.msg_via(&r.uuid),
            reply: self.reply_view(&r.uuid),
            voice: (r.kind == MessageKind::Voice)
                .then(|| self.voice_rec(&r.uuid))
                .flatten()
                .map(|v| VoiceView {
                    duration_ms: v.duration_ms,
                    waveform: v.waveform,
                    listened: r.outgoing || v.listened,
                }),
        }
    }

    // ===== Voice notes =====

    fn voice_rec(&self, uuid: &Id16) -> Option<VoiceRec> {
        let ms = self.msg.as_ref()?;
        records::load::<VoiceRec>(&*ms.kv, t::MSGVOICE, uuid)
    }

    fn put_voice(&mut self, uuid: &Id16, v: &VoiceRec) {
        if let Some(ms) = self.msg.as_mut() {
            let _ = ms.kv.apply(vec![records::put(t::MSGVOICE, uuid, v)]);
        }
    }

    /// Send a voice note recorded as Ogg Opus. Only the audio packets are
    /// kept (no container, no tags); `waveform` is up to 64 bars for display.
    pub fn send_voice(
        &mut self,
        conv: Id16,
        ogg: Vec<u8>,
        waveform: Vec<u8>,
        reply_to: Option<Id16>,
        opts: SendOptions,
    ) -> CoreResult<Id16> {
        let c = self.check_send(&conv)?;
        if self.settings().high_risk {
            return Err(CoreError::InvalidInput("High-Risk mode is text only".into()));
        }
        if c.kind == ConversationKind::Channel {
            return Err(CoreError::InvalidInput("channels are text only".into()));
        }
        let clip = mesh_media::voice::VoiceClip::from_ogg(&ogg)
            .map_err(|_| CoreError::InvalidInput("not a voice recording".into()))?;
        let blob = clip.to_blob();
        if blob.len() > MAX_VOICE_BYTES {
            return Err(CoreError::InvalidInput("voice note too long".into()));
        }
        if c.kind != ConversationKind::Direct && blob.len() > LARGE_CONTENT_BYTES {
            return Err(CoreError::InvalidInput(
                "voice notes over 256 KB go to one contact only".into(),
            ));
        }
        let waveform: Vec<u8> = waveform.into_iter().take(MAX_WAVEFORM_BARS).collect();
        let quote = match reply_to {
            Some(to) => {
                let (_, orig) = self.load_msg(&to).ok_or(CoreError::NotFound)?;
                if orig.conversation != conv {
                    return Err(CoreError::InvalidInput("reply to a message of this chat".into()));
                }
                Some((to, self.quote_of(&orig)))
            }
            None => None,
        };
        let mut rec = self.new_outgoing(&c, conv, MessageKind::Voice, blob, None, opts);
        rec.total_size = Some(rec.body.len() as u64);
        rec.chunks_total = chunk_voice(&waveform, quote.clone(), &rec.body).len() as u32;
        rec.chunks_have = rec.chunks_total;
        self.put_voice(
            &rec.uuid,
            &VoiceRec {
                duration_ms: clip.duration_ms(),
                waveform,
                listened: true,
            },
        );
        if let Some((to, q)) = quote {
            self.put_reply(&rec.uuid, to, q);
        }
        self.finish_new_outgoing(rec)
    }

    /// A voice note as a fresh Ogg Opus file for the platform player, built
    /// here from validated packets (never the other phone's bytes as sent).
    pub fn voice_audio(&self, m: Id16) -> CoreResult<Vec<u8>> {
        let (_, rec) = self.load_msg(&m).ok_or(CoreError::NotFound)?;
        if rec.kind != MessageKind::Voice || rec.chunks_have != rec.chunks_total {
            return Err(CoreError::NotFound);
        }
        let clip = mesh_media::voice::VoiceClip::from_blob(&rec.body).map_err(|_| CoreError::Crypto)?;
        Ok(clip.to_ogg())
    }

    /// Played on this phone: the bubble stops marking it new.
    pub fn voice_listened(&mut self, m: Id16) -> CoreResult<()> {
        let mut v = self.voice_rec(&m).ok_or(CoreError::NotFound)?;
        if !v.listened {
            v.listened = true;
            self.put_voice(&m, &v);
            if let Some((_, rec)) = self.load_msg(&m) {
                let view = self.view(&rec);
                self.emit(Event::MessageAdded {
                    conversation: rec.conversation,
                    message: view,
                });
            }
        }
        Ok(())
    }

    // ===== Replies =====

    fn reply_rec(&self, uuid: &Id16) -> Option<ReplyRec> {
        let ms = self.msg.as_ref()?;
        records::load::<ReplyRec>(&*ms.kv, t::MSGREPLY, uuid)
    }

    fn reply_view(&self, uuid: &Id16) -> Option<ReplyView> {
        let mut r = self.reply_rec(uuid)?;
        let found = self.msg_exists(&r.to);
        // The original arrived after the reply: learn its author now.
        if found && !r.found {
            if let Some((_, o)) = self.load_msg(&r.to) {
                r.author_me = o.outgoing;
                r.author_peer = o.sender_peer;
                r.author_pseudonym = o.sender_pseudonym;
            }
        }
        let author = if r.author_me {
            Some("You".to_string())
        } else if let Some(p) = r.author_peer {
            self.msg
                .as_ref()
                .and_then(|m| m.peers.get(&p))
                .map(|p| mesh_media::sanitize_text(&p.nickname))
        } else {
            r.author_pseudonym.map(|pk| pseudonym_name(&pk))
        };
        Some(ReplyView {
            to: r.to,
            quote: mesh_media::sanitize_text(&r.quote),
            author,
            found,
        })
    }

    fn msg_exists(&self, uuid: &Id16) -> bool {
        self.msg
            .as_ref()
            .is_some_and(|m| m.kv.get(t::MSGIDX, uuid).ok().flatten().is_some())
    }

    /// What a reply shows of the message it answers: a text snippet, the
    /// file's name, or the kind of media. Cut at a character boundary within
    /// `MAX_QUOTE_BYTES`.
    fn quote_of(&self, rec: &MsgRec) -> String {
        let raw = match rec.kind {
            MessageKind::Text | MessageKind::System | MessageKind::Call => {
                String::from_utf8_lossy(&rec.body).to_string()
            }
            MessageKind::Image => "Photo".into(),
            MessageKind::File => rec.file_name.clone().unwrap_or_else(|| "file".into()),
            MessageKind::Voice => {
                let ms = self.voice_rec(&rec.uuid).map_or(0, |v| v.duration_ms);
                let t = ms.div_ceil(1000);
                format!("Voice note · {}:{:02}", t / 60, t % 60)
            }
        };
        let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let budget = MAX_QUOTE_BYTES - '…'.len_utf8();
        if one_line.len() <= MAX_QUOTE_BYTES {
            return one_line;
        }
        let mut end = budget;
        while !one_line.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &one_line[..end])
    }

    /// Record what `uuid` answers, resolving the original's author here.
    fn put_reply(&mut self, uuid: &Id16, to: Id16, quote: String) {
        let orig = self.load_msg(&to).map(|x| x.1);
        let rec = ReplyRec {
            to,
            quote,
            found: orig.is_some(),
            author_me: orig.as_ref().is_some_and(|o| o.outgoing),
            author_peer: orig.as_ref().and_then(|o| o.sender_peer),
            author_pseudonym: orig.as_ref().and_then(|o| o.sender_pseudonym),
        };
        if let Some(ms) = self.msg.as_mut() {
            let _ = ms.kv.apply(vec![records::put(t::MSGREPLY, uuid, &rec)]);
        }
    }

    /// Swipe to reply: a text answering message `reply_to` of this chat.
    pub fn send_text_reply(
        &mut self,
        conv: Id16,
        text: String,
        reply_to: Id16,
        opts: SendOptions,
    ) -> CoreResult<Id16> {
        let c = self.check_send(&conv)?;
        if text.is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(CoreError::InvalidInput(format!(
                "text must be 1–{MAX_TEXT_BYTES} bytes"
            )));
        }
        let (_, orig) = self.load_msg(&reply_to).ok_or(CoreError::NotFound)?;
        if orig.conversation != conv {
            return Err(CoreError::InvalidInput("reply to a message of this chat".into()));
        }
        let quote = self.quote_of(&orig);
        let rec = self.new_outgoing(&c, conv, MessageKind::Text, text.into_bytes(), None, opts);
        self.put_reply(&rec.uuid, reply_to, quote);
        self.finish_new_outgoing(rec)
    }

    /// Messages of a conversation, oldest first. Displaying starts the
    /// disappearing timer of incoming messages (15.4 step 3).
    pub fn messages(
        &mut self,
        conv: Id16,
        before: Option<Id16>,
        limit: u32,
    ) -> CoreResult<Vec<Message>> {
        let now = self.now();
        let ms = self.ms()?;
        let rows = ms
            .kv
            .scan_prefix(t::MSG, &conv)
            .map_err(|_| CoreError::Internal(30))?;
        let mut recs: Vec<MsgRec> = rows.iter().filter_map(|(_, v)| records::dec(v)).collect();
        if let Some(b) = before {
            if let Some(pos) = recs.iter().position(|r| r.uuid == b) {
                recs.truncate(pos);
            }
        }
        let start = recs.len().saturating_sub(limit as usize);
        let mut recs = recs.split_off(start);
        for r in recs.iter_mut() {
            if !r.outgoing
                && r.expires_at.is_none()
                && r.disappear_secs > 0
                && r.chunks_have == r.chunks_total
            {
                r.expires_at = Some(now + r.disappear_secs as u64 * 1000);
                let rc = r.clone();
                self.store_msg(&rc);
            }
        }
        Ok(recs.iter().map(|r| self.view(r)).collect())
    }

    pub(crate) fn touch_conv(&mut self, conv: &Id16, preview: &str, incoming: bool) {
        let now = self.now();
        let Some(ms) = self.msg.as_mut() else { return };
        if let Some(c) = ms.convs.get_mut(conv) {
            c.last_activity = now;
            c.last_preview = preview.chars().take(80).collect();
            if incoming {
                c.unread += 1;
            }
        }
        self.save_conv(conv);
        self.emit(Event::ConversationsChanged);
    }

    // ===== Sending =====

    fn check_send(&mut self, conv: &Id16) -> CoreResult<ConvRec> {
        let ms = self.ms()?;
        let c = ms.convs.get(conv).cloned().ok_or(CoreError::NotFound)?;
        if let Some(p) = c.peer {
            if ms
                .peers
                .get(&p)
                .is_some_and(|p| p.trust == Trust::KeyChanged)
            {
                return Err(CoreError::TrustBlocked);
            }
        }
        if let Some(g) = ms.groups.get(conv) {
            if g.rec.left {
                return Err(CoreError::InvalidInput("left group".into()));
            }
        }
        Ok(c)
    }

    fn new_outgoing(
        &mut self,
        conv: &ConvRec,
        conv_id: Id16,
        kind: MessageKind,
        body: Vec<u8>,
        file_name: Option<String>,
        opts: SendOptions,
    ) -> MsgRec {
        let now = self.now();
        let release = match opts.when {
            SendWhen::Now => Some(now),
            SendWhen::In10Min => Some(now + 10 * 60_000),
            SendWhen::In30Min => Some(now + 30 * 60_000),
            SendWhen::In2Hours => Some(now + 2 * 3_600_000),
            SendWhen::Hold => None,
        };
        let disappear = if opts.expiry_secs > 0 {
            opts.expiry_secs
        } else {
            conv.disappear_secs
        };
        let total = body.len() as u64;
        MsgRec {
            uuid: random_array(),
            conversation: conv_id,
            outgoing: true,
            sender_peer: None,
            sender_pseudonym: None,
            kind,
            body,
            file_name,
            total_size: (kind == MessageKind::File).then_some(total),
            chunks_have: 1,
            chunks_total: 1,
            state: DeliveryState::Held,
            release_at: release,
            sent_at: now,
            received_at: None,
            expires_at: None,
            disappear_secs: disappear,
            reply_tag: None,
            read: true,
            pending_envs: vec![],
            lora_sent: false,
        }
    }

    fn finish_new_outgoing(&mut self, mut rec: MsgRec) -> CoreResult<Id16> {
        let now = self.now();
        let preview = match rec.kind {
            MessageKind::Text => String::from_utf8_lossy(&rec.body).to_string(),
            MessageKind::Image => "Photo".into(),
            MessageKind::Voice => "Voice message".into(),
            _ => rec.file_name.clone().unwrap_or_default(),
        };
        if rec.release_at.is_some_and(|t| t <= now) {
            rec.release_at = None;
            self.dispatch(&mut rec)?;
        } else {
            self.store_msg(&rec);
        }
        let conv = rec.conversation;
        self.emit(Event::MessageAdded {
            conversation: conv,
            message: self.view(&rec),
        });
        self.touch_conv(&conv, &preview, false);
        Ok(rec.uuid)
    }

    pub fn send_text(&mut self, conv: Id16, text: String, opts: SendOptions) -> CoreResult<Id16> {
        let c = self.check_send(&conv)?;
        if text.is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(CoreError::InvalidInput(format!(
                "text must be 1–{MAX_TEXT_BYTES} bytes"
            )));
        }
        let rec = self.new_outgoing(&c, conv, MessageKind::Text, text.into_bytes(), None, opts);
        self.finish_new_outgoing(rec)
    }

    pub fn send_image(
        &mut self,
        conv: Id16,
        rgba: Vec<u8>,
        w: u32,
        h: u32,
        opts: SendOptions,
    ) -> CoreResult<Id16> {
        let c = self.check_send(&conv)?;
        if self.settings().high_risk {
            return Err(CoreError::InvalidInput(
                "High-Risk mode is text only".into(),
            ));
        }
        let jpeg = mesh_media::encode_photo(&rgba, w, h)
            .map_err(|e| CoreError::InvalidInput(format!("image: {e}")))?;
        self.send_jpeg(conv, &c, jpeg, opts)
    }

    /// Send already-encoded baseline JPEG (re-validated in Rust).
    pub fn send_jpeg_bytes(
        &mut self,
        conv: Id16,
        jpeg: Vec<u8>,
        opts: SendOptions,
    ) -> CoreResult<Id16> {
        let c = self.check_send(&conv)?;
        if self.settings().high_risk {
            return Err(CoreError::InvalidInput(
                "High-Risk mode is text only".into(),
            ));
        }
        let img = mesh_media::decode_jpeg(&jpeg)
            .map_err(|e| CoreError::InvalidInput(format!("image: {e}")))?;
        let jpeg = mesh_media::encode_photo(&img.rgba, img.width, img.height)
            .map_err(|e| CoreError::InvalidInput(format!("image: {e}")))?;
        self.send_jpeg(conv, &c, jpeg, opts)
    }

    fn send_jpeg(
        &mut self,
        conv: Id16,
        c: &ConvRec,
        jpeg: Vec<u8>,
        opts: SendOptions,
    ) -> CoreResult<Id16> {
        if c.kind == ConversationKind::Channel {
            return Err(CoreError::InvalidInput("channels are text only".into()));
        }
        let mut rec = self.new_outgoing(c, conv, MessageKind::Image, jpeg, None, opts);
        rec.chunks_total = rec.body.len().div_ceil(IMAGE_CHUNK_BYTES) as u32;
        rec.chunks_have = rec.chunks_total;
        self.finish_new_outgoing(rec)
    }

    pub fn send_file(
        &mut self,
        conv: Id16,
        name: String,
        bytes: Vec<u8>,
        opts: SendOptions,
    ) -> CoreResult<Id16> {
        let c = self.check_send(&conv)?;
        if self.settings().high_risk {
            return Err(CoreError::InvalidInput(
                "High-Risk mode is text only".into(),
            ));
        }
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err(CoreError::InvalidInput("file must be ≤ 5 MB".into()));
        }
        if c.kind != ConversationKind::Direct && bytes.len() > LARGE_CONTENT_BYTES {
            return Err(CoreError::InvalidInput(
                "files over 256 KB go to one contact only".into(),
            ));
        }
        if c.kind == ConversationKind::Channel {
            return Err(CoreError::InvalidInput("channels are text only".into()));
        }
        let name = mesh_media::sanitize_filename(&name);
        let mut rec = self.new_outgoing(&c, conv, MessageKind::File, bytes, Some(name), opts);
        rec.chunks_total =
            chunk_file(rec.file_name.as_deref().unwrap_or("file"), &rec.body).len() as u32;
        rec.chunks_have = rec.chunks_total;
        self.finish_new_outgoing(rec)
    }

    pub fn release_held(&mut self, m: Id16) -> CoreResult<()> {
        let (_, mut rec) = self.load_msg(&m).ok_or(CoreError::NotFound)?;
        if rec.state != DeliveryState::Held {
            return Err(CoreError::InvalidInput("not held".into()));
        }
        rec.release_at = None;
        self.dispatch(&mut rec)
    }

    pub(crate) fn release_due_held(&mut self) {
        let now = self.now();
        let Some(ms) = self.msg.as_ref() else { return };
        let ids: Vec<Id16> = ms
            .kv
            .scan(t::PENDING_OUT)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, _)| k.try_into().ok())
            .collect();
        for id in ids {
            if let Some((_, mut rec)) = self.load_msg(&id) {
                if rec.state == DeliveryState::Held && rec.release_at.is_some_and(|t| t <= now) {
                    rec.release_at = None;
                    let _ = self.dispatch(&mut rec);
                }
            }
        }
    }

    /// Turn a message into envelopes (Held → Queued).
    pub(crate) fn dispatch(&mut self, rec: &mut MsgRec) -> CoreResult<()> {
        let now = self.now();
        let ms = self.ms()?;
        let conv = ms
            .convs
            .get(&rec.conversation)
            .cloned()
            .ok_or(CoreError::NotFound)?;
        let reply = self.reply_rec(&rec.uuid);
        let contents: Vec<Content> = match rec.kind {
            MessageKind::Text => {
                let text = String::from_utf8_lossy(&rec.body).to_string();
                vec![match reply {
                    Some(r) => Content::TextReply {
                        reply_to: r.to,
                        quote: r.quote,
                        text,
                    },
                    None => Content::Text(text),
                }]
            }
            MessageKind::Image => chunk_image(&rec.body),
            MessageKind::File => chunk_file(rec.file_name.as_deref().unwrap_or("file"), &rec.body),
            MessageKind::Voice => chunk_voice(
                &self.voice_rec(&rec.uuid).map(|v| v.waveform).unwrap_or_default(),
                reply.map(|r| (r.to, r.quote)),
                &rec.body,
            ),
            MessageKind::System | MessageKind::Call => vec![],
        };
        let count = contents.len() as u16;
        let large = rec.body.len() > LARGE_CONTENT_BYTES;
        let mut envs = Vec::new();
        // Chunked media (D42): one Olm/Megolm message carries a content key
        // and hash; chunks are sealed with that key so any arrival order works.
        let media_key: Option<[u8; 32]> = (count > 1).then(random_array);
        if let Some(key) = media_key {
            let c = Content::MediaKey {
                key,
                hash: mesh_crypto::sha256(&[&rec.body]),
                count,
            };
            let mut ip = InnerPlaintext::new(
                c.kind(),
                rec.uuid,
                rec.sent_at,
                c.encode().map_err(|_| CoreError::Internal(22))?,
            );
            ip.expiry_secs = rec.disappear_secs;
            match conv.kind {
                ConversationKind::Direct => {
                    let peer = conv.peer.ok_or(CoreError::NotFound)?;
                    envs.push(self.send_direct(&peer, &ip, None, false)?);
                }
                ConversationKind::Group => envs.push(self.send_group_ip(&rec.conversation, &ip)?),
                ConversationKind::Channel => {
                    return Err(CoreError::InvalidInput("channels are text only".into()))
                }
            }
        }
        for (i, c) in contents.into_iter().enumerate() {
            let mut ip = InnerPlaintext::new(
                c.kind(),
                rec.uuid,
                rec.sent_at,
                c.encode()
                    .map_err(|_| CoreError::InvalidInput("content".into()))?,
            );
            ip.expiry_secs = rec.disappear_secs;
            ip.chunk_index = i as u16;
            ip.chunk_count = count;
            if let Some(key) = media_key {
                let padded = ip
                    .encode_auto()
                    .map_err(|_| CoreError::InvalidInput("chunk".into()))?;
                let sealed = seal_chunk(&key, &rec.uuid, i as u16, &padded);
                let body = sealed_chunk_body(&rec.uuid, i as u16, &sealed);
                match conv.kind {
                    ConversationKind::Direct => {
                        let peer = conv.peer.ok_or(CoreError::NotFound)?;
                        let id = self.queue_direct_inner(
                            &peer,
                            &direct_inner(SEALED_CHUNK, &body),
                            large.then_some(1),
                            false,
                        )?;
                        if large {
                            self.set_direct_only(id, peer);
                        }
                        envs.push(id);
                    }
                    _ => envs.push(self.send_group_inner(&rec.conversation, &group_inner(&[0; 32], &body))?),
                }
                continue;
            }
            match conv.kind {
                ConversationKind::Direct => {
                    let peer = conv.peer.ok_or(CoreError::NotFound)?;
                    let id = self.send_direct(&peer, &ip, large.then_some(1), false)?;
                    if large {
                        self.set_direct_only(id, peer);
                    }
                    envs.push(id);
                }
                ConversationKind::Group => {
                    envs.push(self.send_group_ip(&rec.conversation, &ip)?);
                }
                ConversationKind::Channel => {
                    let (reply_tag, id) = self.send_post(&rec.conversation, &ip)?;
                    rec.reply_tag = Some(reply_tag);
                    envs.push(id);
                }
            }
        }
        rec.state = DeliveryState::Queued;
        rec.pending_envs = envs.clone();
        if rec.disappear_secs > 0 {
            rec.expires_at = Some(now + rec.disappear_secs as u64 * 1000);
        }
        let ms = self.ms()?;
        let _ = ms.kv.apply(
            envs.iter()
                .map(|e| Op::Put(t::OUTENV, e.to_vec(), rec.uuid.to_vec()))
                .collect(),
        );
        // Channel and group posts have no per-recipient acks.
        if conv.kind != ConversationKind::Direct && self.links.values().any(|l| l.is_up()) {
            rec.state = DeliveryState::Sent;
        }
        self.store_msg(rec);
        self.emit(Event::MessageState {
            message: rec.uuid,
            state: rec.state,
        });
        Ok(())
    }

    pub(crate) fn set_direct_only(&mut self, env: MessageId, peer: Id16) {
        self.direct_only_map().insert(env, peer);
    }

    // ===== Delivery states =====

    fn msg_for_env(&self, env: &MessageId) -> Option<Id16> {
        let ms = self.msg.as_ref()?;
        ms.kv.get(t::OUTENV, env).ok()??.try_into().ok()
    }

    pub(crate) fn on_envelope_sent(&mut self, env: &MessageId, via: Option<crate::types::Via>) {
        let Some(uuid) = self.msg_for_env(env) else {
            return;
        };
        if let Some(v) = via {
            self.set_msg_via(&uuid, v);
        }
        // A file's parts each report here; only the first needs the record.
        if self.sent_marked.contains(&uuid) {
            return;
        }
        if let Some((_, mut rec)) = self.load_msg(&uuid) {
            if rec.state == DeliveryState::Queued {
                rec.state = DeliveryState::Sent;
                self.store_msg(&rec);
                self.emit(Event::MessageState {
                    message: uuid,
                    state: rec.state,
                });
            }
            if rec.state != DeliveryState::Held {
                if self.sent_marked.len() > 4_096 {
                    self.sent_marked.clear();
                }
                self.sent_marked.insert(uuid);
            }
        }
    }

    pub(crate) fn on_delivered(&mut self, env: &MessageId) {
        let now = self.now();
        self.last_delivery_ms = Some(now);
        if self.msg.is_none() {
            if let Some(r) = self.relay.as_mut() {
                let _ = r.kv.put(t::DELIVERED, env, &[]);
            }
            return;
        }
        let Some(uuid) = self.msg_for_env(env) else {
            return;
        };
        if let Some(ms) = self.msg.as_mut() {
            let _ = ms.kv.delete(t::OUTENV, env);
        }
        self.net_forget_envelope(env);
        // Count down without touching the (possibly large) record; it is
        // rewritten once, when the last envelope is acked.
        if let Some(n) = self.pending_left.get_mut(&uuid) {
            *n = n.saturating_sub(1);
            if *n > 0 {
                return;
            }
            self.pending_left.remove(&uuid);
        }
        if let Some((_, mut rec)) = self.load_msg(&uuid) {
            // Still awaited: those whose envelope→message row is not yet gone.
            let ms = self.msg.as_ref();
            rec.pending_envs.retain(|e| {
                e != env && ms.is_some_and(|m| m.kv.get(t::OUTENV, e).ok().flatten().is_some())
            });
            if !rec.pending_envs.is_empty() {
                if self.pending_left.len() > 4_096 {
                    self.pending_left.clear();
                }
                self.pending_left.insert(uuid, rec.pending_envs.len());
                return;
            }
            if rec.pending_envs.is_empty()
                && matches!(
                    rec.state,
                    DeliveryState::Queued | DeliveryState::Sent | DeliveryState::Failed
                )
            {
                rec.state = DeliveryState::Delivered;
                self.emit(Event::MessageState {
                    message: uuid,
                    state: rec.state,
                });
            }
            self.store_msg(&rec);
        }
    }

    pub(crate) fn process_delivered_while_locked(&mut self) {
        let ids: Vec<Vec<u8>> = self
            .relay
            .as_ref()
            .and_then(|r| r.kv.scan(t::DELIVERED).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        for k in ids {
            if let Ok(id) = k.as_slice().try_into() {
                self.on_delivered(&id);
            }
            if let Some(r) = self.relay.as_mut() {
                let _ = r.kv.delete(t::DELIVERED, &k);
            }
        }
    }

    pub fn mark_read(&mut self, conv: Id16) -> CoreResult<()> {
        let now = self.now();
        let receipts = self.settings().read_receipts;
        let ms = self.ms()?;
        let c = ms.convs.get_mut(&conv).ok_or(CoreError::NotFound)?;
        c.unread = 0;
        let kind = c.kind;
        let peer = c.peer;
        let rows = ms.kv.scan_prefix(t::MSG, &conv).unwrap_or_default();
        let mut to_ack = Vec::new();
        let mut ops = Vec::new();
        for (k, v) in rows {
            if let Some(mut r) = records::dec::<MsgRec>(&v) {
                if !r.outgoing && !r.read {
                    r.read = true;
                    to_ack.push(r.uuid);
                    ops.push(records::put(t::MSG, &k, &r));
                }
            }
        }
        let _ = ms.kv.apply(ops);
        self.save_conv(&conv);
        if receipts && !to_ack.is_empty() {
            for chunk in to_ack.chunks(MAX_READ_RECEIPT_IDS) {
                let c = Content::ReadReceipt(chunk.to_vec());
                match (kind, peer) {
                    (ConversationKind::Direct, Some(p)) => {
                        let _ = self.send_control(&p, c, now);
                    }
                    (ConversationKind::Group, _) => {
                        let ip = InnerPlaintext::new(
                            c.kind(),
                            random_array(),
                            now,
                            c.encode().unwrap_or_default(),
                        );
                        let _ = self.send_group_ip(&conv, &ip);
                    }
                    _ => {}
                }
            }
        }
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    pub fn delete_message(&mut self, m: Id16, for_everyone: bool) -> CoreResult<()> {
        let now = self.now();
        let (_, rec) = self.load_msg(&m).ok_or(CoreError::NotFound)?;
        if for_everyone && rec.outgoing {
            let conv = self.ms()?.convs.get(&rec.conversation).cloned();
            match conv.map(|c| (c.kind, c.peer)) {
                Some((ConversationKind::Direct, Some(p))) => {
                    self.send_control(&p, Content::Delete(m), now)?;
                }
                Some((ConversationKind::Group, _)) => {
                    let c = Content::Delete(m);
                    let ip = InnerPlaintext::new(
                        c.kind(),
                        random_array(),
                        now,
                        c.encode().unwrap_or_default(),
                    );
                    self.send_group_ip(&rec.conversation, &ip)?;
                }
                _ => {}
            }
        }
        // Stop relaying our own envelopes for it.
        for e in &rec.pending_envs {
            self.remove_stored(e, false);
        }
        self.delete_msg_rec(&rec);
        self.emit(Event::MessageDeleted { message: m });
        Ok(())
    }

    pub fn image_pixels(&self, m: Id16) -> CoreResult<RgbaImage> {
        let (_, rec) = self.load_msg(&m).ok_or(CoreError::NotFound)?;
        if rec.kind != MessageKind::Image || rec.chunks_have != rec.chunks_total {
            return Err(CoreError::NotFound);
        }
        let img = mesh_media::decode_jpeg(&rec.body).map_err(|_| CoreError::Crypto)?;
        Ok(RgbaImage {
            width: img.width,
            height: img.height,
            rgba: img.rgba,
        })
    }

    pub fn file_bytes(&self, m: Id16) -> CoreResult<(String, Vec<u8>)> {
        let (_, rec) = self.load_msg(&m).ok_or(CoreError::NotFound)?;
        if rec.kind != MessageKind::File || rec.chunks_have != rec.chunks_total {
            return Err(CoreError::NotFound);
        }
        Ok((rec.file_name.unwrap_or_else(|| "file".into()), rec.body))
    }

    // ===== Receiving =====

    pub(crate) fn receive_direct(&mut self, peer: &Id16, inner: &[u8], _env: &Envelope) {
        let Ok((typ, olm)) = parse_direct_inner(inner) else {
            return;
        };
        if typ == SEALED_CHUNK {
            if let Ok((uuid, idx, sealed)) = parse_sealed_chunk(olm) {
                self.on_sealed_chunk(Some(*peer), None, uuid, idx, sealed.to_vec());
            }
            return;
        }
        let Some(pt) = self.olm_decrypt(peer, typ, olm) else {
            return;
        };
        let Ok(ip) = InnerPlaintext::decode(&pt) else {
            self.crypto_failures += 1;
            return;
        };
        self.handle_inner(peer, None, ip);
    }

    /// Apply a decrypted inner plaintext from `peer` (in a group if `group`).
    pub(crate) fn handle_inner(&mut self, peer: &Id16, group: Option<Id16>, ip: InnerPlaintext) {
        let now = self.now();
        // Messages past their expiry are rejected even if they decrypt.
        if ip.sent_ms > 0 && now > ip.sent_ms + EXPIRY_MAX_SECS * 1000 + 3_600_000 {
            return;
        }
        let Ok(Some(content)) = Content::decode(ip.kind, &ip.content, ip.chunk_index == 0) else {
            return;
        };
        if self.ingest_via == Some(crate::types::Via::Internet) {
            let what = match &content {
                Content::Text(_) => "text message",
                Content::TextReply { .. } => "reply",
                Content::ImageChunk(_) => "photo piece",
                Content::FileChunk { .. } => "file piece",
                Content::VoiceChunk { .. } => "voice piece",
                Content::ReadReceipt(_) => "read receipt",
                _ => "control message",
            };
            self.log(format!(
                "opened {what} from the internet route: seal decrypted, sender is a paired contact"
            ));
        }
        // A reply is stored as text, with what it answers kept beside it.
        let content = match content {
            Content::TextReply {
                reply_to,
                quote,
                text,
            } => {
                if !self.msg_exists(&ip.message_uuid) {
                    self.put_reply(&ip.message_uuid, reply_to, quote);
                }
                Content::Text(text)
            }
            c => c,
        };
        match content {
            // Turned into text above.
            Content::TextReply { .. } => {}
            Content::Text(_) | Content::ImageChunk(_) | Content::FileChunk { .. } | Content::VoiceChunk { .. } => {
                let conv = match group {
                    Some(g) => g,
                    None => match self.ensure_direct_conv(peer) {
                        Ok(c) => c,
                        Err(_) => return,
                    },
                };
                self.store_incoming(conv, Some(*peer), None, &ip, content, None);
            }
            Content::ReadReceipt(ids) => {
                for id in ids {
                    if let Some((_, mut r)) = self.load_msg(&id) {
                        if r.outgoing && r.state != DeliveryState::Read {
                            r.state = DeliveryState::Read;
                            self.store_msg(&r);
                            self.emit(Event::MessageState {
                                message: id,
                                state: r.state,
                            });
                        }
                    }
                }
            }
            Content::SessionInit => {}
            Content::OneTimeKeys(keys) => {
                if let Some(p) = self.msg.as_mut().and_then(|m| m.peers.get_mut(peer)) {
                    p.remote_otks.extend(keys);
                    let excess = p.remote_otks.len().saturating_sub(MAX_OTK_IN_MESSAGE);
                    p.remote_otks.drain(..excess);
                }
                self.save_peer(peer);
            }
            Content::PresenceKey(k) => {
                let listed = self
                    .msg
                    .as_mut()
                    .and_then(|m| m.peers.get_mut(peer))
                    .map(|p| {
                        p.presence_key = k;
                        p.listed
                    });
                self.save_peer(peer);
                if listed == Some(true) {
                    if let Some(r) = self.relay.as_mut() {
                        r.presence.insert(*peer, k);
                        let _ = r.kv.apply(vec![records::put(t::PRESENCE, peer, &k)]);
                    }
                    self.rerecognise_links();
                }
            }
            Content::GroupKeyShare {
                group_id,
                session_key,
                tag_key,
                name,
            } => {
                self.on_group_key_share(peer, group_id, &session_key, tag_key, &name);
            }
            // Group members cannot set these for each other: direct only.
            Content::Reach(addr) if group.is_none() => self.on_reach(peer, addr),
            Content::Bridges(links) if group.is_none() => self.on_bridges(peer, links),
            Content::Reach(_) | Content::Bridges(_) => {}
            // Calls are one to one: never from a group.
            Content::CallSignal {
                kind,
                call_id,
                eph,
                reason,
            } if group.is_none() => self.on_call_signal(peer, ip.sent_ms, kind, call_id, eph, reason),
            Content::CallSignal { .. } => {}
            Content::GroupMembership {
                group_id,
                action,
                members,
            } => {
                self.on_group_membership(peer, group_id, action, members);
            }
            Content::MediaKey { key, hash, count } => {
                let rec = MediaKeyRec {
                    key,
                    hash,
                    sender: *peer,
                    group,
                    count,
                    created_ms: now,
                };
                if let Some(ms) = self.msg.as_mut() {
                    let _ = ms
                        .kv
                        .apply(vec![records::put(t::MEDIA_KEY, &ip.message_uuid, &rec)]);
                }
                self.process_pending_chunks(&ip.message_uuid);
            }
            Content::Delete(uuid) => {
                if let Some((_, r)) = self.load_msg(&uuid) {
                    if !r.outgoing && r.sender_peer == Some(*peer) {
                        self.delete_msg_rec(&r);
                        self.emit(Event::MessageDeleted { message: uuid });
                    }
                }
            }
        }
    }

    /// Store a received text / image chunk / file chunk.
    pub(crate) fn store_incoming(
        &mut self,
        conv: Id16,
        peer: Option<Id16>,
        pseudonym: Option<[u8; 32]>,
        ip: &InnerPlaintext,
        content: Content,
        reply_tag: Option<[u8; 16]>,
    ) {
        let now = self.now();
        let disappear = self
            .msg
            .as_ref()
            .and_then(|m| m.convs.get(&conv))
            .map(|c| c.disappear_secs)
            .unwrap_or(0);
        let disappear = if ip.expiry_secs > 0 {
            ip.expiry_secs
        } else {
            disappear
        };
        let existing = self.load_msg(&ip.message_uuid).map(|x| x.1);
        if let Some(e) = &existing {
            if e.outgoing || e.sender_peer != peer {
                return; // UUID collision or spoof attempt
            }
        }
        let (kind, data, name, total) = match content {
            Content::Text(t) => (MessageKind::Text, t.into_bytes(), None, None),
            Content::ImageChunk(d) => (MessageKind::Image, d, None, None),
            Content::FileChunk {
                name,
                total_size,
                data,
            } => (MessageKind::File, data, name, total_size.map(|s| s as u64)),
            Content::VoiceChunk {
                total_size,
                waveform,
                reply,
                data,
            } => {
                if let Some((to, quote)) = reply {
                    if self.reply_rec(&ip.message_uuid).is_none() {
                        self.put_reply(&ip.message_uuid, to, quote);
                    }
                }
                if let Some(w) = waveform {
                    if self.voice_rec(&ip.message_uuid).is_none() {
                        let w: Vec<u8> = w.into_iter().take(MAX_WAVEFORM_BARS).collect();
                        self.put_voice(
                            &ip.message_uuid,
                            &VoiceRec {
                                duration_ms: 0,
                                waveform: w,
                                listened: false,
                            },
                        );
                    }
                }
                (MessageKind::Voice, data, None, total_size.map(|s| s as u64))
            }
            _ => return,
        };
        let mut rec = existing.unwrap_or(MsgRec {
            uuid: ip.message_uuid,
            conversation: conv,
            outgoing: false,
            sender_peer: peer,
            sender_pseudonym: pseudonym,
            kind,
            body: vec![],
            file_name: None,
            total_size: None,
            chunks_have: 0,
            chunks_total: ip.chunk_count as u32,
            state: DeliveryState::Received,
            release_at: None,
            sent_at: ip.sent_ms,
            received_at: Some(now),
            expires_at: None,
            disappear_secs: disappear,
            reply_tag,
            read: false,
            pending_envs: vec![],
            lora_sent: false,
        });
        if rec.chunks_have == 0 {
            if let Some(v) = self.ingest_via {
                self.set_msg_via(&rec.uuid, v);
            }
        }
        if rec.kind != kind
            || rec.chunks_total != ip.chunk_count as u32
            || rec.chunks_have >= rec.chunks_total
        {
            return;
        }
        if name.is_some() {
            rec.file_name = name.map(|n| mesh_media::sanitize_filename(&n));
            rec.total_size = total;
        }
        if kind == MessageKind::Voice && total.is_some() {
            rec.total_size = total;
        }
        let first = rec.chunks_have == 0;
        if rec.chunks_total == 1 {
            rec.body = data;
            rec.chunks_have = 1;
        } else {
            let Some(ms) = self.msg.as_mut() else { return };
            let ck = [ip.message_uuid.as_slice(), &ip.chunk_index.to_be_bytes()].concat();
            if ms.kv.get(t::CHUNK, &ck).ok().flatten().is_some() {
                return;
            }
            let _ = ms.kv.put(t::CHUNK, &ck, &data);
            rec.chunks_have += 1;
            if rec.chunks_have == rec.chunks_total {
                let parts = ms
                    .kv
                    .scan_prefix(t::CHUNK, &ip.message_uuid)
                    .unwrap_or_default();
                let body: Vec<u8> = parts.iter().flat_map(|(_, v)| v.clone()).collect();
                let _ = ms.kv.apply(
                    parts
                        .into_iter()
                        .map(|(k, _)| Op::Delete(t::CHUNK, k))
                        .collect(),
                );
                // D42: the reassembled content must match the hash sent with its key.
                let media = records::load::<MediaKeyRec>(&*ms.kv, t::MEDIA_KEY, &ip.message_uuid);
                if let Some(mk) = &media {
                    let _ = ms.kv.delete(t::MEDIA_KEY, &ip.message_uuid);
                    if mesh_crypto::sha256(&[&body]) != mk.hash {
                        self.crypto_failures += 1;
                        self.delete_msg_rec(&rec);
                        return;
                    }
                }
                let ok = match kind {
                    MessageKind::Image => body.len() <= MAX_IMAGE_BYTES,
                    MessageKind::File => {
                        body.len() <= MAX_FILE_BYTES && rec.total_size == Some(body.len() as u64)
                    }
                    MessageKind::Voice => {
                        body.len() <= MAX_VOICE_BYTES && rec.total_size == Some(body.len() as u64)
                    }
                    _ => true,
                };
                if !ok {
                    self.delete_msg_rec(&rec);
                    return;
                }
                rec.body = body;
            }
        }
        // A complete voice note must be valid audio; its length comes from
        // the packets themselves, not from the sender's word.
        if kind == MessageKind::Voice && rec.chunks_have == rec.chunks_total {
            match mesh_media::voice::VoiceClip::from_blob(&rec.body) {
                Ok(clip) => {
                    let mut v = self.voice_rec(&rec.uuid).unwrap_or(VoiceRec {
                        duration_ms: 0,
                        waveform: vec![],
                        listened: false,
                    });
                    v.duration_ms = clip.duration_ms();
                    self.put_voice(&rec.uuid, &v);
                }
                Err(_) => {
                    self.crypto_failures += 1;
                    self.delete_msg_rec(&rec);
                    return;
                }
            }
        }
        self.store_msg(&rec);
        let view = self.view(&rec);
        self.emit(Event::MessageAdded {
            conversation: conv,
            message: view,
        });
        if first {
            let preview = match kind {
                MessageKind::Text => String::from_utf8_lossy(&rec.body).to_string(),
                MessageKind::Image => "Photo".into(),
                MessageKind::Voice => "Voice message".into(),
                _ => "File".into(),
            };
            self.touch_conv(&conv, &preview, true);
        }
    }

    // ===== Chunked media (D42) =====

    /// A chunk sealed with a media key. `from` is the direct sender; group
    /// chunks carry their group conversation instead.
    pub(crate) fn on_sealed_chunk(
        &mut self,
        from: Option<Id16>,
        group: Option<Id16>,
        uuid: Id16,
        idx: u16,
        sealed: Vec<u8>,
    ) {
        let now = self.now();
        let Some(ms) = self.msg.as_mut() else { return };
        let Some(mk) = records::load::<MediaKeyRec>(&*ms.kv, t::MEDIA_KEY, &uuid) else {
            // Key not here yet: park the chunk (bounded per message).
            let key = [uuid.as_slice(), &idx.to_be_bytes()].concat();
            if ms
                .kv
                .scan_prefix(t::PENDING_CHUNK, &uuid)
                .map(|v| v.len())
                .unwrap_or(0)
                < 1_400
            {
                let rec = PendingChunkRec {
                    from,
                    group,
                    sealed,
                    received_ms: now,
                };
                let _ = ms
                    .kv
                    .apply(vec![records::put(t::PENDING_CHUNK, &key, &rec)]);
            }
            return;
        };
        // The chunk must come from the same context as its key.
        if (group.is_none() && from != Some(mk.sender))
            || (group.is_some() && group != mk.group)
            || idx >= mk.count
        {
            self.crypto_failures += 1;
            return;
        }
        let Ok(pt) = open_chunk(&mk.key, &uuid, idx, &sealed) else {
            self.crypto_failures += 1;
            return;
        };
        let Ok(ip) = InnerPlaintext::decode(&pt) else {
            return;
        };
        if ip.message_uuid != uuid || ip.chunk_index != idx || ip.chunk_count != mk.count {
            self.crypto_failures += 1;
            return;
        }
        self.handle_inner(&mk.sender, mk.group, ip);
    }

    pub(crate) fn process_pending_chunks(&mut self, uuid: &Id16) {
        let Some(ms) = self.msg.as_mut() else { return };
        let pending = ms
            .kv
            .scan_prefix(t::PENDING_CHUNK, uuid)
            .unwrap_or_default();
        let _ = ms.kv.apply(
            pending
                .iter()
                .map(|(k, _)| Op::Delete(t::PENDING_CHUNK, k.clone()))
                .collect(),
        );
        for (k, v) in pending {
            let Some(rec) = records::dec::<PendingChunkRec>(&v) else {
                continue;
            };
            if k.len() != 18 {
                continue;
            }
            let idx = u16::from_be_bytes([k[16], k[17]]);
            self.on_sealed_chunk(rec.from, rec.group, *uuid, idx, rec.sealed);
        }
    }

    // ===== Timers =====

    pub(crate) fn messaging_slow_tick(&mut self, now: u64) {
        self.release_due_held();
        let Some(ms) = self.msg.as_ref() else { return };
        let pending: Vec<Id16> = ms
            .kv
            .scan(t::PENDING_OUT)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, _)| k.try_into().ok())
            .collect();
        let lora_ok = self.host.tx.capabilities().lora_paired && !self.settings().high_risk;
        for id in pending {
            let Some((_, mut rec)) = self.load_msg(&id) else {
                continue;
            };
            if !matches!(rec.state, DeliveryState::Queued | DeliveryState::Sent)
                || rec.pending_envs.is_empty()
            {
                continue;
            }
            // Group and channel posts get no acks: once one left this phone
            // it is done, and its envelopes expiring is no failure.
            let direct = ms_conv_kind(self, &rec.conversation) == Some(ConversationKind::Direct);
            if !direct && rec.state == DeliveryState::Sent {
                continue;
            }
            let alive = rec
                .pending_envs
                .iter()
                .any(|e| self.relay.as_ref().is_some_and(|r| r.store.contains(e)));
            if !alive {
                rec.state = DeliveryState::Failed;
                self.store_msg(&rec);
                self.emit(Event::MessageState {
                    message: id,
                    state: rec.state,
                });
                continue;
            }
            // LoRa fallback after 5 minutes without delivery (section 9).
            if lora_ok
                && !rec.lora_sent
                && rec.kind == MessageKind::Text
                && rec.body.len() <= MAX_LORA_TEXT_BYTES
                && now.saturating_sub(rec.sent_at) >= 5 * 60_000
            {
                let peer = self
                    .msg
                    .as_ref()
                    .and_then(|m| m.convs.get(&rec.conversation))
                    .and_then(|c| c.peer);
                if let Some(peer) = peer {
                    let mut ip = InnerPlaintext::new(
                        mesh_wire::kind::TEXT,
                        rec.uuid,
                        rec.sent_at,
                        rec.body.clone(),
                    );
                    ip.expiry_secs = rec.disappear_secs;
                    if self.send_direct(&peer, &ip, None, true).is_ok() {
                        rec.lora_sent = true;
                        self.set_msg_via(&rec.uuid, crate::types::Via::Lora);
                        self.store_msg(&rec);
                    }
                }
            }
        }
        // Disappearing messages (once a minute is enough).
        if now.saturating_sub(self.last_expire_ms) >= 60_000 {
            self.last_expire_ms = now;
            self.expire_messages(now);
        }
        // Fallback key rotation every 7 days.
        let rotate = self
            .msg
            .as_ref()
            .is_some_and(|m| now.saturating_sub(m.me.fallback_created_ms) >= FALLBACK_ROTATION_MS);
        if rotate {
            if let Some(ms) = self.msg.as_mut() {
                ms.me.rotate_fallback(now);
            }
            self.save_identity();
        }
    }

    pub(crate) fn expire_messages(&mut self, now: u64) {
        let Some(ms) = self.msg.as_ref() else { return };
        let dead: Vec<MsgRec> = ms
            .kv
            .scan(t::MSG)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, v)| records::dec::<MsgRec>(&v))
            .filter(|r| r.expires_at.is_some_and(|e| e <= now))
            .collect();
        for r in dead {
            self.delete_msg_rec(&r);
            self.emit(Event::MessageDeleted { message: r.uuid });
        }
        // Incomplete chunk sets are discarded after 24 hours.
        let Some(ms) = self.msg.as_ref() else { return };
        let stale: Vec<MsgRec> = ms
            .kv
            .scan(t::MSG)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, v)| records::dec::<MsgRec>(&v))
            .filter(|r| {
                !r.outgoing
                    && r.chunks_have < r.chunks_total
                    && r.received_at
                        .is_some_and(|t| now.saturating_sub(t) > CHUNK_REASSEMBLY_MS)
            })
            .collect();
        for r in stale {
            self.delete_msg_rec(&r);
        }
    }
}

/// Stable display name for a channel pseudonym.
pub(crate) fn pseudonym_name(pk: &[u8; 32]) -> String {
    const A: [&str; 16] = [
        "Amber", "Blue", "Cedar", "Dawn", "Ember", "Fern", "Gale", "Hazel", "Iris", "Jade", "Kite",
        "Lark", "Moss", "North", "Opal", "Pine",
    ];
    const B: [&str; 16] = [
        "Fox", "Heron", "Otter", "Wren", "Lynx", "Owl", "Hare", "Crane", "Finch", "Seal", "Moth",
        "Robin", "Swift", "Vole", "Yak", "Newt",
    ];
    let h = mesh_crypto::sha256(&[pk]);
    format!(
        "{} {} {:02x}",
        A[(h[0] & 15) as usize],
        B[(h[1] & 15) as usize],
        h[2]
    )
}

fn ms_conv_kind(e: &Engine, conv: &Id16) -> Option<ConversationKind> {
    e.msg.as_ref()?.convs.get(conv).map(|c| c.kind)
}
