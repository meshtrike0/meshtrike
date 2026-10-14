//! Groups (sections 6, 15.6, amendment A.9) and public channels (6, 7, 15.2).

use super::*;
use mesh_crypto::vodozemac::megolm::{MegolmMessage, SessionConfig, SessionKey};
use mesh_crypto::{group_env_key, group_tag};
use mesh_wire::*;

fn sid_bytes(s: &str) -> [u8; 32] {
    mesh_crypto::vodozemac::base64_decode(s)
        .ok()
        .and_then(|v| v.try_into().ok())
        .unwrap_or([0; 32])
}

impl Engine {
    fn save_group(&mut self, conv: &Id16) {
        let Some(ms) = self.msg.as_mut() else { return };
        let Some(g) = ms.groups.get_mut(conv) else {
            return;
        };
        g.rec.outbound_pickle = g.outbound.pickle().encrypt(&ms.pickle_key);
        let _ = ms.kv.apply(vec![records::put(t::GROUP, conv, &g.rec)]);
    }

    fn install_group_tag(&mut self, conv: Id16, tag_key: [u8; 32]) {
        let now = self.now();
        self.add_tagkey(
            conv,
            TagKeyRec {
                kind: TagKind::Group,
                owner: conv,
                key: tag_key,
                inbound_dir: 0,
                outbound_dir: 0,
                tag: [0; 16],
                created_ms: now,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn new_group_entry(
        &mut self,
        conv: Id16,
        group_id: Id16,
        name: &str,
        tag_key: [u8; 32],
        admin: [u8; 32],
        mine: bool,
        members: Vec<Id16>,
    ) {
        let now = self.now();
        let disappear = self.settings().disappear_default_secs;
        let Some(ms) = self.msg.as_mut() else { return };
        let outbound = GroupSession::new(SessionConfig::version_1());
        let rec = GroupRec {
            group_id,
            name: name.to_string(),
            tag_key,
            outbound_pickle: String::new(),
            outbound_session_id: sid_bytes(&outbound.session_id()),
            outbound_count: 0,
            outbound_created_ms: now,
            admin,
            created_by_me: mine,
            members,
            shared_with: vec![],
            left: false,
        };
        ms.groups.insert(conv, GroupEntry { rec, outbound });
        let c = ConvRec {
            kind: ConversationKind::Group,
            peer: None,
            title: name.to_string(),
            disappear_secs: disappear,
            last_activity: now,
            unread: 0,
            last_preview: String::new(),
        };
        let _ = ms.kv.apply(vec![records::put(t::CONV, &conv, &c)]);
        ms.convs.insert(conv, c);
        self.save_group(&conv);
        self.install_group_tag(conv, tag_key);
    }

    fn member_keys_of(&self, ids: &[Id16], include_me: bool) -> Vec<MemberKeys> {
        let Some(ms) = self.msg.as_ref() else {
            return vec![];
        };
        let mut v: Vec<MemberKeys> = ids
            .iter()
            .filter_map(|id| ms.peers.get(id))
            .map(|p| MemberKeys {
                ed25519: p.ed25519,
                curve25519: p.curve25519,
                noise_static: p.noise_static,
                binding_sig: [0; 64],
                fallback_key: p.remote_fallback,
                fallback_sig: [0; 64],
            })
            .collect();
        // Binding and fallback signatures are only known for ourselves; peers'
        // are re-fetched from their stored signatures below.
        for (m, id) in v
            .iter_mut()
            .zip(ids.iter().filter(|id| ms.peers.contains_key(*id)))
        {
            if let Some(sigs) = self.peer_sigs(id) {
                m.binding_sig = sigs.0;
                m.fallback_sig = sigs.1;
            }
        }
        if include_me {
            v.push(ms.me.member_keys());
        }
        v
    }

    fn peer_sigs(&self, id: &Id16) -> Option<([u8; 64], [u8; 64])> {
        let ms = self.msg.as_ref()?;
        let (a, b) = records::load::<(Vec<u8>, Vec<u8>)>(&*ms.kv, "peer_sigs", id)?;
        Some((a.try_into().ok()?, b.try_into().ok()?))
    }

    pub(crate) fn save_peer_sigs(&mut self, id: &Id16, binding: [u8; 64], fallback: [u8; 64]) {
        if let Some(ms) = self.msg.as_mut() {
            let _ = ms.kv.apply(vec![records::put(
                "peer_sigs",
                id,
                &(binding.to_vec(), fallback.to_vec()),
            )]);
        }
    }

    fn send_membership(
        &mut self,
        to: &Id16,
        group_id: Id16,
        action: MembershipAction,
        members: Vec<MemberKeys>,
    ) {
        let now = self.now();
        if members.is_empty() {
            let _ = self.send_control(
                to,
                Content::GroupMembership {
                    group_id,
                    action,
                    members,
                },
                now,
            );
            return;
        }
        for chunk in members.chunks(MEMBERS_PER_MESSAGE) {
            let _ = self.send_control(
                to,
                Content::GroupMembership {
                    group_id,
                    action,
                    members: chunk.to_vec(),
                },
                now,
            );
        }
    }

    fn share_key_with(&mut self, conv: &Id16, members: &[Id16]) {
        let now = self.now();
        let Some(g) = self.msg.as_ref().and_then(|m| m.groups.get(conv)) else {
            return;
        };
        let share = Content::GroupKeyShare {
            group_id: g.rec.group_id,
            session_key: g.outbound.session_key().to_bytes(),
            tag_key: g.rec.tag_key,
            name: g.rec.name.clone(),
        };
        for m in members {
            if self.send_control(m, share.clone(), now).is_ok() {
                if let Some(g) = self.msg.as_mut().and_then(|ms| ms.groups.get_mut(conv)) {
                    if !g.rec.shared_with.contains(m) {
                        g.rec.shared_with.push(*m);
                    }
                }
            }
        }
        self.save_group(conv);
    }

    pub fn create_group(&mut self, name: String, members: Vec<Id16>) -> CoreResult<Id16> {
        if name.is_empty() || name.len() > MAX_GROUP_NAME_BYTES {
            return Err(CoreError::InvalidInput("group name".into()));
        }
        if members.is_empty() || members.len() > MAX_GROUP_SIZE - 1 {
            return Err(CoreError::InvalidInput("1–49 members".into()));
        }
        let ms = self.ms()?;
        for m in &members {
            let p = ms.peers.get(m).ok_or(CoreError::NotFound)?;
            if !p.listed {
                return Err(CoreError::InvalidInput("members must be contacts".into()));
            }
        }
        let my_ed = ms.me.ed25519();
        let group_id: Id16 = random_array();
        let conv = group_id;
        self.new_group_entry(
            conv,
            group_id,
            &name,
            random_array(),
            my_ed,
            true,
            members.clone(),
        );
        let keys = self.member_keys_of(&members, true);
        self.share_key_with(&conv, &members);
        for m in &members {
            self.send_membership(m, group_id, MembershipAction::Add, keys.clone());
        }
        self.emit(Event::ConversationsChanged);
        Ok(conv)
    }

    fn require_admin(&mut self, conv: &Id16) -> CoreResult<GroupRec> {
        let ms = self.ms()?;
        let me = ms.me.ed25519();
        let g = ms.groups.get(conv).ok_or(CoreError::NotFound)?;
        if g.rec.admin != me {
            return Err(CoreError::InvalidInput(
                "only the admin can change members".into(),
            ));
        }
        Ok(g.rec.clone())
    }

    pub fn add_members(&mut self, conv: Id16, new: Vec<Id16>) -> CoreResult<()> {
        let rec = self.require_admin(&conv)?;
        let new: Vec<Id16> = new
            .into_iter()
            .filter(|m| !rec.members.contains(m))
            .collect();
        if rec.members.len() + new.len() > MAX_GROUP_SIZE - 1 {
            return Err(CoreError::InvalidInput("max 50 members".into()));
        }
        let ms = self.ms()?;
        for m in &new {
            if !ms.peers.get(m).is_some_and(|p| p.listed) {
                return Err(CoreError::NotFound);
            }
        }
        let mut all = rec.members.clone();
        all.extend(new.iter().copied());
        if let Some(g) = self.ms()?.groups.get_mut(&conv) {
            g.rec.members = all.clone();
        }
        self.save_group(&conv);
        let everyone = self.member_keys_of(&all, true);
        let newcomers = self.member_keys_of(&new, false);
        self.share_key_with(&conv, &new);
        for m in &new {
            self.send_membership(m, rec.group_id, MembershipAction::Add, everyone.clone());
        }
        for m in &rec.members {
            self.send_membership(m, rec.group_id, MembershipAction::Add, newcomers.clone());
        }
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    pub fn remove_member(&mut self, conv: Id16, member: Id16) -> CoreResult<()> {
        let rec = self.require_admin(&conv)?;
        if !rec.members.contains(&member) {
            return Err(CoreError::NotFound);
        }
        let removed = self.member_keys_of(&[member], false);
        let remaining: Vec<Id16> = rec
            .members
            .iter()
            .copied()
            .filter(|m| *m != member)
            .collect();
        for m in rec.members.iter() {
            self.send_membership(m, rec.group_id, MembershipAction::Remove, removed.clone());
        }
        if let Some(g) = self.ms()?.groups.get_mut(&conv) {
            g.rec.members = remaining;
        }
        // New outbound session and new group tag key, shared only with remaining members.
        self.rotate_megolm(&conv, true);
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    pub fn leave_group(&mut self, conv: Id16) -> CoreResult<()> {
        let ms = self.ms()?;
        let me = ms.me.member_keys();
        let g = ms.groups.get(&conv).ok_or(CoreError::NotFound)?.rec.clone();
        for m in &g.members {
            self.send_membership(m, g.group_id, MembershipAction::Leave, vec![me.clone()]);
        }
        if let Some(g) = self.ms()?.groups.get_mut(&conv) {
            g.rec.left = true;
        }
        self.save_group(&conv);
        self.remove_tagkey(&conv);
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    pub fn group_members(&self, conv: Id16) -> CoreResult<Vec<GroupMember>> {
        let ms = self.msg.as_ref().ok_or(CoreError::Locked)?;
        let g = ms.groups.get(&conv).ok_or(CoreError::NotFound)?;
        let mut v: Vec<GroupMember> = g
            .rec
            .members
            .iter()
            .filter_map(|id| ms.peers.get(id).map(|p| (id, p)))
            .map(|(id, p)| GroupMember {
                nickname: mesh_media::sanitize_text(&p.nickname),
                identity_key: hex::encode(p.ed25519),
                trust: p.trust,
                is_admin: p.ed25519 == g.rec.admin,
                is_me: false,
                contact: p.listed.then_some(*id),
                member: Some(*id),
            })
            .collect();
        v.push(GroupMember {
            nickname: ms.me.nickname.clone(),
            identity_key: hex::encode(ms.me.ed25519()),
            trust: Trust::Verified,
            is_admin: ms.me.ed25519() == g.rec.admin,
            is_me: true,
            contact: None,
            member: None,
        });
        Ok(v)
    }

    /// Replace our outbound Megolm session (100 messages, 7 days, removal).
    pub(crate) fn rotate_megolm(&mut self, conv: &Id16, new_tag: bool) {
        let now = self.now();
        let Some(ms) = self.msg.as_mut() else { return };
        let me = ms.me.ed25519();
        let Some(g) = ms.groups.get_mut(conv) else {
            return;
        };
        g.outbound = GroupSession::new(SessionConfig::version_1());
        g.rec.outbound_session_id = sid_bytes(&g.outbound.session_id());
        g.rec.outbound_count = 0;
        g.rec.outbound_created_ms = now;
        g.rec.shared_with.clear();
        let members = g.rec.members.clone();
        let mut tag = None;
        if new_tag && g.rec.admin == me {
            g.rec.tag_key = random_array();
            tag = Some(g.rec.tag_key);
        }
        self.save_group(conv);
        if let Some(k) = tag {
            self.install_group_tag(*conv, k);
        }
        self.share_key_with(conv, &members);
    }

    /// Queue a group envelope carrying an already-built inner message.
    /// Returns the envelope's message ID (for its delivery state).
    pub(crate) fn send_group_inner(&mut self, conv: &Id16, inner: &[u8]) -> CoreResult<MessageId> {
        let now = self.now();
        let ms = self.ms()?;
        let g = ms.groups.get(conv).ok_or(CoreError::NotFound)?;
        let tag_key = SymKey(g.rec.tag_key);
        let expiry = self.expiry_for(now, EXPIRY_DEFAULT_SECS);
        let mut env = Envelope {
            typ: EnvType::Group,
            ttl: self.rand_ttl(),
            flags: 0,
            message_id: random_array(),
            tag: group_tag(&tag_key, expiry),
            expiry,
            proof: [0; 8],
            payload: vec![],
        };
        seal_into(&mut env, &group_env_key(&tag_key), inner)
            .map_err(|_| CoreError::Internal(41))?;
        let id = env.message_id;
        let net_copy = env.clone();
        let members: Vec<Id16> = self
            .msg
            .as_ref()
            .and_then(|m| m.groups.get(conv))
            .map(|g| g.rec.members.clone())
            .unwrap_or_default();
        if !self.queue_own(env) {
            return Err(CoreError::StorageFull);
        }
        // Media chunks follow the same routes as the group's text.
        self.net_route(&net_copy, &members);
        Ok(id)
    }

    /// Returns the envelope's message ID (for its delivery state).
    pub(crate) fn send_group_ip(&mut self, conv: &Id16, ip: &InnerPlaintext) -> CoreResult<MessageId> {
        let now = self.now();
        let (needs_rotate, unshared) = {
            let ms = self.ms()?;
            let g = ms.groups.get(conv).ok_or(CoreError::NotFound)?;
            if g.rec.left {
                return Err(CoreError::InvalidInput("left group".into()));
            }
            let rot = g.rec.outbound_count >= MEGOLM_ROTATION_MESSAGES
                || now.saturating_sub(g.rec.outbound_created_ms) >= MEGOLM_ROTATION_MS;
            let un: Vec<Id16> = g
                .rec
                .members
                .iter()
                .copied()
                .filter(|m| !g.rec.shared_with.contains(m))
                .collect();
            (rot, un)
        };
        if needs_rotate {
            self.rotate_megolm(conv, false);
        } else if !unshared.is_empty() {
            self.share_key_with(conv, &unshared);
        }
        let padded = ip
            .encode_auto()
            .map_err(|_| CoreError::InvalidInput("too large".into()))?;
        let ms = self.ms()?;
        let g = ms.groups.get_mut(conv).ok_or(CoreError::NotFound)?;
        let m = g.outbound.encrypt(&padded);
        g.rec.outbound_count += 1;
        let sid = g.rec.outbound_session_id;
        let tag_key = SymKey(g.rec.tag_key);
        self.save_group(conv);
        let expiry = self.expiry_for(now, EXPIRY_DEFAULT_SECS);
        let mut env = Envelope {
            typ: EnvType::Group,
            ttl: self.rand_ttl(),
            flags: 0,
            message_id: random_array(),
            tag: group_tag(&tag_key, expiry),
            expiry,
            proof: [0; 8],
            payload: vec![],
        };
        seal_into(
            &mut env,
            &group_env_key(&tag_key),
            &group_inner(&sid, &m.to_bytes()),
        )
        .map_err(|_| CoreError::Internal(40))?;
        let id = env.message_id;
        let net_copy = env.clone();
        let members: Vec<Id16> = self
            .msg
            .as_ref()
            .and_then(|m| m.groups.get(conv))
            .map(|g| g.rec.members.clone())
            .unwrap_or_default();
        if !self.queue_own(env) {
            return Err(CoreError::StorageFull);
        }
        // Members we hold an address for get it over the internet too; they
        // pass it on over their own mesh.
        self.net_route(&net_copy, &members);
        Ok(id)
    }

    /// Returns false when the sender's key share has not arrived yet.
    pub(crate) fn receive_group(&mut self, conv: &Id16, inner: &[u8]) -> bool {
        // D42: a sealed chunk travels under an all-zero session ID.
        if inner.len() > 32 && inner[..32] == [0; 32] {
            if let Ok((uuid, idx, sealed)) = parse_sealed_chunk(&inner[32..]) {
                self.on_sealed_chunk(None, Some(*conv), uuid, idx, sealed.to_vec());
            }
            return true;
        }
        let Ok((sid, bytes)) = parse_group_inner(inner) else {
            return true;
        };
        let Ok(m) = MegolmMessage::from_bytes(bytes) else {
            self.crypto_failures += 1;
            return true;
        };
        let Some(ms) = self.msg.as_mut() else {
            return false;
        };
        // Key share not received yet: keep the envelope marked for us.
        let Some(entry) = ms.megolm_in.get_mut(&sid) else {
            return false;
        };
        if entry.rec.group_conv != *conv {
            self.crypto_failures += 1;
            return true;
        }
        let Ok(d) = entry.session.decrypt(&m) else {
            self.crypto_failures += 1;
            return true;
        };
        let sender = entry.rec.sender;
        let seen_key = [sid.as_slice(), &d.message_index.to_be_bytes()].concat();
        if ms
            .kv
            .get(t::MEGOLM_SEEN, &seen_key)
            .ok()
            .flatten()
            .is_some()
        {
            return true; // replay
        }
        let _ = ms.kv.put(t::MEGOLM_SEEN, &seen_key, &[]);
        let Ok(ip) = InnerPlaintext::decode(&d.plaintext) else {
            return true;
        };
        if let Some(sender) = sender {
            self.handle_inner(&sender, Some(*conv), ip);
        }
        true
    }

    pub(crate) fn on_group_key_share(
        &mut self,
        peer: &Id16,
        group_id: Id16,
        session_key: &[u8],
        tag_key: [u8; 32],
        name: &str,
    ) {
        let now = self.now();
        let Ok(sk) = SessionKey::from_bytes(session_key) else {
            return;
        };
        let conv = group_id;
        let Some(ms) = self.msg.as_ref() else { return };
        let Some(peer_ed) = ms.peers.get(peer).map(|p| p.ed25519) else {
            return;
        };
        match ms.groups.get(&conv) {
            None => {
                // First contact with this group: the sharer becomes the provisional admin.
                self.new_group_entry(conv, group_id, name, tag_key, peer_ed, false, vec![*peer]);
                self.emit(Event::ConversationsChanged);
            }
            Some(g) => {
                if !g.rec.members.contains(peer) {
                    return;
                }
                if g.rec.admin == peer_ed && (g.rec.tag_key != tag_key || g.rec.name != name) {
                    let new_tag = g.rec.tag_key != tag_key;
                    if let Some(ms) = self.msg.as_mut() {
                        if let Some(g) = ms.groups.get_mut(&conv) {
                            g.rec.tag_key = tag_key;
                            g.rec.name = name.to_string();
                        }
                        if let Some(c) = ms.convs.get_mut(&conv) {
                            c.title = name.to_string();
                        }
                    }
                    self.save_group(&conv);
                    self.save_conv(&conv);
                    if new_tag {
                        self.install_group_tag(conv, tag_key);
                    }
                }
            }
        }
        let session = InboundGroupSession::new(&sk, SessionConfig::version_1());
        let sid = sid_bytes(&session.session_id());
        let Some(ms) = self.msg.as_mut() else { return };
        let rec = InMegolmRec {
            group_conv: conv,
            sender: Some(*peer),
            pickle: session.pickle().encrypt(&ms.pickle_key),
            created_ms: now,
        };
        let _ = ms.kv.apply(vec![records::put(t::MEGOLM_IN, &sid, &rec)]);
        ms.megolm_in.insert(sid, InMegolm { rec, session });
        self.rescan_group_envelopes();
    }

    /// Group envelopes stored before we had the tag key or Megolm key.
    pub(crate) fn rescan_group_envelopes(&mut self) {
        let cands: Vec<(MessageId, [u8; 16], u32)> = self
            .relay
            .as_ref()
            .map(|r| {
                r.store
                    .iter()
                    .filter(|s| s.env.typ == EnvType::Group && !s.mine)
                    .map(|s| (s.env.message_id, s.env.tag, s.env.expiry))
                    .collect()
            })
            .unwrap_or_default();
        for (id, tag, exp) in cands {
            if self.lookup_tag(&tag, exp).is_some() {
                let offer = self
                    .relay
                    .as_ref()
                    .and_then(|r| r.store.get(&id))
                    .is_some_and(|s| s.offer);
                self.update_stored(&id, true, offer);
                self.handle_for_me(&id);
            }
        }
    }

    pub(crate) fn on_group_membership(
        &mut self,
        peer: &Id16,
        group_id: Id16,
        action: MembershipAction,
        members: Vec<MemberKeys>,
    ) {
        let now = self.now();
        let conv = group_id;
        let Some(ms) = self.msg.as_ref() else { return };
        let my_ed = ms.me.ed25519();
        let Some(peer_ed) = ms.peers.get(peer).map(|p| p.ed25519) else {
            return;
        };
        if !ms.groups.contains_key(&conv) {
            if action != MembershipAction::Add {
                return;
            }
            self.new_group_entry(
                conv,
                group_id,
                "Group",
                random_array(),
                peer_ed,
                false,
                vec![*peer],
            );
        }
        let admin = self
            .msg
            .as_ref()
            .and_then(|m| m.groups.get(&conv))
            .map(|g| g.rec.admin)
            .unwrap_or([0; 32]);
        match action {
            MembershipAction::Add => {
                if peer_ed != admin {
                    return;
                }
                for m in members {
                    if m.ed25519 == my_ed {
                        continue;
                    }
                    let Some(id) = self.peer_for_member(&m, now) else {
                        continue;
                    };
                    if let Some(g) = self.msg.as_mut().and_then(|ms| ms.groups.get_mut(&conv)) {
                        if !g.rec.members.contains(&id) && g.rec.members.len() < MAX_GROUP_SIZE - 1
                        {
                            g.rec.members.push(id);
                        }
                    }
                }
                self.save_group(&conv);
            }
            MembershipAction::Remove => {
                if peer_ed != admin {
                    return;
                }
                let mut removed_me = false;
                for m in &members {
                    if m.ed25519 == my_ed {
                        removed_me = true;
                    }
                    let id = self.msg.as_ref().and_then(|ms| {
                        ms.peers
                            .iter()
                            .find(|(_, p)| p.ed25519 == m.ed25519)
                            .map(|(k, _)| *k)
                    });
                    if let (Some(id), Some(g)) = (
                        id,
                        self.msg.as_mut().and_then(|ms| ms.groups.get_mut(&conv)),
                    ) {
                        g.rec.members.retain(|x| *x != id);
                    }
                }
                if removed_me {
                    if let Some(g) = self.msg.as_mut().and_then(|ms| ms.groups.get_mut(&conv)) {
                        g.rec.left = true;
                    }
                    self.save_group(&conv);
                    self.remove_tagkey(&conv);
                } else {
                    self.rotate_megolm(&conv, false);
                }
            }
            MembershipAction::Leave => {
                if members.len() != 1 || members[0].ed25519 != peer_ed {
                    return;
                }
                if let Some(g) = self.msg.as_mut().and_then(|ms| ms.groups.get_mut(&conv)) {
                    g.rec.members.retain(|x| x != peer);
                }
                self.rotate_megolm(&conv, true);
            }
        }
        self.emit(Event::ConversationsChanged);
    }

    /// Find or create the peer record for a group member (A.9).
    fn peer_for_member(&mut self, m: &MemberKeys, now: u64) -> Option<Id16> {
        let ms = self.msg.as_ref()?;
        if let Some((id, _)) = ms.peers.iter().find(|(_, p)| p.ed25519 == m.ed25519) {
            return Some(*id);
        }
        mesh_identity::verify_member(m).ok()?;
        let dh = x25519_dalek::StaticSecret::from(*ms.me.noise_static_sk)
            .diffie_hellman(&x25519_dalek::PublicKey::from(m.noise_static));
        let tag_key = mesh_crypto::hkdf32(dh.as_bytes(), &[b"member-tag-v1"]);
        let id: Id16 = random_array();
        let rec = PeerRec {
            ed25519: m.ed25519,
            curve25519: m.curve25519,
            noise_static: m.noise_static,
            nickname: format!("Member {}", &hex::encode(m.ed25519)[..6]),
            trust: Trust::Unverified,
            listed: false,
            presence_key: [0; 32],
            tag_key,
            remote_otks: vec![],
            remote_fallback: m.fallback_key,
            my_otks_held: 0,
            conversation: None,
            created_ms: now,
        };
        self.install_peer(id, rec);
        self.save_peer_sigs(&id, m.binding_sig, m.fallback_sig);
        Some(id)
    }

    pub(crate) fn groups_slow_tick(&mut self, _now: u64) {}

    // ===== Channels =====

    pub fn join_channel(&mut self, name: String) -> CoreResult<Id16> {
        let norm = mesh_crypto::normalize_channel_name(&name);
        if norm.is_empty() || norm.len() > 64 {
            return Err(CoreError::InvalidInput("channel name".into()));
        }
        let now = self.now();
        let key = mesh_crypto::channel_key(&norm);
        let hash = mesh_crypto::channel_hash(&key);
        let conv = mesh_crypto::hmac16(&key.0, &[b"chan-conv"]);
        let ms = self.ms()?;
        if ms.channels.contains_key(&conv) {
            return Ok(conv);
        }
        let rec = ChannelRec {
            name: norm.clone(),
            pseudonym_seed: random_array(),
        };
        let c = ConvRec {
            kind: ConversationKind::Channel,
            peer: None,
            title: norm,
            disappear_secs: 0,
            last_activity: now,
            unread: 0,
            last_preview: String::new(),
        };
        let _ = ms.kv.apply(vec![
            records::put(t::CHANNEL, &conv, &rec),
            records::put(t::CONV, &conv, &c),
        ]);
        ms.channels.insert(conv, rec);
        ms.convs.insert(conv, c);
        self.add_tagkey(
            conv,
            TagKeyRec {
                kind: TagKind::Channel,
                owner: conv,
                key: key.0,
                inbound_dir: 0,
                outbound_dir: 0,
                tag: hash,
                created_ms: now,
            },
        );
        // Posts already carried by our relay store become readable.
        let ids: Vec<MessageId> = self
            .relay
            .as_ref()
            .map(|r| {
                r.store
                    .iter()
                    .filter(|s| s.env.typ == EnvType::Broadcast && s.env.tag == hash && !s.mine)
                    .map(|s| s.env.message_id)
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            self.handle_for_me(&id);
        }
        self.emit(Event::ConversationsChanged);
        Ok(conv)
    }

    pub fn leave_channel(&mut self, conv: Id16) -> CoreResult<()> {
        let ms = self.ms()?;
        ms.channels.remove(&conv).ok_or(CoreError::NotFound)?;
        ms.convs.remove(&conv);
        let mut ops = vec![
            Op::Delete(t::CHANNEL, conv.to_vec()),
            Op::Delete(t::CONV, conv.to_vec()),
        ];
        for (k, _) in ms.kv.scan_prefix(t::MSG, &conv).unwrap_or_default() {
            ops.push(Op::Delete(t::MSG, k));
        }
        let _ = ms.kv.apply(ops);
        self.remove_tagkey(&conv);
        self.emit(Event::ConversationsChanged);
        Ok(())
    }

    /// Sign, seal and proof-of-work a channel post; returns its reply tag.
    /// Returns the post's reply tag and envelope message ID.
    pub(crate) fn send_post(
        &mut self,
        conv: &Id16,
        ip: &InnerPlaintext,
    ) -> CoreResult<([u8; 16], MessageId)> {
        let now = self.now();
        let ms = self.ms()?;
        let ch = ms.channels.get(conv).cloned().ok_or(CoreError::NotFound)?;
        let key = mesh_crypto::channel_key(&ch.name);
        let hash = mesh_crypto::channel_hash(&key);
        let seed = mesh_crypto::Ed25519Seed(ch.pseudonym_seed);
        let pk = seed.public();
        let reply_tag: [u8; 16] = random_array();
        let message_id: MessageId = random_array();
        let inner = ip
            .encode_auto()
            .map_err(|_| CoreError::InvalidInput("too large".into()))?;
        let sig = seed.sign(&Post::signing_bytes(
            &hash,
            &message_id,
            &pk,
            &reply_tag,
            &inner,
        ));
        let post = Post {
            pseudonym_pk: pk,
            signature: sig,
            reply_tag,
            inner,
        };
        let expiry = self.expiry_for(now, EXPIRY_DEFAULT_SECS);
        let mut env = Envelope {
            typ: EnvType::Broadcast,
            ttl: self.rand_ttl(),
            flags: 0,
            message_id,
            tag: hash,
            expiry,
            proof: [0; 8],
            payload: vec![],
        };
        seal_into(&mut env, &key, &post.encode())
            .map_err(|_| CoreError::InvalidInput("too large".into()))?;
        let bits = match self.cfg.pow_bits_override {
            Some(b) => b,
            None => self.broadcast_rate.required_bits(now),
        };
        let nonce =
            mesh_crypto::pow::pow_solve(&env.pow_prefix(), &env.payload, bits, self.rng.next());
        env.proof = nonce.to_be_bytes();
        self.broadcast_rate.record(now);
        // Our reply tag receives remote-pairing handshakes (15.2).
        self.add_tagkey(
            ip.message_uuid,
            TagKeyRec {
                kind: TagKind::Reply,
                owner: ip.message_uuid,
                key: mesh_crypto::handshake_key(&reply_tag).0,
                inbound_dir: 0,
                outbound_dir: 0,
                tag: reply_tag,
                created_ms: now,
            },
        );
        let rec = PostRec {
            channel_conv: *conv,
            reply_tag,
            pseudonym_pk: pk,
            mine: true,
            created_ms: now,
        };
        let _ = self
            .ms()?
            .kv
            .apply(vec![records::put(t::POST, &ip.message_uuid, &rec)]);
        if !self.queue_own(env) {
            return Err(CoreError::StorageFull);
        }
        Ok((reply_tag, message_id))
    }

    pub(crate) fn receive_post(&mut self, conv: &Id16, inner: &[u8], env: &Envelope) {
        let now = self.now();
        let Ok(post) = Post::decode(inner) else {
            return;
        };
        let Some(ch) = self
            .msg
            .as_ref()
            .and_then(|m| m.channels.get(conv))
            .cloned()
        else {
            return;
        };
        let key = mesh_crypto::channel_key(&ch.name);
        let hash = mesh_crypto::channel_hash(&key);
        let msg = Post::signing_bytes(
            &hash,
            &env.message_id,
            &post.pseudonym_pk,
            &post.reply_tag,
            &post.inner,
        );
        if mesh_crypto::ed25519_verify(&post.pseudonym_pk, &msg, &post.signature).is_err() {
            self.crypto_failures += 1;
            return;
        }
        if post.pseudonym_pk == mesh_crypto::Ed25519Seed(ch.pseudonym_seed).public() {
            return; // our own post
        }
        let Ok(ip) = InnerPlaintext::decode(&post.inner) else {
            return;
        };
        let Ok(Some(content @ Content::Text(_))) = Content::decode(ip.kind, &ip.content, true)
        else {
            return;
        };
        if let Some(ms) = self.msg.as_mut() {
            let rec = PostRec {
                channel_conv: *conv,
                reply_tag: post.reply_tag,
                pseudonym_pk: post.pseudonym_pk,
                mine: false,
                created_ms: now,
            };
            let _ = ms
                .kv
                .apply(vec![records::put(t::POST, &ip.message_uuid, &rec)]);
        }
        self.store_incoming(
            *conv,
            None,
            Some(post.pseudonym_pk),
            &ip,
            content,
            Some(post.reply_tag),
        );
    }
}
