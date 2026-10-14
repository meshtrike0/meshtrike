//! Contact pairing (section 15.1 in person / Noise IK, 15.2 remote / Noise XX,
//! amendments A.8 and A.10).

use super::*;
use mesh_crypto::noise::{Handshake, Pattern};
use mesh_crypto::{ct_eq, handshake_key, hmac16};
use mesh_identity::{ContactCard, PairingQr};
use mesh_wire::*;

const KIND_XX1: u8 = 1;
const KIND_XX2: u8 = 2;
const KIND_XX3: u8 = 3;
const KIND_IK1: u8 = 4;
const KIND_IK2: u8 = 5;

pub(crate) struct IkShow {
    token: [u8; 16],
    tag: [u8; 16],
    created: u64,
}

pub(crate) struct IkScan {
    hs: Handshake,
    qr: PairingQr,
    seed: [u8; 32],
    created: u64,
}

pub(crate) struct XxOut {
    hs: Handshake,
    h1: Vec<u8>,
    reply_tag: [u8; 16],
    return_tag: [u8; 16],
    post_pk: [u8; 32],
    seed: [u8; 32],
    created: u64,
}

pub(crate) struct XxIn {
    hs: Handshake,
    h1: Vec<u8>,
    reply_tag: [u8; 16],
    return_tag: [u8; 16],
    post: Id16,
    seed: [u8; 32],
    accepted: bool,
    created: u64,
}

#[derive(Default)]
pub(crate) struct Pairings {
    ik_show: HashMap<Id16, IkShow>,
    ik_scan: HashMap<Id16, IkScan>,
    xx_out: HashMap<Id16, XxOut>,
    xx_in: HashMap<Id16, XxIn>,
}

impl Pairings {
    /// Pairing UUIDs we scan for while an in-person pairing is in progress.
    pub fn scanning_uuids(&self, now: u64) -> Vec<[u8; 16]> {
        self.ik_scan
            .values()
            .filter(|s| now.saturating_sub(s.created) < PAIRING_WINDOW_MS)
            .map(|s| mesh_crypto::pairing_uuid(&s.qr.token))
            .collect()
    }
}

fn pairing_tag(token: &[u8; 16]) -> [u8; 16] {
    hmac16(token, &[b"pair-tag-v1"])
}

impl Engine {
    fn handshake_env(
        &mut self,
        tag: &[u8; 16],
        kind: u8,
        noise: &[u8],
        ttl: Option<u8>,
    ) -> CoreResult<Envelope> {
        let now = self.now();
        let expiry = self.expiry_for(now, 3600);
        let mut env = Envelope {
            typ: EnvType::Handshake,
            ttl: ttl.unwrap_or_else(|| self.rand_ttl()),
            flags: 0,
            message_id: random_array(),
            tag: *tag,
            expiry,
            proof: [0; 8],
            payload: vec![],
        };
        seal_into(&mut env, &handshake_key(tag), &handshake_inner(kind, noise))
            .map_err(|_| CoreError::InvalidInput("handshake too large".into()))?;
        Ok(env)
    }

    fn add_handshake_tag(&mut self, owner: Id16, kind: TagKind, tag: [u8; 16]) {
        let now = self.now();
        self.add_tagkey(
            owner,
            TagKeyRec {
                kind,
                owner,
                key: handshake_key(&tag).0,
                inbound_dir: 0,
                outbound_dir: 0,
                tag,
                created_ms: now,
            },
        );
    }

    // ===== In person (15.1) =====

    /// "Show my code": 97-byte QR payload, valid 120 s.
    pub fn pairing_qr(&mut self) -> CoreResult<Vec<u8>> {
        let now = self.now();
        let eph = self.eph_id;
        let ms = self.ms()?;
        let token: [u8; 16] = random_array();
        let qr = PairingQr {
            ed25519: ms.me.ed25519(),
            noise_static: ms.me.noise_pk(),
            token,
            ephemeral_id: eph,
        };
        let tag = pairing_tag(&token);
        let id = hmac16(&token, &[b"pairing-id"]);
        ms.pairings
            .ik_show
            .retain(|_, s| now.saturating_sub(s.created) < PAIRING_WINDOW_MS);
        ms.pairings.ik_show.insert(
            id,
            IkShow {
                token,
                tag,
                created: now,
            },
        );
        self.add_handshake_tag(id, TagKind::PairingIk, tag);
        self.pairing_uuid = Some((mesh_crypto::pairing_uuid(&token), now + PAIRING_WINDOW_MS));
        self.refresh_radio();
        Ok(qr.encode())
    }

