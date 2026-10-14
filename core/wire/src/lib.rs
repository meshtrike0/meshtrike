//! Wire encodings (section 7, amendment A): envelope, outer sealing, inner
//! plaintext, broadcast posts, link frames, summary vectors. Fixed big-endian
//! layouts only. Every parser is total: malformed input returns `WireError`.
#![forbid(unsafe_code)]

pub mod frames;
pub mod inner;

pub use frames::*;
pub use inner::*;

use mesh_constants::*;
use mesh_crypto::{open, seal, SymKey};

#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum WireError {
    #[error("truncated")]
    Truncated,
    #[error("bad version")]
    Version,
    #[error("bad type")]
    Type,
    #[error("bad length")]
    Length,
    #[error("bad field")]
    Field,
    #[error("authentication failed")]
    Auth,
    #[error("too large")]
    TooLarge,
}

// ===== Byte helpers =====

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        if end > self.buf.len() {
            return Err(WireError::Truncated);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn arr<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.arr()?))
    }
    pub fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.arr()?))
    }
    pub fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_be_bytes(self.arr()?))
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.buf[self.pos..];
        self.pos = self.buf.len();
        s
    }
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
}

// ===== Envelope =====

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EnvType {
    Direct = 0x01,
    Group = 0x02,
    Broadcast = 0x03,
    Handshake = 0x04,
    Ack = 0x05,
    Cover = 0x06,
}

impl EnvType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x01 => Self::Direct,
            0x02 => Self::Group,
            0x03 => Self::Broadcast,
            0x04 => Self::Handshake,
            0x05 => Self::Ack,
            0x06 => Self::Cover,
            _ => return None,
        })
    }
    pub fn is_sealed(self) -> bool {
        !matches!(self, Self::Ack | Self::Cover)
    }
}

pub const FLAG_LORA: u8 = 0x01;

pub type MessageId = [u8; ID_LEN];
pub type Tag = [u8; TAG_LEN];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub typ: EnvType,
    pub ttl: u8,
    pub flags: u8,
    pub message_id: MessageId,
    pub tag: Tag,
    /// Unix seconds, a whole hour.
    pub expiry: u32,
    pub proof: [u8; 8],
    pub payload: Vec<u8>,
}

impl Envelope {
    pub fn header(&self, ttl: u8) -> [u8; ENVELOPE_HEADER_LEN] {
        let mut h = [0u8; ENVELOPE_HEADER_LEN];
        h[0] = PROTOCOL_VERSION;
        h[1] = self.typ as u8;
        h[2] = ttl;
        h[3] = self.flags;
        h[4..20].copy_from_slice(&self.message_id);
        h[20..36].copy_from_slice(&self.tag);
        h[36..40].copy_from_slice(&self.expiry.to_be_bytes());
        h[40..48].copy_from_slice(&self.proof);
        // 48..50 reserved, zero
        h
    }

    /// Associated data for the outer layer: header with TTL = 0. For
    /// Broadcast the proof field is zeroed too: the PoW nonce is computed over
    /// the sealed payload and is itself protected by the PoW check (A.7).
    pub fn ad(&self) -> [u8; ENVELOPE_HEADER_LEN] {
        let mut h = self.header(0);
        if self.typ == EnvType::Broadcast {
            h[40..48].fill(0);
        }
        h
    }

    /// PoW prefix: header with TTL and proof zeroed.
    pub fn pow_prefix(&self) -> [u8; ENVELOPE_HEADER_LEN] {
        let mut h = self.header(0);
        h[40..48].fill(0);
        h
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(ENVELOPE_HEADER_LEN + self.payload.len());
        v.extend_from_slice(&self.header(self.ttl));
        v.extend_from_slice(&self.payload);
        v
    }

    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(buf);
        if r.u8()? != PROTOCOL_VERSION {
            return Err(WireError::Version);
        }
        let typ = EnvType::from_u8(r.u8()?).ok_or(WireError::Type)?;
        let ttl = r.u8()?;
        if ttl > MAX_TTL {
            return Err(WireError::Field);
        }
        let flags = r.u8()?;
        if flags & !FLAG_LORA != 0 {
            return Err(WireError::Field);
        }
        let message_id = r.arr()?;
        let tag = r.arr()?;
        let expiry = r.u32()?;
        if expiry % 3600 != 0 {
            return Err(WireError::Field);
        }
        let proof = r.arr()?;
        if r.u16()? != 0 {
            return Err(WireError::Field);
        }
        let payload = r.rest().to_vec();
        if !valid_payload_len(typ, flags, payload.len()) {
            return Err(WireError::Length);
        }
        Ok(Self {
            typ,
            ttl,
            flags,
            message_id,
            tag,
            expiry,
            proof,
            payload,
        })
    }

    pub fn encoded_len(&self) -> usize {
        ENVELOPE_HEADER_LEN + self.payload.len()
    }
}

