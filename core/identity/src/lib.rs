//! Identity and contact exchange (sections 4, 15.1): the local account
//! (Olm account + Noise static key + presence key), contact cards, and the
//! in-person pairing QR payload.
#![forbid(unsafe_code)]

use mesh_constants::*;
use mesh_crypto::vodozemac::olm::{Account, AccountPickle};
use mesh_crypto::vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};
use mesh_crypto::{ed25519_verify, random_array, x25519_public};
use mesh_wire::{MemberKeys, Reader, WireError};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("malformed")]
    Malformed,
    #[error("signature")]
    Signature,
    #[error("pickle")]
    Pickle,
    #[error("nickname")]
    Nickname,
}

impl From<WireError> for IdentityError {
    fn from(_: WireError) -> Self {
        IdentityError::Malformed
    }
}

pub fn validate_nickname(n: &str) -> Result<(), IdentityError> {
    if n.is_empty() || n.len() > MAX_NICKNAME_BYTES {
        return Err(IdentityError::Nickname);
    }
    Ok(())
}

/// The local user's long-term keys.
pub struct LocalIdentity {
    pub account: Account,
    pub noise_static_sk: Zeroizing<[u8; 32]>,
    pub binding_sig: [u8; 64],
    pub fallback_pk: [u8; 32],
    pub fallback_sig: [u8; 64],
    pub fallback_created_ms: u64,
    pub presence_key: Zeroizing<[u8; 32]>,
    pub nickname: String,
}

impl LocalIdentity {
    pub fn generate(nickname: &str, now_ms: u64) -> Result<Self, IdentityError> {
        validate_nickname(nickname)?;
        let mut account = Account::new();
        let noise_static_sk = Zeroizing::new(random_array::<32>());
        let noise_pk = x25519_public(&noise_static_sk);
        let binding = [account.curve25519_key().to_bytes().as_slice(), &noise_pk].concat();
        let binding_sig = account.sign(&binding).to_bytes();
        account.generate_one_time_keys(LOCAL_OTK_POOL);
        let mut me = Self {
            account,
            noise_static_sk,
            binding_sig,
            fallback_pk: [0; 32],
            fallback_sig: [0; 64],
            fallback_created_ms: now_ms,
            presence_key: Zeroizing::new(random_array()),
            nickname: nickname.to_string(),
        };
        me.rotate_fallback(now_ms);
        Ok(me)
    }

