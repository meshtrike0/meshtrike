//! Inner plaintext (section 7): the payload after end-to-end decryption.

use crate::{Reader, WireError};
use mesh_constants::*;

pub mod kind {
    pub const TEXT: u8 = 0x01;
    pub const IMAGE_CHUNK: u8 = 0x02;
    pub const FILE_CHUNK: u8 = 0x03;
    /// Content key for a chunked image or file (amendment D42).
    pub const MEDIA_KEY: u8 = 0x04;
    /// Text answering an earlier message: its id and a short quote.
    pub const TEXT_REPLY: u8 = 0x05;
    /// A voice note part; the first carries its size and waveform.
    pub const VOICE_CHUNK: u8 = 0x06;
    pub const READ_RECEIPT: u8 = 0x10;
    pub const SESSION_INIT: u8 = 0x20;
    pub const ONE_TIME_KEYS: u8 = 0x21;
    pub const PRESENCE_KEY: u8 = 0x22;
    /// The sender's internet (Nym mixnet) address, or empty to withdraw it.
    pub const REACH: u8 = 0x23;
    /// Disguise bridges (share links) offered by a contact.
    pub const BRIDGES: u8 = 0x24;
    pub const GROUP_KEY_SHARE: u8 = 0x30;
    pub const GROUP_MEMBERSHIP: u8 = 0x31;
    pub const DELETE: u8 = 0x40;
    /// Nearby voice call signalling: offer, answer or end.
    pub const CALL_SIGNAL: u8 = 0x50;
}

/// What a call signal says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CallSignalKind {
    Offer = 0,
    Answer = 1,
    End = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerPlaintext {
    pub kind: u8,
    pub message_uuid: [u8; 16],
    pub sent_ms: u64,
    pub expiry_secs: u32,
    pub chunk_index: u16,
    pub chunk_count: u16,
    pub content: Vec<u8>,
}

impl InnerPlaintext {
    pub fn new(kind: u8, message_uuid: [u8; 16], sent_ms: u64, content: Vec<u8>) -> Self {
        Self {
            kind,
            message_uuid,
            sent_ms,
            expiry_secs: 0,
            chunk_index: 0,
            chunk_count: 1,
            content,
        }
    }

    pub fn unpadded_len(&self) -> usize {
        INNER_HEADER_LEN + self.content.len()
    }

    /// Encode and zero-pad to `bucket`.
    pub fn encode_padded(&self, bucket: usize) -> Result<Vec<u8>, WireError> {
        if self.unpadded_len() > bucket || self.content.len() > u16::MAX as usize {
            return Err(WireError::TooLarge);
        }
        let mut v = Vec::with_capacity(bucket);
        v.push(self.kind);
        v.extend_from_slice(&self.message_uuid);
        v.extend_from_slice(&self.sent_ms.to_be_bytes());
        v.extend_from_slice(&self.expiry_secs.to_be_bytes());
        v.extend_from_slice(&self.chunk_index.to_be_bytes());
        v.extend_from_slice(&self.chunk_count.to_be_bytes());
        v.extend_from_slice(&(self.content.len() as u16).to_be_bytes());
        v.extend_from_slice(&self.content);
        v.resize(bucket, 0);
        Ok(v)
    }

