//! Meshtastic radio bridge encoding (section 9, D26). The handful of protobuf
//! messages we need are hand-encoded from their public field numbers; no
//! GPL-licensed `.proto` files are used. The radio is an untrusted pipe:
//! payloads are already sealed 233-byte envelopes.
//!
//! ToRadio   { packet(1): MeshPacket }
//! MeshPacket{ to(2): fixed32, decoded(4): Data, id(6): fixed32, hop_limit(9): uint32 }
//! Data      { portnum(1): enum, payload(2): bytes }
//! FromRadio { id(1): uint32, packet(2): MeshPacket, ... }

pub const PRIVATE_APP_PORT: u64 = 256;
pub const BROADCAST: u32 = 0xFFFF_FFFF;
pub const MAX_PAYLOAD: usize = 233;

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn key(field: u32, wire: u8, out: &mut Vec<u8>) {
    varint(((field as u64) << 3) | wire as u64, out);
}

fn bytes_field(field: u32, b: &[u8], out: &mut Vec<u8>) {
    key(field, 2, out);
    varint(b.len() as u64, out);
    out.extend_from_slice(b);
}

/// Wrap an envelope as a ToRadio message for the radio's BLE ToRadio characteristic.
pub fn encode_to_radio(envelope: &[u8], packet_id: u32, hop_limit: u32) -> Option<Vec<u8>> {
    if envelope.is_empty() || envelope.len() > MAX_PAYLOAD {
        return None;
    }
    let mut data = Vec::new();
    key(1, 0, &mut data);
    varint(PRIVATE_APP_PORT, &mut data);
    bytes_field(2, envelope, &mut data);
    let mut pkt = Vec::new();
    key(2, 5, &mut pkt);
    pkt.extend_from_slice(&BROADCAST.to_le_bytes());
    bytes_field(4, &data, &mut pkt);
    key(6, 5, &mut pkt);
    pkt.extend_from_slice(&packet_id.to_le_bytes());
    key(9, 0, &mut pkt);
    varint(hop_limit.min(7) as u64, &mut pkt);
    let mut to = Vec::new();
    bytes_field(1, &pkt, &mut to);
    Some(to)
}

struct Fields<'a> {
    b: &'a [u8],
    i: usize,
}

enum Val<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed,
}

impl<'a> Fields<'a> {
    fn read_varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            v |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    fn next(&mut self) -> Option<Option<(u32, Val<'a>)>> {
        if self.i >= self.b.len() {
            return Some(None);
        }
        let k = self.read_varint()?;
        let field = (k >> 3) as u32;
        let v = match k & 7 {
            0 => Val::Varint(self.read_varint()?),
            1 => {
                self.i = self.i.checked_add(8).filter(|&e| e <= self.b.len())?;
                Val::Fixed
            }
            2 => {
                let n = usize::try_from(self.read_varint()?).ok()?;
                let end = self.i.checked_add(n).filter(|&e| e <= self.b.len())?;
                let s = &self.b[self.i..end];
                self.i = end;
                Val::Bytes(s)
            }
            5 => {
                self.i = self.i.checked_add(4).filter(|&e| e <= self.b.len())?;
                Val::Fixed
            }
            _ => return None,
        };
        Some(Some((field, v)))
    }
}

fn find_bytes(b: &[u8], field: u32) -> Option<&[u8]> {
    let mut f = Fields { b, i: 0 };
    let mut found = None;
    while let Some((n, v)) = f.next()? {
        if let (true, Val::Bytes(x)) = (n == field, v) {
            found = Some(x);
        }
    }
    found
}

fn find_varint(b: &[u8], field: u32) -> Option<u64> {
    let mut f = Fields { b, i: 0 };
    let mut found = None;
    while let Some((n, v)) = f.next()? {
        if let (true, Val::Varint(x)) = (n == field, v) {
            found = Some(x);
        }
    }
    found
}

/// Extract our envelope from a FromRadio message (PRIVATE_APP port only).
pub fn decode_from_radio(from_radio: &[u8]) -> Option<Vec<u8>> {
    let packet = find_bytes(from_radio, 2)?;
    let data = find_bytes(packet, 4)?;
    if find_varint(data, 1)? != PRIVATE_APP_PORT {
        return None;
    }
    let payload = find_bytes(data, 2)?;
    (payload.len() <= MAX_PAYLOAD).then(|| payload.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_from_radio() {
        let env = vec![7u8; 233];
        let to = encode_to_radio(&env, 0x01020304, 3).unwrap();
        // A FromRadio carrying the same MeshPacket under field 2, with an id.
        let packet = find_bytes(&to, 1).unwrap();
        let mut from = Vec::new();
        key(1, 0, &mut from);
        varint(99, &mut from);
        bytes_field(2, packet, &mut from);
        assert_eq!(decode_from_radio(&from), Some(env));
    }

    #[test]
    fn rejects_other_ports_and_garbage() {
        let mut data = Vec::new();
        key(1, 0, &mut data);
        varint(1, &mut data); // TEXT_MESSAGE_APP
        bytes_field(2, b"hi", &mut data);
        let mut pkt = Vec::new();
        bytes_field(4, &data, &mut pkt);
        let mut from = Vec::new();
        bytes_field(2, &pkt, &mut from);
        assert_eq!(decode_from_radio(&from), None);
        assert_eq!(decode_from_radio(&[0xFF, 0xFF, 0xFF]), None);
        assert_eq!(decode_from_radio(&[]), None);
        assert!(encode_to_radio(&[0; 234], 1, 3).is_none());
    }

    #[test]
    fn never_panics_on_noise() {
        let mut x = 0x1234_5678_u64;
        for len in 0..300 {
            let buf: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect();
            let _ = decode_from_radio(&buf);
        }
    }
}