    pub fn ed25519(&self) -> [u8; 32] {
        *self.account.ed25519_key().as_bytes()
    }
    pub fn curve25519(&self) -> [u8; 32] {
        self.account.curve25519_key().to_bytes()
    }
    pub fn noise_pk(&self) -> [u8; 32] {
        x25519_public(&self.noise_static_sk)
    }
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.account.sign(msg).to_bytes()
    }

    /// Generate a new fallback key (7-day rotation, section 4). vodozemac
    /// keeps the previous fallback key so in-flight PreKey messages still work.
    pub fn rotate_fallback(&mut self, now_ms: u64) {
        self.account.generate_fallback_key();
        if let Some(pk) = self.account.fallback_key().values().next() {
            self.fallback_pk = pk.to_bytes();
        }
        self.fallback_sig = self.sign(&fallback_sig_bytes(&self.fallback_pk));
        self.fallback_created_ms = now_ms;
        self.account.mark_keys_as_published();
    }

    /// Fresh one-time keys handed to one contact.
    pub fn fresh_one_time_keys(&mut self, n: usize) -> Vec<[u8; 32]> {
        let created = self.account.generate_one_time_keys(n).created;
        self.account.mark_keys_as_published();
        created.iter().map(|k| k.to_bytes()).collect()
    }

    pub fn contact_card(&mut self, tag_seed: [u8; 32]) -> ContactCard {
        let otks = self.fresh_one_time_keys(CARD_OTKS);
        let mut card = ContactCard {
            ed25519: self.ed25519(),
            curve25519: self.curve25519(),
            noise_static: self.noise_pk(),
            binding_sig: self.binding_sig,
            fallback_key: self.fallback_pk,
            fallback_sig: self.fallback_sig,
            one_time_keys: otks,
            presence_key: *self.presence_key,
            tag_seed,
            nickname: self.nickname.clone(),
            signature: [0; 64],
        };
        card.signature = self.sign(&card.encode_unsigned());
        card
    }

    pub fn member_keys(&self) -> MemberKeys {
        MemberKeys {
            ed25519: self.ed25519(),
            curve25519: self.curve25519(),
            noise_static: self.noise_pk(),
            binding_sig: self.binding_sig,
            fallback_key: self.fallback_pk,
            fallback_sig: self.fallback_sig,
        }
    }

    /// Serialise for storage; the account pickle is encrypted with `pickle_key`.
    pub fn to_bytes(&self, pickle_key: &[u8; 32]) -> Vec<u8> {
        let pickle = self.account.pickle().encrypt(pickle_key);
        let mut v = Vec::new();
        v.extend_from_slice(&(pickle.len() as u32).to_be_bytes());
        v.extend_from_slice(pickle.as_bytes());
        v.extend_from_slice(&*self.noise_static_sk);
        v.extend_from_slice(&self.binding_sig);
        v.extend_from_slice(&self.fallback_pk);
        v.extend_from_slice(&self.fallback_sig);
        v.extend_from_slice(&self.fallback_created_ms.to_be_bytes());
        v.extend_from_slice(&*self.presence_key);
        v.push(self.nickname.len() as u8);
        v.extend_from_slice(self.nickname.as_bytes());
        v
    }

    pub fn from_bytes(b: &[u8], pickle_key: &[u8; 32]) -> Result<Self, IdentityError> {
        let mut r = Reader::new(b);
        let n = r.u32()? as usize;
        let pickle = std::str::from_utf8(r.take(n)?).map_err(|_| IdentityError::Pickle)?;
        let account = Account::from_pickle(
            AccountPickle::from_encrypted(pickle, pickle_key).map_err(|_| IdentityError::Pickle)?,
        );
        let noise_static_sk = Zeroizing::new(r.arr()?);
        let binding_sig = r.arr()?;
        let fallback_pk = r.arr()?;
        let fallback_sig = r.arr()?;
        let fallback_created_ms = r.u64()?;
        let presence_key = Zeroizing::new(r.arr()?);
        let nl = r.u8()? as usize;
        let nickname =
            String::from_utf8(r.take(nl)?.to_vec()).map_err(|_| IdentityError::Malformed)?;
        Ok(Self {
            account,
            noise_static_sk,
            binding_sig,
            fallback_pk,
            fallback_sig,
            fallback_created_ms,
            presence_key,
            nickname,
        })
    }
}

pub fn fallback_sig_bytes(fallback_pk: &[u8; 32]) -> Vec<u8> {
    [b"fallback-v1".as_slice(), fallback_pk].concat()
}

pub fn verify_binding(
    ed: &[u8; 32],
    curve: &[u8; 32],
    noise: &[u8; 32],
    sig: &[u8; 64],
) -> Result<(), IdentityError> {
    ed25519_verify(ed, &[curve.as_slice(), noise].concat(), sig)
        .map_err(|_| IdentityError::Signature)
}

/// Verify the keys a group membership message carries for a non-contact member.
pub fn verify_member(m: &MemberKeys) -> Result<(), IdentityError> {
    verify_binding(&m.ed25519, &m.curve25519, &m.noise_static, &m.binding_sig)?;
    ed25519_verify(
        &m.ed25519,
        &fallback_sig_bytes(&m.fallback_key),
        &m.fallback_sig,
    )
    .map_err(|_| IdentityError::Signature)
}

