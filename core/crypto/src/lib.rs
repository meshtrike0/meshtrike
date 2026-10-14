//! Cryptographic building blocks: hashing and KDFs, the outer envelope AEAD,
//! recipient tags and derived keys (sections 5, 7), safety numbers (4),
//! Noise wrappers (6), proof-of-work (8). Olm/Megolm come from `vodozemac`,
//! re-exported here so every crate uses the same version.
#![forbid(unsafe_code)]

pub mod noise;
pub mod pow;

pub use vodozemac;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, ZeroizeOnDrop};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("authentication failed")]
    Auth,
    #[error("bad key or signature encoding")]
    Encoding,
    #[error("signature invalid")]
    Signature,
    #[error("noise: {0}")]
    Noise(String),
    #[error("randomness unavailable")]
    Rng,
}

// ===== Randomness =====

/// Fill `buf` from the OS secure random generator.
pub fn fill_random(buf: &mut [u8]) {
    getrandom::fill(buf).expect("OS random generator unavailable");
}

pub fn random_array<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    fill_random(&mut out);
    out
}

// ===== Hashes and KDFs =====

pub fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

pub fn sha512(parts: &[&[u8]]) -> [u8; 64] {
    let mut h = Sha512::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

pub fn hmac16(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    let full = hmac_sha256(key, parts);
    let mut out = [0u8; 16];
    out.copy_from_slice(&full[..16]);
    out
}

/// HKDF-SHA256 with no salt, 32-byte output.
pub fn hkdf32(ikm: &[u8], info: &[&[u8]]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let info: Vec<u8> = info.concat();
    let mut okm = [0u8; 32];
    hk.expand(&info, &mut okm)
        .expect("32 bytes is a valid HKDF length");
    okm
}

/// Constant-time equality.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

// ===== Outer envelope AEAD (A.1) =====

/// A 32-byte symmetric key that is wiped on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct SymKey(pub [u8; 32]);

impl std::fmt::Debug for SymKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SymKey(..)")
    }
}

fn xnonce(message_id: &[u8; 16]) -> XNonce {
    let mut n = [0u8; 24];
    n[..16].copy_from_slice(message_id);
    XNonce::from(n)
}

pub fn seal(key: &SymKey, message_id: &[u8; 16], ad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(&key.0.into());
    cipher
        .encrypt(
            &xnonce(message_id),
            Payload {
                msg: plaintext,
                aad: ad,
            },
        )
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory buffers")
}

pub fn open(
    key: &SymKey,
    message_id: &[u8; 16],
    ad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new(&key.0.into());
    cipher
        .decrypt(
            &xnonce(message_id),
            Payload {
                msg: ciphertext,
                aad: ad,
            },
        )
        .map_err(|_| CryptoError::Auth)
}

// ===== Tags and derived keys (sections 5, 7, A.4) =====

/// 0x00 if the sender's identity key sorts lower than the recipient's, else 0x01.
pub fn direction(sender_ed: &[u8; 32], recipient_ed: &[u8; 32]) -> u8 {
    if sender_ed < recipient_ed {
        0
    } else {
        1
    }
}

/// Pairwise tag key from the two tag seeds, sorted byte-wise.
pub fn pairwise_tag_key(seed_a: &[u8; 32], seed_b: &[u8; 32]) -> SymKey {
    let (lo, hi) = if seed_a <= seed_b {
        (seed_a, seed_b)
    } else {
        (seed_b, seed_a)
    };
    SymKey(hkdf32(
        &[lo.as_slice(), hi.as_slice()].concat(),
        &[b"tag-v1"],
    ))
}

pub fn direct_tag(tag_key: &SymKey, dir: u8, expiry: u32) -> [u8; 16] {
    hmac16(&tag_key.0, &[b"tag-v2", &[dir], &expiry.to_be_bytes()])
}

pub fn direct_env_key(tag_key: &SymKey, dir: u8) -> SymKey {
    SymKey(hkdf32(&tag_key.0, &[b"env-v2", &[dir]]))
}

pub fn group_tag(group_tag_key: &SymKey, expiry: u32) -> [u8; 16] {
    hmac16(&group_tag_key.0, &[b"tag-v2", &expiry.to_be_bytes()])
}

pub fn group_env_key(group_tag_key: &SymKey) -> SymKey {
    SymKey(hkdf32(&group_tag_key.0, &[b"env-v2"]))
}

/// NFC-normalised, lower-cased, trimmed channel name.
pub fn normalize_channel_name(name: &str) -> String {
    name.trim().to_lowercase().nfc().collect()
}

pub fn channel_key(name: &str) -> SymKey {
    SymKey(hkdf32(
        normalize_channel_name(name).as_bytes(),
        &[b"chan-v1"],
    ))
}

pub fn channel_hash(channel_key: &SymKey) -> [u8; 16] {
    let h = sha256(&[b"chan-id-v1", &channel_key.0]);
    h[..16].try_into().unwrap()
}

pub fn handshake_key(tag: &[u8; 16]) -> SymKey {
    SymKey(hkdf32(tag, &[b"hs-v2"]))
}

pub fn ack_secret(env_key: &SymKey, message_id: &[u8; 16]) -> [u8; 16] {
    hmac16(&env_key.0, &[b"ack-v1", message_id])
}

