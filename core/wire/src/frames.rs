//! Link frames (A.6): the only fragmentation layer. Each frame is one Noise
//! transport message on BLE / Wi-Fi.

use crate::{MessageId, Reader, Tag, WireError};
use mesh_constants::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameType {
    Envelope = 0x01,
    Padding = 0x02,
    Summary = 0x03,
    Request = 0x04,
    Beacon = 0x05,
    /// Ask to open a Wi-Fi peer-to-peer link (section 9).
    WifiRequest = 0x06,
    /// Agree to open it.
    WifiAccept = 0x07,
    /// Live call audio, sealed end to end with the call's own keys.
    CallMedia = 0x08,
}

impl FrameType {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x01 => Self::Envelope,
            0x02 => Self::Padding,
            0x03 => Self::Summary,
            0x04 => Self::Request,
            0x05 => Self::Beacon,
            0x06 => Self::WifiRequest,
            0x07 => Self::WifiAccept,
            0x08 => Self::CallMedia,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFrame {
    pub ftype: FrameType,
    pub stream: u16,
    pub index: u8,
    pub count: u8,
    pub data: Vec<u8>,
}

impl LinkFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(LINK_FRAME_HEADER_LEN + self.data.len());
        v.push(self.ftype as u8);
        v.extend_from_slice(&self.stream.to_be_bytes());
        v.push(self.index);
        v.push(self.count);
        v.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
        v.extend_from_slice(&self.data);
        v
    }

    /// Fixed-size encoding for High-Risk mode: zero padding after the data.
    pub fn encode_fixed(&self, total: usize) -> Vec<u8> {
        let mut v = self.encode();
        v.resize(total.max(v.len()), 0);
        v
    }

    pub fn decode(buf: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(buf);
        let ftype = FrameType::from_u8(r.u8()?).ok_or(WireError::Type)?;
        let stream = r.u16()?;
        let index = r.u8()?;
        let count = r.u8()?;
        if count == 0 || index >= count {
            return Err(WireError::Field);
        }
        let len = r.u16()? as usize;
        let data = r.take(len)?.to_vec();
        Ok(Self {
            ftype,
            stream,
            index,
            count,
            data,
        })
    }
}

/// Frame data capacity for a given on-air frame size (Noise message size).
pub fn frame_capacity(noise_msg_len: usize) -> usize {
    noise_msg_len.saturating_sub(NOISE_TAG_LEN + LINK_FRAME_HEADER_LEN)
}

/// Standard-mode Noise message size for an ATT MTU.
/// ATT payload is MTU − 3, and an attribute value is at most 512 bytes
/// (Bluetooth Core spec; Android rejects longer notifications).
pub fn noise_msg_len_for_mtu(mtu: u16) -> usize {
    (mtu as usize).saturating_sub(3).min(512)
}

pub fn fragment(
    ftype: FrameType,
    stream: u16,
    data: &[u8],
    capacity: usize,
) -> Result<Vec<LinkFrame>, WireError> {
    if capacity == 0 {
        return Err(WireError::Length);
    }
    let parts: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(capacity).collect()
    };
    if parts.len() > 255 {
        return Err(WireError::TooLarge);
    }
    let count = parts.len() as u8;
    Ok(parts
        .into_iter()
        .enumerate()
        .map(|(i, p)| LinkFrame {
            ftype,
            stream,
            index: i as u8,
            count,
            data: p.to_vec(),
        })
        .collect())
}

struct Partial {
    ftype: FrameType,
    parts: Vec<Option<Vec<u8>>>,
    have: usize,
    started_ms: u64,
}

/// Per-connection reassembly with the 60-second timeout.
#[derive(Default)]
pub struct Reassembler {
    streams: HashMap<u16, Partial>,
}

impl Reassembler {
    pub fn push(
        &mut self,
        f: LinkFrame,
        now_ms: u64,
    ) -> Result<Option<(FrameType, Vec<u8>)>, WireError> {
        self.expire(now_ms);
        if f.ftype == FrameType::Padding {
            return Ok(None);
        }
        if f.count == 1 {
            return Ok(Some((f.ftype, f.data)));
        }
        let p = self.streams.entry(f.stream).or_insert_with(|| Partial {
            ftype: f.ftype,
            parts: vec![None; f.count as usize],
            have: 0,
            started_ms: now_ms,
        });
        if p.parts.len() != f.count as usize || p.ftype != f.ftype {
            self.streams.remove(&f.stream);
            return Err(WireError::Field);
        }
        let slot = &mut p.parts[f.index as usize];
        if slot.is_none() {
            *slot = Some(f.data);
            p.have += 1;
        }
        if p.have == p.parts.len() {
            let p = self.streams.remove(&f.stream).unwrap();
            let data = p.parts.into_iter().flatten().flatten().collect();
            return Ok(Some((p.ftype, data)));
        }
        Ok(None)
    }