    /// "Scan code": start Noise IK to the scanned phone.
    pub fn pair_from_qr(&mut self, payload: Vec<u8>) -> CoreResult<Id16> {
        let now = self.now();
        let qr = PairingQr::decode(&payload)
            .map_err(|_| CoreError::InvalidInput("not a pairing code".into()))?;
        let ms = self.ms()?;
        if qr.ed25519 == ms.me.ed25519() {
            return Err(CoreError::InvalidInput("that is your own code".into()));
        }
        let seed: [u8; 32] = random_array();
        let card = ms.me.contact_card(seed);
        let mut hs = Handshake::initiator(
            Pattern::IK,
            Some(&ms.me.noise_static_sk),
            Some(&qr.noise_static),
        )
        .map_err(|_| CoreError::Crypto)?;
        let mut payload = qr.token.to_vec();
        payload.extend(card.encode());
        let m1 = hs.write(&payload).map_err(|_| CoreError::Crypto)?;
        let tag = pairing_tag(&qr.token);
        let id = hmac16(&qr.token, &[b"pairing-id"]);
        ms.pairings.ik_scan.insert(
            id,
            IkScan {
                hs,
                qr,
                seed,
                created: now,
            },
        );
        self.save_identity_pub();
        self.add_handshake_tag(id, TagKind::PairingIk, tag);
        let env = self.handshake_env(&tag, KIND_IK1, &m1, Some(1))?;
        self.queue_own(env);
        self.refresh_radio();
        Ok(id)
    }

    pub(crate) fn save_identity_pub(&mut self) {
        if let Some(ms) = self.msg.as_mut() {
            let b = ms.me.to_bytes(&ms.pickle_key);
            let _ = ms.kv.put(t::IDENTITY, b"me", &b);
        }
    }

    // ===== Remote (15.2) =====

    pub fn pair_from_post(&mut self, post: Id16) -> CoreResult<Id16> {
        let now = self.now();
        let ms = self.ms()?;
        let p = records::load::<PostRec>(&*ms.kv, t::POST, &post).ok_or(CoreError::NotFound)?;
        if p.mine {
            return Err(CoreError::InvalidInput("that is your own post".into()));
        }
        let return_tag: [u8; 16] = random_array();
        let mut hs = Handshake::initiator(Pattern::XX, Some(&ms.me.noise_static_sk), None)
            .map_err(|_| CoreError::Crypto)?;
        let m1 = hs.write(&return_tag).map_err(|_| CoreError::Crypto)?;
        let h1 = hs.handshake_hash();
        let id: Id16 = random_array();
        ms.pairings.xx_out.insert(
            id,
            XxOut {
                hs,
                h1,
                reply_tag: p.reply_tag,
                return_tag,
                post_pk: p.pseudonym_pk,
                seed: random_array(),
                created: now,
            },
        );
        self.add_handshake_tag(id, TagKind::Return, return_tag);
        let env = self.handshake_env(&p.reply_tag, KIND_XX1, &m1, None)?;
        self.queue_own(env);
        Ok(id)
    }

    pub fn accept_pairing(&mut self, id: Id16) -> CoreResult<()> {
        let ms = self.ms()?;
        let mut req = ms.pairings.xx_in.remove(&id).ok_or(CoreError::NotFound)?;
        let post =
            records::load::<PostRec>(&*ms.kv, t::POST, &req.post).ok_or(CoreError::NotFound)?;
        let ch = ms
            .channels
            .get(&post.channel_conv)
            .cloned()
            .ok_or(CoreError::NotFound)?;
        let card = ms.me.contact_card(req.seed).encode();
        let pseudo = mesh_crypto::Ed25519Seed(ch.pseudonym_seed);
        let sig = pseudo.sign(&[b"xx-bind-v1".as_slice(), &req.h1].concat());
        let mut payload = (card.len() as u16).to_be_bytes().to_vec();
        payload.extend(card);
        payload.extend(pseudo.public());
        payload.extend(sig);
        let m2 = req.hs.write(&payload).map_err(|_| CoreError::Crypto)?;
        req.accepted = true;
        let return_tag = req.return_tag;
        ms.pairings.xx_in.insert(id, req);
        self.save_identity_pub();
        let env = self.handshake_env(&return_tag, KIND_XX2, &m2, None)?;
        self.queue_own(env);
        Ok(())
    }

    pub fn decline_pairing(&mut self, id: Id16) -> CoreResult<()> {
        self.ms()?
            .pairings
            .xx_in
            .remove(&id)
            .ok_or(CoreError::NotFound)?;
        Ok(())
    }