pub fn ack_proof(ack_secret: &[u8; 16]) -> [u8; 8] {
    sha256(&[ack_secret])[..8].try_into().unwrap()
}

/// Presence tag for a beacon (section 5).
pub fn presence_tag(presence_key: &[u8; 32], epoch: u64, ephemeral_id: &[u8; 16]) -> [u8; 16] {
    hmac16(presence_key, &[&epoch.to_be_bytes(), ephemeral_id])
}

/// Format 16 bytes as an RFC 4122 version-4-looking UUID.
pub fn as_v4_uuid(mut b: [u8; 16]) -> [u8; 16] {
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    b
}

pub fn community_uuid(community_key: &[u8; 32], epoch: u64) -> [u8; 16] {
    as_v4_uuid(hmac16(community_key, &[b"uuid-v1", &epoch.to_be_bytes()]))
}

pub fn pairing_uuid(token: &[u8; 16]) -> [u8; 16] {
    as_v4_uuid(hmac16(token, &[b"pair-v1"]))
}

// ===== Safety number (section 4) =====

pub fn safety_number(a: &[u8; 32], b: &[u8; 32]) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let h = sha512(&[lo, hi]);
    h[..30]
        .chunks(5)
        .map(|c| {
            let v = c.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64);
            format!("{:05}", v % 100_000)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ===== Ed25519 (contact cards, channel pseudonyms) =====

/// Ed25519 signing key held as its 32-byte seed; wiped on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Ed25519Seed(pub [u8; 32]);

impl Ed25519Seed {
    pub fn generate() -> Self {
        Self(random_array())
    }
    pub fn public(&self) -> [u8; 32] {
        SigningKey::from_bytes(&self.0).verifying_key().to_bytes()
    }
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        SigningKey::from_bytes(&self.0).sign(msg).to_bytes()
    }
}

pub fn ed25519_verify(public: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> Result<(), CryptoError> {
    let vk = VerifyingKey::from_bytes(public).map_err(|_| CryptoError::Encoding)?;
    let sig = ed25519_dalek::Signature::from_bytes(sig);
    vk.verify_strict(msg, &sig)
        .map_err(|_| CryptoError::Signature)
}

// ===== X25519 static keys (Noise) =====

pub fn x25519_public(secret: &[u8; 32]) -> [u8; 32] {
    let sk = x25519_dalek::StaticSecret::from(*secret);
    x25519_dalek::PublicKey::from(&sk).to_bytes()
}

/// X25519 shared secret; refuses a low-order peer key (all-zero result).
pub fn x25519_shared(secret: &[u8; 32], peer_public: &[u8; 32]) -> Result<[u8; 32], CryptoError> {
    let sk = x25519_dalek::StaticSecret::from(*secret);
    let shared = sk.diffie_hellman(&x25519_dalek::PublicKey::from(*peer_public));
    if !shared.was_contributory() {
        return Err(CryptoError::Auth);
    }
    Ok(shared.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_ad_binding() {
        let k = SymKey(random_array());
        let id = random_array::<16>();
        let ct = seal(&k, &id, b"header", b"hello");
        assert_eq!(open(&k, &id, b"header", &ct).unwrap(), b"hello");
        assert_eq!(open(&k, &id, b"HEADER", &ct), Err(CryptoError::Auth));
        assert_eq!(ct.len(), 5 + 16);
    }

    #[test]
    fn direction_tags_differ() {
        let k = pairwise_tag_key(&[1; 32], &[2; 32]);
        assert_eq!(k, pairwise_tag_key(&[2; 32], &[1; 32]));
        assert_ne!(direct_tag(&k, 0, 3600), direct_tag(&k, 1, 3600));
        assert_ne!(direct_tag(&k, 0, 3600), direct_tag(&k, 0, 7200));
        assert_ne!(direct_env_key(&k, 0), direct_env_key(&k, 1));
    }

    #[test]
    fn safety_number_symmetric_format() {
        let s = safety_number(&[7; 32], &[9; 32]);
        assert_eq!(s, safety_number(&[9; 32], &[7; 32]));
        assert_eq!(s.len(), 6 * 5 + 5);
        assert!(s
            .split(' ')
            .all(|g| g.len() == 5 && g.chars().all(|c| c.is_ascii_digit())));
    }

    #[test]
    fn uuid_bits() {
        let u = community_uuid(&[3; 32], 42);
        assert_eq!(u[6] >> 4, 4);
        assert_eq!(u[8] >> 6, 0b10);
    }

    #[test]
    fn channel_names_normalize() {
        assert_eq!(channel_key(" Plaza "), channel_key("plaza"));
        assert_eq!(
            channel_key("Caf\u{0065}\u{0301}"),
            channel_key("caf\u{00e9}")
        );
    }

    #[test]
    fn ed25519_sign_verify() {
        let s = Ed25519Seed::generate();
        let sig = s.sign(b"m");
        ed25519_verify(&s.public(), b"m", &sig).unwrap();
        assert!(ed25519_verify(&s.public(), b"x", &sig).is_err());
    }

    #[test]
    fn ack_commitment() {
        let k = SymKey([5; 32]);
        let id = [1; 16];
        let sec = ack_secret(&k, &id);
        assert_eq!(ack_proof(&sec), ack_proof(&ack_secret(&k, &id)));
    }
}