    pub fn expire(&mut self, now_ms: u64) {
        self.streams
            .retain(|_, p| now_ms.saturating_sub(p.started_ms) < FRAGMENT_TIMEOUT_MS);
    }
}

// ===== Summary vector and envelope request (A.6) =====

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryEntry {
    pub message_id: MessageId,
    pub tag: Tag,
    pub expiry: u32,
}

pub fn encode_summary(entries: &[SummaryEntry]) -> Vec<u8> {
    let n = entries.len().min(SUMMARY_VECTOR_MAX);
    let mut v = Vec::with_capacity(2 + n * 36);
    v.extend_from_slice(&(n as u16).to_be_bytes());
    for e in &entries[..n] {
        v.extend_from_slice(&e.message_id);
        v.extend_from_slice(&e.tag);
        v.extend_from_slice(&e.expiry.to_be_bytes());
    }
    v
}

pub fn decode_summary(buf: &[u8]) -> Result<Vec<SummaryEntry>, WireError> {
    let mut r = Reader::new(buf);
    let n = r.u16()? as usize;
    if n > SUMMARY_VECTOR_MAX {
        return Err(WireError::TooLarge);
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(SummaryEntry {
            message_id: r.arr()?,
            tag: r.arr()?,
            expiry: r.u32()?,
        });
    }
    Ok(out)
}

pub fn encode_request(ids: &[MessageId]) -> Vec<u8> {
    let n = ids.len().min(SUMMARY_VECTOR_MAX);
    let mut v = Vec::with_capacity(2 + n * 16);
    v.extend_from_slice(&(n as u16).to_be_bytes());
    ids[..n].iter().for_each(|i| v.extend_from_slice(i));
    v
}

pub fn decode_request(buf: &[u8]) -> Result<Vec<MessageId>, WireError> {
    let mut r = Reader::new(buf);
    let n = r.u16()? as usize;
    if n > SUMMARY_VECTOR_MAX {
        return Err(WireError::TooLarge);
    }
    (0..n).map(|_| r.arr()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_reassemble_out_of_order() {
        let data: Vec<u8> = (0..4354u32).map(|i| i as u8).collect();
        let mut frames = fragment(FrameType::Envelope, 7, &data, frame_capacity(509)).unwrap();
        assert_eq!(frames.len(), 9);
        frames.reverse();
        let mut r = Reassembler::default();
        let mut out = None;
        for f in frames {
            let f = LinkFrame::decode(&f.encode()).unwrap();
            if let Some(x) = r.push(f, 0).unwrap() {
                out = Some(x);
            }
        }
        assert_eq!(out, Some((FrameType::Envelope, data)));
    }

    #[test]
    fn reassembly_times_out() {
        let frames = fragment(FrameType::Envelope, 1, &[0u8; 1000], 100).unwrap();
        let mut r = Reassembler::default();
        r.push(frames[0].clone(), 0).unwrap();
        for f in &frames[1..] {
            assert_eq!(r.push(f.clone(), FRAGMENT_TIMEOUT_MS + 1).unwrap(), None);
        }
    }

    #[test]
    fn high_risk_frame_size() {
        let cap = frame_capacity(COVER_FRAME_LEN);
        assert_eq!(cap, 157);
        let f = fragment(FrameType::Envelope, 1, &[1u8; 514], cap).unwrap();
        assert_eq!(f.len(), 4);
        assert!(f
            .iter()
            .all(|x| x.encode_fixed(COVER_FRAME_LEN - NOISE_TAG_LEN).len() == 164));
    }

    #[test]
    fn summary_roundtrip() {
        let e = vec![
            SummaryEntry {
                message_id: [1; 16],
                tag: [2; 16],
                expiry: 3600
            };
            3
        ];
        assert_eq!(decode_summary(&encode_summary(&e)).unwrap(), e);
        let ids = vec![[4u8; 16]; 5];
        assert_eq!(decode_request(&encode_request(&ids)).unwrap(), ids);
    }
}