    // ===== Incoming handshake envelopes =====

    pub(crate) fn receive_handshake(&mut self, rec: &TagKeyRec, inner: &[u8]) {
        let Ok((kind, noise)) = parse_handshake_inner(inner) else {
            return;
        };
        let r = match (kind, rec.kind) {
            (KIND_IK1, TagKind::PairingIk) => self.on_ik1(rec, noise),
            (KIND_IK2, TagKind::PairingIk) => self.on_ik2(rec, noise),
            (KIND_XX1, TagKind::Reply) => self.on_xx1(rec, noise),
            (KIND_XX2, TagKind::Return) => self.on_xx2(rec, noise),
            (KIND_XX3, TagKind::Reply) => self.on_xx3(rec, noise),
            _ => Ok(()),
        };
        if let Err(e) = r {
            self.crypto_failures += 1;
            let step = match kind {
                KIND_IK1 | KIND_IK2 => "in-person pairing",
                _ => "channel chat request",
            };
            self.log(format!("{step}: handshake step {kind} rejected: {e}"));
        }
    }

    fn on_ik1(&mut self, rec: &TagKeyRec, noise: &[u8]) -> CoreResult<()> {
        let now = self.now();
        let ms = self.ms()?;
        let Some(show) = ms.pairings.ik_show.get(&rec.owner) else {
            return Ok(());
        };
        if now.saturating_sub(show.created) >= PAIRING_WINDOW_MS {
            return Ok(());
        }
        let token = show.token;
        let tag = show.tag;
        let mut hs = Handshake::responder(Pattern::IK, Some(&ms.me.noise_static_sk))
            .map_err(|_| CoreError::Crypto)?;
        let payload = hs.read(noise).map_err(|_| CoreError::Crypto)?;
        if payload.len() < 16 || !ct_eq(&payload[..16], &token) {
            return Err(CoreError::Crypto);
        }
        let card = ContactCard::decode_verified(&payload[16..]).map_err(|_| CoreError::Crypto)?;
        if Some(card.noise_static) != hs.remote_static() {
            return Err(CoreError::Crypto);
        }
        let seed: [u8; 32] = random_array();
        let mine = ms.me.contact_card(seed).encode();
        let m2 = hs.write(&mine).map_err(|_| CoreError::Crypto)?;
        ms.pairings.ik_show.remove(&rec.owner);
        self.save_identity_pub();
        let env = self.handshake_env(&tag, KIND_IK2, &m2, Some(1))?;
        self.queue_own(env);
        self.remove_tagkey(&rec.owner);
        self.pairing_uuid = None;
        self.refresh_radio();
        let contact = self.add_contact(&card, true, seed)?;
        let nickname = self.peer_name(&contact);
        self.emit(Event::PairingResult {
            pairing: rec.owner,
            outcome: PairingOutcome::Paired {
                contact,
                nickname,
                verified: true,
            },
        });
        Ok(())
    }

    fn on_ik2(&mut self, rec: &TagKeyRec, noise: &[u8]) -> CoreResult<()> {
        let ms = self.ms()?;
        let Some(mut scan) = ms.pairings.ik_scan.remove(&rec.owner) else {
            return Ok(());
        };
        let payload = scan.hs.read(noise).map_err(|_| CoreError::Crypto)?;
        let card = ContactCard::decode_verified(&payload).map_err(|_| CoreError::Crypto)?;
        if card.ed25519 != scan.qr.ed25519 || card.noise_static != scan.qr.noise_static {
            self.emit(Event::PairingResult {
                pairing: rec.owner,
                outcome: PairingOutcome::Failed("identity mismatch".into()),
            });
            return Err(CoreError::Crypto);
        }
        self.remove_tagkey(&rec.owner);
        let contact = self.add_contact(&card, true, scan.seed)?;
        let nickname = self.peer_name(&contact);
        self.emit(Event::PairingResult {
            pairing: rec.owner,
            outcome: PairingOutcome::Paired {
                contact,
                nickname,
                verified: true,
            },
        });
        Ok(())
    }