/// Contact card (section 4): sent only inside Noise handshakes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCard {
    pub ed25519: [u8; 32],
    pub curve25519: [u8; 32],
    pub noise_static: [u8; 32],
    pub binding_sig: [u8; 64],
    pub fallback_key: [u8; 32],
    pub fallback_sig: [u8; 64],
    pub one_time_keys: Vec<[u8; 32]>,
    pub presence_key: [u8; 32],
    pub tag_seed: [u8; 32],
    pub nickname: String,
    pub signature: [u8; 64],
}

impl ContactCard {
    fn encode_unsigned(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(740);
        v.push(0x01);
        v.extend_from_slice(&self.ed25519);
        v.extend_from_slice(&self.curve25519);
        v.extend_from_slice(&self.noise_static);
        v.extend_from_slice(&self.binding_sig);
        v.extend_from_slice(&self.fallback_key);
        v.extend_from_slice(&self.fallback_sig);
        v.push(self.one_time_keys.len() as u8);
        self.one_time_keys
            .iter()
            .for_each(|k| v.extend_from_slice(k));
        v.extend_from_slice(&self.presence_key);
        v.extend_from_slice(&self.tag_seed);
        v.push(self.nickname.len() as u8);
        v.extend_from_slice(self.nickname.as_bytes());
        v
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.encode_unsigned();
        v.extend_from_slice(&self.signature);
        v
    }

    /// Parse and verify both signatures (section 4: MUST reject on failure).
    pub fn decode_verified(b: &[u8]) -> Result<Self, IdentityError> {
        let mut r = Reader::new(b);
        if r.u8()? != 0x01 {
            return Err(IdentityError::Malformed);
        }
        let ed25519 = r.arr()?;
        let curve25519 = r.arr()?;
        let noise_static = r.arr()?;
        let binding_sig = r.arr()?;
        let fallback_key = r.arr()?;
        let fallback_sig = r.arr()?;
        let n = r.u8()? as usize;
        if n > CARD_OTKS {
            return Err(IdentityError::Malformed);
        }
        let one_time_keys = (0..n).map(|_| r.arr()).collect::<Result<Vec<_>, _>>()?;
        let presence_key = r.arr()?;
        let tag_seed = r.arr()?;
        let nl = r.u8()? as usize;
        let nickname =
            String::from_utf8(r.take(nl)?.to_vec()).map_err(|_| IdentityError::Malformed)?;
        validate_nickname(&nickname)?;
        let signature = r.arr()?;
        if r.remaining() != 0 {
            return Err(IdentityError::Malformed);
        }
        let card = Self {
            ed25519,
            curve25519,
            noise_static,
            binding_sig,
            fallback_key,
            fallback_sig,
            one_time_keys,
            presence_key,
            tag_seed,
            nickname,
            signature,
        };
        ed25519_verify(&card.ed25519, &card.encode_unsigned(), &card.signature)
            .map_err(|_| IdentityError::Signature)?;
        verify_binding(
            &card.ed25519,
            &card.curve25519,
            &card.noise_static,
            &card.binding_sig,
        )?;
        ed25519_verify(
            &card.ed25519,
            &fallback_sig_bytes(&card.fallback_key),
            &card.fallback_sig,
        )
        .map_err(|_| IdentityError::Signature)?;
        // Keys must parse as valid curve points / Ed25519 keys.
        Ed25519PublicKey::from_slice(&card.ed25519).map_err(|_| IdentityError::Malformed)?;
        let _ = Curve25519PublicKey::from_bytes(card.curve25519);
        let _ =
            Ed25519Signature::from_slice(&card.signature).map_err(|_| IdentityError::Malformed)?;
        Ok(card)
    }
}

/// In-person pairing QR payload (15.1): 97 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingQr {
    pub ed25519: [u8; 32],
    pub noise_static: [u8; 32],
    pub token: [u8; 16],
    pub ephemeral_id: [u8; 16],
}