pub fn valid_payload_len(typ: EnvType, flags: u8, len: usize) -> bool {
    if !typ.is_sealed() {
        return len == ACK_PAYLOAD_LEN;
    }
    if flags & FLAG_LORA != 0 {
        return typ == EnvType::Direct && len == LORA_WIRE_SIZE + AEAD_TAG_LEN;
    }
    WIRE_SIZES.iter().any(|w| w + AEAD_TAG_LEN == len)
}

/// Pick the wire size for an inner message: the bucket of its padded
/// plaintext, bumped to the next bucket if E2E overhead still does not fit.
pub fn wire_size_for(inner_msg_len: usize, lora: bool) -> Result<usize, WireError> {
    if lora {
        return if inner_msg_len + 2 <= LORA_WIRE_SIZE {
            Ok(LORA_WIRE_SIZE)
        } else {
            Err(WireError::TooLarge)
        };
    }
    WIRE_SIZES
        .iter()
        .copied()
        .find(|w| inner_msg_len + 2 <= *w)
        .ok_or(WireError::TooLarge)
}

/// Seal `inner_msg` into the envelope payload (A.1): len ‖ msg ‖ zero pad to W.
pub fn seal_into(env: &mut Envelope, key: &SymKey, inner_msg: &[u8]) -> Result<(), WireError> {
    let w = wire_size_for(inner_msg.len(), env.flags & FLAG_LORA != 0)?;
    let mut pt = Vec::with_capacity(w);
    pt.extend_from_slice(&(inner_msg.len() as u16).to_be_bytes());
    pt.extend_from_slice(inner_msg);
    pt.resize(w, 0);
    env.payload = Vec::new();
    env.payload = seal(key, &env.message_id, &env.ad(), &pt);
    Ok(())
}

pub fn open_payload(env: &Envelope, key: &SymKey) -> Result<Vec<u8>, WireError> {
    let pt = open(key, &env.message_id, &env.ad(), &env.payload).map_err(|_| WireError::Auth)?;
    let mut r = Reader::new(&pt);
    let len = r.u16()? as usize;
    let msg = r.take(len)?.to_vec();
    if r.rest().iter().any(|&b| b != 0) {
        return Err(WireError::Field);
    }
    Ok(msg)
}

// ===== Ack payload (A.3) =====

pub fn ack_payload(acked: &MessageId, secret: &[u8; 16]) -> Vec<u8> {
    let mut v = Vec::with_capacity(ACK_PAYLOAD_LEN);
    v.extend_from_slice(acked);
    v.extend_from_slice(secret);
    v.resize(ACK_PAYLOAD_LEN, 0);
    v
}

pub fn parse_ack(env: &Envelope) -> Result<(MessageId, [u8; 16]), WireError> {
    if env.typ != EnvType::Ack {
        return Err(WireError::Type);
    }
    let mut r = Reader::new(&env.payload);
    Ok((r.arr()?, r.arr()?))
}

// ===== Inner-message framing (A.3) =====

pub fn direct_inner(olm_type: u8, olm: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + olm.len());
    v.push(olm_type);
    v.extend_from_slice(olm);
    v
}

pub fn parse_direct_inner(msg: &[u8]) -> Result<(u8, &[u8]), WireError> {
    match msg.split_first() {
        Some((&t, rest)) if t <= SEALED_CHUNK => Ok((t, rest)),
        _ => Err(WireError::Field),
    }
}

/// Direct inner-message type for a chunk sealed with a media key (D42).
pub const SEALED_CHUNK: u8 = 2;

/// Sealed chunk body: message UUID (16) ‖ chunk index (2) ‖ AEAD ciphertext.
pub fn sealed_chunk_body(uuid: &[u8; 16], index: u16, sealed: &[u8]) -> Vec<u8> {
    [uuid.as_slice(), &index.to_be_bytes(), sealed].concat()
}

pub fn parse_sealed_chunk(b: &[u8]) -> Result<([u8; 16], u16, &[u8]), WireError> {
    if b.len() < 18 + 16 {
        return Err(WireError::Truncated);
    }
    Ok((
        b[..16].try_into().unwrap(),
        u16::from_be_bytes([b[16], b[17]]),
        &b[18..],
    ))
}

/// Seal one padded inner plaintext of a chunked message under its media key.
pub fn seal_chunk(key: &[u8; 32], uuid: &[u8; 16], index: u16, padded: &[u8]) -> Vec<u8> {
    let (nonce, ad) = chunk_nonce_ad(uuid, index);
    mesh_crypto::seal(&SymKey(*key), &nonce, &ad, padded)
}

pub fn open_chunk(
    key: &[u8; 32],
    uuid: &[u8; 16],
    index: u16,
    sealed: &[u8],
) -> Result<Vec<u8>, WireError> {
    let (nonce, ad) = chunk_nonce_ad(uuid, index);
    open(&SymKey(*key), &nonce, &ad, sealed).map_err(|_| WireError::Auth)
}

fn chunk_nonce_ad(uuid: &[u8; 16], index: u16) -> ([u8; 16], Vec<u8>) {
    let h = mesh_crypto::sha256(&[b"chunk-nonce-v1", uuid, &index.to_be_bytes()]);
    let ad = [b"chunk-v1".as_slice(), uuid, &index.to_be_bytes()].concat();
    (h[..16].try_into().unwrap(), ad)
}