    fn on_xx1(&mut self, rec: &TagKeyRec, noise: &[u8]) -> CoreResult<()> {
        let now = self.now();
        let ms = self.ms()?;
        let post =
            records::load::<PostRec>(&*ms.kv, t::POST, &rec.owner).ok_or(CoreError::NotFound)?;
        if !post.mine {
            return Ok(());
        }
        let channel = ms
            .channels
            .get(&post.channel_conv)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let mut hs = Handshake::responder(Pattern::XX, Some(&ms.me.noise_static_sk))
            .map_err(|_| CoreError::Crypto)?;
        let payload = hs.read(noise).map_err(|_| CoreError::Crypto)?;
        let return_tag: [u8; 16] = payload
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::Crypto)?;
        let h1 = hs.handshake_hash();
        if ms.pairings.xx_in.len() >= 16 {
            return Ok(()); // bound pending requests
        }
        let id: Id16 = random_array();
        ms.pairings.xx_in.insert(
            id,
            XxIn {
                hs,
                h1,
                reply_tag: post.reply_tag,
                return_tag,
                post: rec.owner,
                seed: random_array(),
                accepted: false,
                created: now,
            },
        );
        self.emit(Event::PairingResult {
            pairing: id,
            outcome: PairingOutcome::Request { channel },
        });
        Ok(())
    }

    fn on_xx2(&mut self, rec: &TagKeyRec, noise: &[u8]) -> CoreResult<()> {
        let ms = self.ms()?;
        let Some(mut out) = ms.pairings.xx_out.remove(&rec.owner) else {
            return Ok(());
        };
        let payload = out.hs.read(noise).map_err(|_| CoreError::Crypto)?;
        let mut r = Reader::new(&payload);
        let n = r.u16().map_err(|_| CoreError::Crypto)? as usize;
        let card = ContactCard::decode_verified(r.take(n).map_err(|_| CoreError::Crypto)?)
            .map_err(|_| CoreError::Crypto)?;
        let pk: [u8; 32] = r.arr().map_err(|_| CoreError::Crypto)?;
        let sig: [u8; 64] = r.arr().map_err(|_| CoreError::Crypto)?;
        if pk != out.post_pk || Some(card.noise_static) != out.hs.remote_static() {
            return Err(CoreError::Crypto);
        }
        mesh_crypto::ed25519_verify(&pk, &[b"xx-bind-v1".as_slice(), &out.h1].concat(), &sig)
            .map_err(|_| CoreError::Crypto)?;
        let mine = ms.me.contact_card(out.seed).encode();
        let m3 = out.hs.write(&mine).map_err(|_| CoreError::Crypto)?;
        self.save_identity_pub();
        let env = self.handshake_env(&out.reply_tag, KIND_XX3, &m3, None)?;
        self.queue_own(env);
        self.remove_tagkey(&rec.owner);
        let _ = out.return_tag;
        let contact = self.add_contact(&card, false, out.seed)?;
        let nickname = self.peer_name(&contact);
        self.emit(Event::PairingResult {
            pairing: rec.owner,
            outcome: PairingOutcome::Paired {
                contact,
                nickname,
                verified: false,
            },
        });
        Ok(())
    }

    fn on_xx3(&mut self, rec: &TagKeyRec, noise: &[u8]) -> CoreResult<()> {
        let ms = self.ms()?;
        let ids: Vec<Id16> = ms
            .pairings
            .xx_in
            .iter()
            .filter(|(_, p)| p.accepted && p.reply_tag == rec.tag)
            .map(|(k, _)| *k)
            .collect();
        for id in ids {
            let ms = self.ms()?;
            let p = ms.pairings.xx_in.get_mut(&id).unwrap();
            let Ok(payload) = p.hs.read(noise) else {
                continue;
            };
            let p = ms.pairings.xx_in.remove(&id).unwrap();
            let card = ContactCard::decode_verified(&payload).map_err(|_| CoreError::Crypto)?;
            if Some(card.noise_static) != p.hs.remote_static() {
                return Err(CoreError::Crypto);
            }
            let contact = self.add_contact(&card, false, p.seed)?;
            let nickname = self.peer_name(&contact);
            self.emit(Event::PairingResult {
                pairing: id,
                outcome: PairingOutcome::Paired {
                    contact,
                    nickname,
                    verified: false,
                },
            });
            return Ok(());
        }
        Ok(())
    }

    /// Store a verified contact card (15.1 step 6, 15.5).
    pub(crate) fn add_contact(
        &mut self,
        card: &ContactCard,
        verified: bool,
        my_seed: [u8; 32],
    ) -> CoreResult<Id16> {
        let now = self.now();
        let ms = self.ms()?;
        let my_ed = ms.me.ed25519();
        let tag_key = mesh_crypto::pairwise_tag_key(&my_seed, &card.tag_seed).0;
        let existing = ms
            .peers
            .iter()
            .find(|(_, p)| p.ed25519 == card.ed25519)
            .map(|(k, p)| (*k, p.clone()));
        // A different key for the nickname of a verified contact: Key changed (15.5).
        let changed: Vec<Id16> = ms
            .peers
            .iter()
            .filter(|(_, p)| {
                p.listed
                    && p.nickname == card.nickname
                    && p.ed25519 != card.ed25519
                    && p.trust == Trust::Verified
            })
            .map(|(k, _)| *k)
            .collect();
        let (id, mut rec) = match existing {
            Some((id, mut p)) => {
                p.trust = match (verified, p.trust, p.listed) {
                    (true, _, _) => Trust::Verified,
                    (false, Trust::Verified, true) => Trust::Verified,
                    _ => Trust::Unverified,
                };
                (id, p)
            }
            None => (
                random_array(),
                PeerRec {
                    ed25519: card.ed25519,
                    curve25519: card.curve25519,
                    noise_static: card.noise_static,
                    nickname: card.nickname.clone(),
                    trust: if verified {
                        Trust::Verified
                    } else {
                        Trust::Unverified
                    },
                    listed: true,
                    presence_key: card.presence_key,
                    tag_key,
                    remote_otks: vec![],
                    remote_fallback: card.fallback_key,
                    my_otks_held: 0,
                    conversation: None,
                    created_ms: now,
                },
            ),
        };
        rec.listed = true;
        rec.nickname = card.nickname.clone();
        rec.curve25519 = card.curve25519;
        rec.noise_static = card.noise_static;
        rec.presence_key = card.presence_key;
        rec.tag_key = tag_key;
        rec.remote_otks = card.one_time_keys.clone();
        rec.remote_fallback = card.fallback_key;
        rec.my_otks_held = CARD_OTKS as u32;
        self.install_peer(id, rec);
        self.save_peer_sigs(&id, card.binding_sig, card.fallback_sig);
        for c in changed {
            if let Some(p) = self.msg.as_mut().and_then(|m| m.peers.get_mut(&c)) {
                p.trust = Trust::KeyChanged;
            }
            self.save_peer(&c);
            self.emit(Event::Warning {
                warning: Warning::KeyChanged { contact: c },
            });
        }
        self.ensure_direct_conv(&id)?;
        // Fresh pairing: drop old sessions; the lower identity key starts Olm.
        if let Some(ms) = self.msg.as_mut() {
            ms.sessions.remove(&id);
        }
        if my_ed < card.ed25519 {
            let _ = self.send_control(&id, mesh_wire::Content::SessionInit, now);
        }
        if let Some(contact) = self.contact_view(&id) {
            self.emit(Event::ContactChanged { contact });
        }
        self.emit(Event::ConversationsChanged);
        Ok(id)
    }

    pub(crate) fn pairing_slow_tick(&mut self, now: u64) {
        let Some(ms) = self.msg.as_mut() else { return };
        let mut failed = Vec::new();
        let mut tags = Vec::new();
        ms.pairings.ik_show.retain(|id, s| {
            let keep = now.saturating_sub(s.created) < PAIRING_WINDOW_MS + 5_000;
            if !keep {
                tags.push(*id);
            }
            keep
        });
        ms.pairings.ik_scan.retain(|id, s| {
            let keep = now.saturating_sub(s.created) < PAIRING_WINDOW_MS;
            if !keep {
                tags.push(*id);
                failed.push(*id);
            }
            keep
        });
        ms.pairings.xx_out.retain(|id, s| {
            let keep = now.saturating_sub(s.created) < REMOTE_PAIRING_LIFETIME_MS;
            if !keep {
                tags.push(*id);
                failed.push(*id);
            }
            keep
        });
        ms.pairings
            .xx_in
            .retain(|_, s| now.saturating_sub(s.created) < REMOTE_PAIRING_LIFETIME_MS);
        // Reply tags of our posts live as long as the posts (72 h).
        let old_reply: Vec<Id16> = self
            .relay
            .as_ref()
            .map(|r| {
                r.tagkeys
                    .iter()
                    .filter(|(_, t)| {
                        t.kind == TagKind::Reply
                            && now.saturating_sub(t.created_ms) > EXPIRY_MAX_SECS * 1000
                    })
                    .map(|(k, _)| *k)
                    .collect()
            })
            .unwrap_or_default();
        tags.extend(old_reply);
        for id in tags {
            self.remove_tagkey(&id);
        }
        for id in failed {
            self.emit(Event::PairingResult {
                pairing: id,
                outcome: PairingOutcome::Failed("timed out".into()),
            });
        }
    }
}