impl PairingQr {
    pub fn encode(&self) -> Vec<u8> {
        [
            &[0x01u8][..],
            &self.ed25519,
            &self.noise_static,
            &self.token,
            &self.ephemeral_id,
        ]
        .concat()
    }
    pub fn decode(b: &[u8]) -> Result<Self, IdentityError> {
        if b.len() != PAIRING_QR_LEN || b[0] != 0x01 {
            return Err(IdentityError::Malformed);
        }
        let mut r = Reader::new(&b[1..]);
        Ok(Self {
            ed25519: r.arr()?,
            noise_static: r.arr()?,
            token: r.arr()?,
            ephemeral_id: r.arr()?,
        })
    }
}

/// Community QR payload (12.1): "MQC1" ‖ key (32) ‖ name.
pub fn encode_community_qr(key: &[u8; 32], name: &str) -> Vec<u8> {
    [b"MQC1".as_slice(), key, name.as_bytes()].concat()
}

pub fn decode_community_qr(b: &[u8]) -> Result<([u8; 32], String), IdentityError> {
    if b.len() < 36 || &b[..4] != b"MQC1" || b.len() > 36 + 64 {
        return Err(IdentityError::Malformed);
    }
    let name = String::from_utf8(b[36..].to_vec()).map_err(|_| IdentityError::Malformed)?;
    if name.is_empty() {
        return Err(IdentityError::Malformed);
    }
    Ok((b[4..36].try_into().unwrap(), name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_roundtrip_and_tamper() {
        let mut me = LocalIdentity::generate("alice", 0).unwrap();
        let card = me.contact_card([3; 32]);
        let enc = card.encode();
        assert!(enc.len() <= 740);
        assert_eq!(ContactCard::decode_verified(&enc).unwrap(), card);
        for i in [5usize, 100, 300, enc.len() - 1] {
            let mut bad = enc.clone();
            bad[i] ^= 1;
            assert!(ContactCard::decode_verified(&bad).is_err(), "byte {i}");
        }
        verify_member(&me.member_keys()).unwrap();
    }

    #[test]
    fn identity_persistence() {
        let me = LocalIdentity::generate("bob", 5).unwrap();
        let b = me.to_bytes(&[1; 32]);
        let back = LocalIdentity::from_bytes(&b, &[1; 32]).unwrap();
        assert_eq!(back.ed25519(), me.ed25519());
        assert_eq!(back.noise_pk(), me.noise_pk());
        assert!(LocalIdentity::from_bytes(&b, &[2; 32]).is_err());
    }

    #[test]
    fn qr_payloads() {
        let q = PairingQr {
            ed25519: [1; 32],
            noise_static: [2; 32],
            token: [3; 16],
            ephemeral_id: [4; 16],
        };
        assert_eq!(q.encode().len(), PAIRING_QR_LEN);
        assert_eq!(PairingQr::decode(&q.encode()).unwrap(), q);
        let c = encode_community_qr(&[5; 32], "north");
        assert_eq!(
            decode_community_qr(&c).unwrap(),
            ([5; 32], "north".to_string())
        );
    }

    #[test]
    fn olm_session_between_cards() {
        use mesh_crypto::vodozemac::olm::{OlmMessage, SessionConfig};
        let a = LocalIdentity::generate("a", 0).unwrap();
        let mut b = LocalIdentity::generate("b", 0).unwrap();
        let bc = b.contact_card([0; 32]);
        let mut s = a
            .account
            .create_outbound_session(
                SessionConfig::version_1(),
                Curve25519PublicKey::from_bytes(bc.curve25519),
                Curve25519PublicKey::from_bytes(bc.one_time_keys[0]),
            )
            .unwrap();
        let m = s.encrypt(b"hi").unwrap();
        let OlmMessage::PreKey(pk) = m else { panic!() };
        let res = b
            .account
            .create_inbound_session(
                SessionConfig::version_1(),
                Curve25519PublicKey::from_bytes(a.curve25519()),
                &pk,
            )
            .unwrap();
        assert_eq!(res.plaintext, b"hi");
    }
}