/// Group inner message: kind 0x00 Megolm (session ID 32 ‖ message).
pub fn group_inner(session_id: &[u8; 32], megolm: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(32 + megolm.len());
    v.extend_from_slice(session_id);
    v.extend_from_slice(megolm);
    v
}

pub fn parse_group_inner(msg: &[u8]) -> Result<([u8; 32], &[u8]), WireError> {
    if msg.len() < 33 {
        return Err(WireError::Truncated);
    }
    Ok((msg[..32].try_into().unwrap(), &msg[32..]))
}

pub fn handshake_inner(kind: u8, noise: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + noise.len());
    v.push(kind);
    v.extend_from_slice(noise);
    v
}

pub fn parse_handshake_inner(msg: &[u8]) -> Result<(u8, &[u8]), WireError> {
    match msg.split_first() {
        Some((&k, rest)) if (1..=5).contains(&k) => Ok((k, rest)),
        _ => Err(WireError::Field),
    }
}

// ===== Broadcast post (section 7) =====

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub pseudonym_pk: [u8; 32],
    pub signature: [u8; 64],
    pub reply_tag: [u8; 16],
    pub inner: Vec<u8>,
}

impl Post {
    pub fn signing_bytes(
        channel_hash: &[u8; 16],
        message_id: &MessageId,
        pk: &[u8; 32],
        reply_tag: &[u8; 16],
        inner: &[u8],
    ) -> Vec<u8> {
        [
            b"post-v1".as_slice(),
            channel_hash,
            message_id,
            pk,
            reply_tag,
            inner,
        ]
        .concat()
    }
    pub fn encode(&self) -> Vec<u8> {
        [
            self.pseudonym_pk.as_slice(),
            &self.signature,
            &self.reply_tag,
            &self.inner,
        ]
        .concat()
    }
    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(buf);
        Ok(Self {
            pseudonym_pk: r.arr()?,
            signature: r.arr()?,
            reply_tag: r.arr()?,
            inner: r.rest().to_vec(),
        })
    }
}

/// Round a time up to the next whole hour (expiry field).
pub fn expiry_hour(unix_secs: u64) -> u32 {
    (unix_secs.div_ceil(3600) * 3600) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_crypto::random_array;

    fn env(typ: EnvType) -> Envelope {
        Envelope {
            typ,
            ttl: 6,
            flags: 0,
            message_id: random_array(),
            tag: random_array(),
            expiry: 3600 * 500_000,
            proof: [0; 8],
            payload: vec![],
        }
    }

    #[test]
    fn seal_roundtrip_all_buckets_and_sizes() {
        let key = SymKey(random_array());
        for (i, inner) in INNER_BUCKETS.iter().enumerate() {
            let mut e = env(EnvType::Direct);
            let msg = vec![7u8; inner + WIRE_MARGIN - 2];
            seal_into(&mut e, &key, &msg).unwrap();
            let bytes = e.encode();
            assert_eq!(
                bytes.len(),
                ENVELOPE_HEADER_LEN + WIRE_SIZES[i] + AEAD_TAG_LEN
            );
            let mut d = Envelope::decode(&bytes).unwrap();
            assert_eq!(open_payload(&d, &key).unwrap(), msg);
            // TTL is not authenticated; everything else is.
            d.ttl = 2;
            assert!(open_payload(&d, &key).is_ok());
            d.expiry += 3600;
            assert_eq!(open_payload(&d, &key), Err(WireError::Auth));
        }
    }

    #[test]
    fn lora_envelope_is_233_bytes() {
        let key = SymKey(random_array());
        let mut e = env(EnvType::Direct);
        e.flags = FLAG_LORA;
        seal_into(&mut e, &key, &[1u8; 160]).unwrap();
        assert_eq!(e.encode().len(), LORA_ENVELOPE_LEN);
        assert!(seal_into(&mut e, &key, &[1u8; 166]).is_err());
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(Envelope::decode(&[]).is_err());
        assert!(Envelope::decode(&[2, 9, 5, 0]).is_err());
        let mut e = env(EnvType::Ack);
        e.payload = ack_payload(&[1; 16], &[2; 16]);
        let b = e.encode();
        let d = Envelope::decode(&b).unwrap();
        assert_eq!(parse_ack(&d).unwrap(), ([1; 16], [2; 16]));
        let mut bad = b.clone();
        bad[0] = 1;
        assert_eq!(Envelope::decode(&bad), Err(WireError::Version));
        let mut bad = b.clone();
        bad[2] = 9;
        assert_eq!(Envelope::decode(&bad), Err(WireError::Field));
        let mut bad = b;
        bad.pop();
        assert_eq!(Envelope::decode(&bad), Err(WireError::Length));
    }

    #[test]
    fn expiry_rounding() {
        assert_eq!(expiry_hour(3600), 3600);
        assert_eq!(expiry_hour(3601), 7200);
    }
}