    /// Encode into the smallest standard bucket that fits.
    pub fn encode_auto(&self) -> Result<Vec<u8>, WireError> {
        let b = inner_bucket_for(self.unpadded_len()).ok_or(WireError::TooLarge)?;
        self.encode_padded(b)
    }

    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(buf);
        let kind = r.u8()?;
        let message_uuid = r.arr()?;
        let sent_ms = r.u64()?;
        let expiry_secs = r.u32()?;
        let chunk_index = r.u16()?;
        let chunk_count = r.u16()?;
        if chunk_count == 0 || chunk_index >= chunk_count {
            return Err(WireError::Field);
        }
        let len = r.u16()? as usize;
        let content = r.take(len)?.to_vec();
        if r.rest().iter().any(|&b| b != 0) {
            return Err(WireError::Field);
        }
        Ok(Self {
            kind,
            message_uuid,
            sent_ms,
            expiry_secs,
            chunk_index,
            chunk_count,
            content,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberKeys {
    pub ed25519: [u8; 32],
    pub curve25519: [u8; 32],
    pub noise_static: [u8; 32],
    pub binding_sig: [u8; 64],
    pub fallback_key: [u8; 32],
    pub fallback_sig: [u8; 64],
}

pub const MEMBER_KEYS_LEN: usize = 256;
/// Members per membership message so it fits the 4,096 bucket.
pub const MEMBERS_PER_MESSAGE: usize = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MembershipAction {
    Add = 0,
    Remove = 1,
    Leave = 2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// A text reply: the answered message's id and a quote snippet (shown
    /// even when the original is gone on the receiving phone).
    TextReply {
        reply_to: [u8; 16],
        quote: String,
        text: String,
    },
    ImageChunk(Vec<u8>),
    /// Part of a voice note (bare Opus packets, see mesh_media::voice). The
    /// first part carries the total size, the waveform bars and, for a voice
    /// reply, the answered message's id and a quote snippet.
    VoiceChunk {
        total_size: Option<u32>,
        waveform: Option<Vec<u8>>,
        reply: Option<([u8; 16], String)>,
        data: Vec<u8>,
    },
    FileChunk {
        name: Option<String>,
        total_size: Option<u32>,
        data: Vec<u8>,
    },
    ReadReceipt(Vec<[u8; 16]>),
    SessionInit,
    OneTimeKeys(Vec<[u8; 32]>),
    PresenceKey([u8; 32]),
    GroupKeyShare {
        group_id: [u8; 16],
        session_key: Vec<u8>,
        tag_key: [u8; 32],
        name: String,
    },
    GroupMembership {
        group_id: [u8; 16],
        action: MembershipAction,
        members: Vec<MemberKeys>,
    },
    Delete([u8; 16]),
    /// Internet (Nym) address; empty withdraws it.
    Reach(String),
    /// Bridge share links (at most `MAX_BRIDGES_IN_MESSAGE`).
    Bridges(Vec<String>),
    /// Nearby call signalling: the call, the sender's one-time X25519 key
    /// (offer and answer) and, for an end, the reason.
    CallSignal {
        kind: CallSignalKind,
        call_id: [u8; 8],
        eph: [u8; 32],
        reason: u8,
    },
    /// Key, SHA-256 of the whole content, and chunk count (D42).
    MediaKey {
        key: [u8; 32],
        hash: [u8; 32],
        count: u16,
    },
}

fn utf8(b: &[u8]) -> Result<String, WireError> {
    String::from_utf8(b.to_vec()).map_err(|_| WireError::Field)
}

impl Content {
    pub fn kind(&self) -> u8 {
        match self {
            Content::Text(_) => kind::TEXT,
            Content::TextReply { .. } => kind::TEXT_REPLY,
            Content::ImageChunk(_) => kind::IMAGE_CHUNK,
            Content::VoiceChunk { .. } => kind::VOICE_CHUNK,
            Content::FileChunk { .. } => kind::FILE_CHUNK,
            Content::ReadReceipt(_) => kind::READ_RECEIPT,
            Content::SessionInit => kind::SESSION_INIT,
            Content::OneTimeKeys(_) => kind::ONE_TIME_KEYS,
            Content::PresenceKey(_) => kind::PRESENCE_KEY,
            Content::GroupKeyShare { .. } => kind::GROUP_KEY_SHARE,
            Content::GroupMembership { .. } => kind::GROUP_MEMBERSHIP,
            Content::Delete(_) => kind::DELETE,
            Content::MediaKey { .. } => kind::MEDIA_KEY,
            Content::Reach(_) => kind::REACH,
            Content::Bridges(_) => kind::BRIDGES,
            Content::CallSignal { .. } => kind::CALL_SIGNAL,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut v = Vec::new();
        match self {
            Content::Text(t) => {
                if t.len() > MAX_TEXT_BYTES {
                    return Err(WireError::TooLarge);
                }
                v.extend_from_slice(t.as_bytes());
            }
            Content::TextReply {
                reply_to,
                quote,
                text,
            } => {
                if text.len() > MAX_TEXT_BYTES || quote.len() > MAX_QUOTE_BYTES {
                    return Err(WireError::TooLarge);
                }
                v.extend_from_slice(reply_to);
                v.push(quote.len() as u8);
                v.extend_from_slice(quote.as_bytes());
                v.extend_from_slice(text.as_bytes());
            }
            Content::ImageChunk(d) => {
                if d.len() > IMAGE_CHUNK_BYTES {
                    return Err(WireError::TooLarge);
                }
                v.extend_from_slice(d);
            }
            Content::VoiceChunk {
                total_size,
                waveform,
                reply,
                data,
            } => {
                if let (Some(s), Some(w)) = (total_size, waveform) {
                    if w.len() > MAX_WAVEFORM_BARS {
                        return Err(WireError::TooLarge);
                    }
                    v.extend_from_slice(&s.to_be_bytes());
                    v.push(w.len() as u8);
                    v.extend_from_slice(w);
                    match reply {
                        None => v.push(0),
                        Some((to, quote)) => {
                            if quote.len() > MAX_QUOTE_BYTES {
                                return Err(WireError::TooLarge);
                            }
                            v.push(1);
                            v.extend_from_slice(to);
                            v.push(quote.len() as u8);
                            v.extend_from_slice(quote.as_bytes());
                        }
                    }
                } else if reply.is_some() {
                    return Err(WireError::Field);
                }
                v.extend_from_slice(data);
                if v.len() > IMAGE_CHUNK_BYTES {
                    return Err(WireError::TooLarge);
                }
            }
            Content::FileChunk {
                name,
                total_size,
                data,
            } => {
                if let (Some(n), Some(s)) = (name, total_size) {
                    if n.len() > MAX_FILE_NAME_BYTES {
                        return Err(WireError::TooLarge);
                    }
                    v.push(n.len() as u8);
                    v.extend_from_slice(n.as_bytes());
                    v.extend_from_slice(&s.to_be_bytes());
                }
                v.extend_from_slice(data);
                if v.len() > IMAGE_CHUNK_BYTES {
                    return Err(WireError::TooLarge);
                }
            }
            Content::ReadReceipt(ids) => {
                if ids.len() > MAX_READ_RECEIPT_IDS {
                    return Err(WireError::TooLarge);
                }
                v.push(ids.len() as u8);
                ids.iter().for_each(|i| v.extend_from_slice(i));
            }
            Content::SessionInit => {}
            Content::OneTimeKeys(keys) => {
                if keys.len() > MAX_OTK_IN_MESSAGE {
                    return Err(WireError::TooLarge);
                }
                v.push(keys.len() as u8);
                keys.iter().for_each(|k| v.extend_from_slice(k));
            }
            Content::PresenceKey(k) => v.extend_from_slice(k),
            Content::GroupKeyShare {
                group_id,
                session_key,
                tag_key,
                name,
            } => {
                if name.len() > MAX_GROUP_NAME_BYTES {
                    return Err(WireError::TooLarge);
                }
                v.extend_from_slice(group_id);
                v.extend_from_slice(&(session_key.len() as u16).to_be_bytes());
                v.extend_from_slice(session_key);
                v.extend_from_slice(tag_key);
                v.push(name.len() as u8);
                v.extend_from_slice(name.as_bytes());
            }
            Content::GroupMembership {
                group_id,
                action,
                members,
            } => {
                if members.len() > MEMBERS_PER_MESSAGE {
                    return Err(WireError::TooLarge);
                }
                v.extend_from_slice(group_id);
                v.push(*action as u8);
                v.push(members.len() as u8);
                for m in members {
                    for part in [
                        &m.ed25519[..],
                        &m.curve25519,
                        &m.noise_static,
                        &m.binding_sig,
                        &m.fallback_key,
                        &m.fallback_sig,
                    ] {
                        v.extend_from_slice(part);
                    }
                }
            }
            Content::Delete(id) => v.extend_from_slice(id),
            Content::MediaKey { key, hash, count } => {
                v.extend_from_slice(key);
                v.extend_from_slice(hash);
                v.extend_from_slice(&count.to_be_bytes());
            }
            Content::Reach(a) => {
                if a.len() > MAX_REACH_BYTES {
                    return Err(WireError::TooLarge);
                }
                v.extend_from_slice(a.as_bytes());
            }
            Content::CallSignal {
                kind,
                call_id,
                eph,
                reason,
            } => {
                v.push(*kind as u8);
                v.extend_from_slice(call_id);
                v.extend_from_slice(eph);
                v.push(*reason);
            }
            Content::Bridges(links) => {
                if links.len() > MAX_BRIDGES_IN_MESSAGE {
                    return Err(WireError::TooLarge);
                }
                v.push(links.len() as u8);
                for l in links {
                    if l.len() > MAX_BRIDGE_LINK_BYTES {
                        return Err(WireError::TooLarge);
                    }
                    v.extend_from_slice(&(l.len() as u16).to_be_bytes());
                    v.extend_from_slice(l.as_bytes());
                }
            }
        }
        Ok(v)
    }

    /// Parse content. `first_chunk` tells file chunks whether a header is present.
    /// Unknown kinds return `Ok(None)` (dropped silently, section 7).
    pub fn decode(kind_byte: u8, b: &[u8], first_chunk: bool) -> Result<Option<Self>, WireError> {
        let mut r = Reader::new(b);
        let c = match kind_byte {
            kind::TEXT => {
                if b.len() > MAX_TEXT_BYTES {
                    return Err(WireError::TooLarge);
                }
                Content::Text(utf8(b)?)
            }
            kind::TEXT_REPLY => {
                let reply_to = r.arr()?;
                let q = r.u8()? as usize;
                if q > MAX_QUOTE_BYTES {
                    return Err(WireError::Field);
                }
                let quote = utf8(r.take(q)?)?;
                let rest = r.rest();
                if rest.len() > MAX_TEXT_BYTES {
                    return Err(WireError::TooLarge);
                }
                Content::TextReply {
                    reply_to,
                    quote,
                    text: utf8(rest)?,
                }
            }
            kind::IMAGE_CHUNK => {
                if b.len() > IMAGE_CHUNK_BYTES {
                    return Err(WireError::TooLarge);
                }
                Content::ImageChunk(b.to_vec())
            }
            kind::VOICE_CHUNK => {
                if b.len() > IMAGE_CHUNK_BYTES {
                    return Err(WireError::TooLarge);
                }
                if first_chunk {
                    let size = r.u32()?;
                    let n = r.u8()? as usize;
                    if n > MAX_WAVEFORM_BARS {
                        return Err(WireError::Field);
                    }
                    let waveform = r.take(n)?.to_vec();
                    let reply = match r.u8()? {
                        0 => None,
                        1 => {
                            let to: [u8; 16] = r.arr()?;
                            let q = r.u8()? as usize;
                            if q > MAX_QUOTE_BYTES {
                                return Err(WireError::Field);
                            }
                            Some((to, utf8(r.take(q)?)?))
                        }
                        _ => return Err(WireError::Field),
                    };
                    Content::VoiceChunk {
                        total_size: Some(size),
                        waveform: Some(waveform),
                        reply,
                        data: r.rest().to_vec(),
                    }
                } else {
                    Content::VoiceChunk {
                        total_size: None,
                        waveform: None,
                        reply: None,
                        data: b.to_vec(),
                    }
                }
            }
            kind::FILE_CHUNK => {
                if first_chunk {
                    let n = r.u8()? as usize;
                    if n > MAX_FILE_NAME_BYTES {
                        return Err(WireError::Field);
                    }
                    let name = utf8(r.take(n)?)?;
                    let size = r.u32()?;
                    Content::FileChunk {
                        name: Some(name),
                        total_size: Some(size),
                        data: r.rest().to_vec(),
                    }
                } else {
                    Content::FileChunk {
                        name: None,
                        total_size: None,
                        data: b.to_vec(),
                    }
                }
            }
            kind::READ_RECEIPT => {
                let n = r.u8()? as usize;
                if n > MAX_READ_RECEIPT_IDS {
                    return Err(WireError::Field);
                }
                Content::ReadReceipt((0..n).map(|_| r.arr()).collect::<Result<_, _>>()?)
            }
            kind::SESSION_INIT => Content::SessionInit,
            kind::ONE_TIME_KEYS => {
                let n = r.u8()? as usize;
                if n > MAX_OTK_IN_MESSAGE {
                    return Err(WireError::Field);
                }
                Content::OneTimeKeys((0..n).map(|_| r.arr()).collect::<Result<_, _>>()?)
            }
            kind::PRESENCE_KEY => Content::PresenceKey(r.arr()?),
            kind::GROUP_KEY_SHARE => {
                let group_id = r.arr()?;
                let n = r.u16()? as usize;
                let session_key = r.take(n)?.to_vec();
                let tag_key = r.arr()?;
                let nl = r.u8()? as usize;
                if nl > MAX_GROUP_NAME_BYTES {
                    return Err(WireError::Field);
                }
                let name = utf8(r.take(nl)?)?;
                Content::GroupKeyShare {
                    group_id,
                    session_key,
                    tag_key,
                    name,
                }
            }
            kind::GROUP_MEMBERSHIP => {
                let group_id = r.arr()?;
                let action = match r.u8()? {
                    0 => MembershipAction::Add,
                    1 => MembershipAction::Remove,
                    2 => MembershipAction::Leave,
                    _ => return Err(WireError::Field),
                };
                let n = r.u8()? as usize;
                if n > MEMBERS_PER_MESSAGE {
                    return Err(WireError::Field);
                }
                let mut members = Vec::with_capacity(n);
                for _ in 0..n {
                    members.push(MemberKeys {
                        ed25519: r.arr()?,
                        curve25519: r.arr()?,
                        noise_static: r.arr()?,
                        binding_sig: r.arr()?,
                        fallback_key: r.arr()?,
                        fallback_sig: r.arr()?,
                    });
                }
                Content::GroupMembership {
                    group_id,
                    action,
                    members,
                }
            }
            kind::DELETE => Content::Delete(r.arr()?),
            kind::CALL_SIGNAL => {
                let kind = match r.u8()? {
                    0 => CallSignalKind::Offer,
                    1 => CallSignalKind::Answer,
                    2 => CallSignalKind::End,
                    _ => return Err(WireError::Field),
                };
                Content::CallSignal {
                    kind,
                    call_id: r.arr()?,
                    eph: r.arr()?,
                    reason: r.u8()?,
                }
            }
            kind::MEDIA_KEY => Content::MediaKey {
                key: r.arr()?,
                hash: r.arr()?,
                count: r.u16()?,
            },
            kind::REACH => {
                if b.len() > MAX_REACH_BYTES {
                    return Err(WireError::TooLarge);
                }
                Content::Reach(utf8(b)?)
            }
            kind::BRIDGES => {
                let n = r.u8()? as usize;
                if n > MAX_BRIDGES_IN_MESSAGE {
                    return Err(WireError::TooLarge);
                }
                let mut links = Vec::with_capacity(n);
                for _ in 0..n {
                    let len = r.u16()? as usize;
                    if len > MAX_BRIDGE_LINK_BYTES {
                        return Err(WireError::TooLarge);
                    }
                    links.push(utf8(r.take(len)?)?);
                }
                Content::Bridges(links)
            }
            _ => return Ok(None),
        };
        Ok(Some(c))
    }
}

/// Split a byte blob into content chunks of at most `IMAGE_CHUNK_BYTES`;
/// for files the first chunk also carries the name header.
pub fn chunk_image(data: &[u8]) -> Vec<Content> {
    data.chunks(IMAGE_CHUNK_BYTES)
        .map(|c| Content::ImageChunk(c.to_vec()))
        .collect()
}

/// A voice note's parts: the first carries the total size, the waveform and
/// what it replies to (if anything).
pub fn chunk_voice(waveform: &[u8], reply: Option<([u8; 16], String)>, data: &[u8]) -> Vec<Content> {
    let waveform = &waveform[..waveform.len().min(MAX_WAVEFORM_BARS)];
    let reply = reply.filter(|(_, q)| q.len() <= MAX_QUOTE_BYTES);
    let header = 4 + 1 + waveform.len() + 1 + reply.as_ref().map_or(0, |(_, q)| 16 + 1 + q.len());
    let first = (IMAGE_CHUNK_BYTES - header).min(data.len());
    let mut out = vec![Content::VoiceChunk {
        total_size: Some(data.len() as u32),
        waveform: Some(waveform.to_vec()),
        reply,
        data: data[..first].to_vec(),
    }];
    for c in data[first..].chunks(IMAGE_CHUNK_BYTES) {
        out.push(Content::VoiceChunk {
            total_size: None,
            waveform: None,
            reply: None,
            data: c.to_vec(),
        });
    }
    out
}

pub fn chunk_file(name: &str, data: &[u8]) -> Vec<Content> {
    let header = 1 + name.len() + 4;
    let first = (IMAGE_CHUNK_BYTES - header).min(data.len());
    let mut out = vec![Content::FileChunk {
        name: Some(name.to_string()),
        total_size: Some(data.len() as u32),
        data: data[..first].to_vec(),
    }];
    for c in data[first..].chunks(IMAGE_CHUNK_BYTES) {
        out.push(Content::FileChunk {
            name: None,
            total_size: None,
            data: c.to_vec(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reach_and_bridges_roundtrip() {
        for c in [
            Content::Reach("abc.def@ghi".into()),
            Content::Reach(String::new()),
            Content::Bridges(vec![
                "vless://a@1.2.3.4:443?security=reality".into(),
                "vless://b@h:8443".into(),
            ]),
        ] {
            let enc = c.encode().unwrap();
            assert_eq!(Content::decode(c.kind(), &enc, true).unwrap(), Some(c));
        }
        assert!(Content::Reach("x".repeat(MAX_REACH_BYTES + 1))
            .encode()
            .is_err());
        assert!(
            Content::Bridges(vec![String::new(); MAX_BRIDGES_IN_MESSAGE + 1])
                .encode()
                .is_err()
        );
    }

    #[test]
    fn call_signal_roundtrip() {
        for kind in [CallSignalKind::Offer, CallSignalKind::Answer, CallSignalKind::End] {
            let c = Content::CallSignal { kind, call_id: [3; 8], eph: [4; 32], reason: 2 };
            let enc = c.encode().unwrap();
            assert_eq!(enc.len(), 42);
            assert_eq!(Content::decode(c.kind(), &enc, true).unwrap(), Some(c));
        }
        let mut bad = vec![9u8];
        bad.extend([0u8; 41]);
        assert!(Content::decode(kind::CALL_SIGNAL, &bad, true).is_err());
    }

    #[test]
    fn voice_chunks_roundtrip() {
        let data: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
        let wf: Vec<u8> = (0..64u8).collect();
        let parts = chunk_voice(&wf, None, &data);
        assert!(parts.len() > 2);
        let mut joined = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            let enc = p.encode().unwrap();
            assert!(enc.len() <= IMAGE_CHUNK_BYTES);
            let d = Content::decode(p.kind(), &enc, i == 0).unwrap().unwrap();
            assert_eq!(&d, p);
            if let Content::VoiceChunk { waveform, total_size, data, .. } = d {
                if i == 0 {
                    assert_eq!(waveform.as_deref(), Some(&wf[..]));
                    assert_eq!(total_size, Some(10_000));
                }
                joined.extend(data);
            }
        }
        assert_eq!(joined, data);
    }

    #[test]
    fn voice_reply_rides_the_first_part() {
        let data: Vec<u8> = (0..9_000u32).map(|i| (i * 7) as u8).collect();
        let quote = "é".repeat(MAX_QUOTE_BYTES / 2);
        let parts = chunk_voice(&[9; 64], Some(([5; 16], quote.clone())), &data);
        let mut joined = Vec::new();
        for (i, p) in parts.iter().enumerate() {
            let enc = p.encode().unwrap();
            assert!(enc.len() <= IMAGE_CHUNK_BYTES);
            let d = Content::decode(p.kind(), &enc, i == 0).unwrap().unwrap();
            assert_eq!(&d, p);
            if let Content::VoiceChunk { reply, data, .. } = d {
                assert_eq!(reply, (i == 0).then(|| ([5; 16], quote.clone())));
                joined.extend(data);
            }
        }
        assert_eq!(joined, data);
        // A reply on a later part, or an unknown flag, is refused.
        let bad = Content::VoiceChunk { total_size: None, waveform: None, reply: Some(([1; 16], "x".into())), data: vec![1] };
        assert!(bad.encode().is_err());
        let mut enc = parts[0].encode().unwrap();
        enc[4 + 1 + 64] = 2;
        assert!(Content::decode(parts[0].kind(), &enc, true).is_err());
    }

    #[test]
    fn text_reply_roundtrip_and_limits() {
        let c = Content::TextReply {
            reply_to: [7; 16],
            quote: "Are you coming to the square?".into(),
            text: "Yes, at five".into(),
        };
        let enc = c.encode().unwrap();
        assert_eq!(Content::decode(c.kind(), &enc, true).unwrap(), Some(c));
        // Longest reply with the longest quote still fits the largest bucket.
        let big = Content::TextReply {
            reply_to: [1; 16],
            quote: "q".repeat(MAX_QUOTE_BYTES),
            text: "t".repeat(MAX_TEXT_BYTES),
        };
        let ip = InnerPlaintext::new(big.kind(), [2; 16], 1, big.encode().unwrap());
        assert_eq!(ip.encode_auto().unwrap().len(), 4096);
        assert!(Content::TextReply {
            reply_to: [0; 16],
            quote: "q".repeat(MAX_QUOTE_BYTES + 1),
            text: "t".into(),
        }
        .encode()
        .is_err());
        // A quote length beyond the limit is rejected when parsing.
        let mut bad = vec![0u8; 16];
        bad.push((MAX_QUOTE_BYTES + 1) as u8);
        bad.extend(std::iter::repeat_n(b'q', MAX_QUOTE_BYTES + 1));
        assert!(Content::decode(kind::TEXT_REPLY, &bad, true).is_err());
    }

    #[test]
    fn inner_roundtrip_and_padding() {
        let c = Content::Text("hello".into());
        let ip = InnerPlaintext::new(c.kind(), [9; 16], 123, c.encode().unwrap());
        let enc = ip.encode_auto().unwrap();
        assert_eq!(enc.len(), 256);
        let d = InnerPlaintext::decode(&enc).unwrap();
        assert_eq!(d, ip);
        assert_eq!(Content::decode(d.kind, &d.content, true).unwrap(), Some(c));
        assert_eq!(Content::decode(0x7F, b"x", true).unwrap(), None);
    }

    #[test]
    fn max_text_fits_4096() {
        let c = Content::Text("a".repeat(MAX_TEXT_BYTES));
        let ip = InnerPlaintext::new(c.kind(), [0; 16], 0, c.encode().unwrap());
        assert_eq!(ip.encode_auto().unwrap().len(), 4096);
        assert!(Content::Text("a".repeat(MAX_TEXT_BYTES + 1))
            .encode()
            .is_err());
    }

    #[test]
    fn membership_max_fits_4096() {
        let m = MemberKeys {
            ed25519: [1; 32],
            curve25519: [2; 32],
            noise_static: [3; 32],
            binding_sig: [4; 64],
            fallback_key: [5; 32],
            fallback_sig: [6; 64],
        };
        let c = Content::GroupMembership {
            group_id: [0; 16],
            action: MembershipAction::Add,
            members: vec![m; MEMBERS_PER_MESSAGE],
        };
        let enc = c.encode().unwrap();
        assert!(INNER_HEADER_LEN + enc.len() <= 4096);
        assert_eq!(
            Content::decode(kind::GROUP_MEMBERSHIP, &enc, true).unwrap(),
            Some(c)
        );
    }

    #[test]
    fn file_chunking() {
        let data = vec![7u8; 10_000];
        let chunks = chunk_file("a.pdf", &data);
        let mut joined = vec![];
        for (i, c) in chunks.iter().enumerate() {
            let enc = c.encode().unwrap();
            assert!(enc.len() <= IMAGE_CHUNK_BYTES);
            if let Some(Content::FileChunk { data, .. }) =
                Content::decode(kind::FILE_CHUNK, &enc, i == 0).unwrap()
            {
                joined.extend(data);
            }
        }
        assert_eq!(joined, data);
    }
}
