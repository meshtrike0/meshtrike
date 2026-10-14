//! Voice notes: Opus audio, carried as bare packets and rebuilt for playback.
//!
//! - The sender's recorder writes an Ogg Opus file. [`VoiceClip::from_ogg`]
//!   keeps only the Opus audio packets (and the channel count and pre-skip),
//!   dropping the container and its comment header ("vendor", tags), so
//!   nothing but the audio itself leaves the phone.
//! - On the wire a clip is a small framed list of packets ([`VoiceClip::to_blob`]).
//! - The receiver validates every packet's table-of-contents byte and the
//!   total length ([`VoiceClip::from_blob`]), then builds a fresh, minimal Ogg
//!   file ([`VoiceClip::to_ogg`]) for the platform player. Bytes from the
//!   other phone never reach a container parser outside this module.

use crate::MediaError;

/// Longest voice note, in milliseconds (5 minutes), plus encoder slack.
pub const MAX_VOICE_MS: u32 = 300_000;
const SLACK_MS: u32 = 2_000;
/// Bars in a voice note's waveform (0–255 each), drawn in the bubble.
pub const WAVEFORM_BARS: usize = 64;
/// Largest Opus packet accepted (RFC 6716 allows up to 1275 per frame).
const MAX_PACKET: usize = 1_500;
/// Upper bound on packets (2.5 ms frames for five minutes).
const MAX_PACKETS: usize = 120_000;
const MAGIC: &[u8; 4] = b"MTV1";
/// Opus always runs at 48 kHz for timing.
const RATE: u32 = 48_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceClip {
    pub channels: u8,
    pub pre_skip: u16,
    pub packets: Vec<Vec<u8>>,
}

/// Samples (at 48 kHz) one Opus packet holds, from its TOC byte (RFC 6716
/// section 3.1); None if the packet is not well formed.
pub fn packet_samples(p: &[u8]) -> Option<u32> {
    let toc = *p.first()?;
    if p.len() > MAX_PACKET {
        return None;
    }
    let config = toc >> 3;
    let frame: u32 = match config {
        0..=11 => [480, 960, 1920, 2880][(config % 4) as usize],
        12..=15 => [480, 960][((config - 12) % 2) as usize],
        _ => [120, 240, 480, 960][((config - 16) % 4) as usize],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 => {
            // Two frames of equal size.
            if !(p.len() - 1).is_multiple_of(2) {
                return None;
            }
            2
        }
        2 => {
            if p.len() < 2 {
                return None;
            }
            2
        }
        _ => {
            let n = (*p.get(1)? & 0x3F) as u32;
            if n == 0 {
                return None;
            }
            n
        }
    };
    let total = frame * frames;
    // A packet carries at most 120 ms.
    (total <= 5_760).then_some(total)
}

impl VoiceClip {
    /// Length in milliseconds (pre-skip excluded).
    pub fn duration_ms(&self) -> u32 {
        let samples: u64 = self
            .packets
            .iter()
            .map(|p| packet_samples(p).unwrap_or(0) as u64)
            .sum();
        let samples = samples.saturating_sub(self.pre_skip as u64);
        (samples * 1000 / RATE as u64) as u32
    }

    fn validate(&self) -> Result<(), MediaError> {
        if !(1..=2).contains(&self.channels) || self.packets.is_empty() || self.packets.len() > MAX_PACKETS {
            return Err(MediaError::Malformed);
        }
        if self.packets.iter().any(|p| packet_samples(p).is_none()) {
            return Err(MediaError::Malformed);
        }
        if self.duration_ms() > MAX_VOICE_MS + SLACK_MS {
            return Err(MediaError::TooLarge);
        }
        Ok(())
    }

    /// The audio of an Ogg Opus recording, without its container or tags.
    pub fn from_ogg(bytes: &[u8]) -> Result<Self, MediaError> {
        let packets = ogg_packets(bytes)?;
        let mut it = packets.into_iter();
        let head = it.next().ok_or(MediaError::Malformed)?;
        if head.len() < 19 || &head[..8] != b"OpusHead" || head[8] >> 4 != 0 {
            return Err(MediaError::Malformed);
        }
        let channels = head[9];
        let pre_skip = u16::from_le_bytes([head[10], head[11]]);
        // Mapping family 0 only (mono or stereo, no extra table).
        if head[18] != 0 {
            return Err(MediaError::Malformed);
        }
        let tags = it.next().ok_or(MediaError::Malformed)?;
        if tags.len() < 8 || &tags[..8] != b"OpusTags" {
            return Err(MediaError::Malformed);
        }
        let clip = VoiceClip {
            channels,
            pre_skip,
            packets: it.filter(|p| !p.is_empty()).collect(),
        };
        clip.validate()?;
        Ok(clip)
    }

    /// Wire form: "MTV1" | channels u8 | pre_skip u16 | count u32 | (len u16 | packet)*.
    pub fn to_blob(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(11 + self.packets.iter().map(|p| p.len() + 2).sum::<usize>());
        v.extend_from_slice(MAGIC);
        v.push(self.channels);
        v.extend_from_slice(&self.pre_skip.to_be_bytes());
        v.extend_from_slice(&(self.packets.len() as u32).to_be_bytes());
        for p in &self.packets {
            v.extend_from_slice(&(p.len() as u16).to_be_bytes());
            v.extend_from_slice(p);
        }
        v
    }

    pub fn from_blob(b: &[u8]) -> Result<Self, MediaError> {
        if b.len() < 11 || &b[..4] != MAGIC {
            return Err(MediaError::Malformed);
        }
        let channels = b[4];
        let pre_skip = u16::from_be_bytes([b[5], b[6]]);
        let count = u32::from_be_bytes([b[7], b[8], b[9], b[10]]) as usize;
        if count > MAX_PACKETS {
            return Err(MediaError::TooLarge);
        }
        let mut pos = 11;
        let mut packets = Vec::with_capacity(count);
        for _ in 0..count {
            let len = u16::from_be_bytes([*b.get(pos).ok_or(MediaError::Malformed)?, *b.get(pos + 1).ok_or(MediaError::Malformed)?]) as usize;
            pos += 2;
            let p = b.get(pos..pos + len).ok_or(MediaError::Malformed)?;
            packets.push(p.to_vec());
            pos += len;
        }
        if pos != b.len() {
            return Err(MediaError::Malformed);
        }
        let clip = VoiceClip { channels, pre_skip, packets };
        clip.validate()?;
        Ok(clip)
    }

    /// A fresh, minimal Ogg Opus file for the platform player.
    pub fn to_ogg(&self) -> Vec<u8> {
        const SERIAL: u32 = 0x4D54_5631;
        let mut w = OggWriter { out: Vec::new(), serial: SERIAL, seq: 0 };
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1);
        head.push(self.channels);
        head.extend_from_slice(&self.pre_skip.to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        w.page(&[&head], 0, 0x02);
        let vendor = b"meshtrike";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.page(&[&tags], 0, 0);
        let mut granule: u64 = 0;
        let mut batch: Vec<&[u8]> = Vec::new();
        let (mut segs, mut bytes) = (0usize, 0usize);
        let n = self.packets.len();
        for (i, p) in self.packets.iter().enumerate() {
            let s = p.len() / 255 + 1;
            if !batch.is_empty() && (segs + s > 255 || bytes + p.len() > 4_000) {
                w.page(&batch, granule, 0);
                batch.clear();
                segs = 0;
                bytes = 0;
            }
            batch.push(p);
            segs += s;
            bytes += p.len();
            granule += packet_samples(p).unwrap_or(0) as u64;
            if i == n - 1 {
                w.page(&batch, granule, 0x04);
            }
        }
        w.out
    }
}

// ===== Ogg pages (RFC 3533) =====

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

struct OggWriter {
    out: Vec<u8>,
    serial: u32,
    seq: u32,
}

impl OggWriter {
    fn page(&mut self, packets: &[&[u8]], granule: u64, flags: u8) {
        let mut lacing = Vec::new();
        for p in packets {
            lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
            lacing.push((p.len() % 255) as u8);
        }
        let start = self.out.len();
        self.out.extend_from_slice(b"OggS");
        self.out.push(0);
        self.out.push(flags);
        self.out.extend_from_slice(&granule.to_le_bytes());
        self.out.extend_from_slice(&self.serial.to_le_bytes());
        self.out.extend_from_slice(&self.seq.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
        self.out.push(lacing.len() as u8);
        self.out.extend_from_slice(&lacing);
        for p in packets {
            self.out.extend_from_slice(p);
        }
        let crc = crc32(&self.out[start..]);
        self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
        self.seq += 1;
    }
}

/// Every packet of the first logical stream, checking page CRCs.
fn ogg_packets(b: &[u8]) -> Result<Vec<Vec<u8>>, MediaError> {
    let mut pos = 0;
    let mut serial = None;
    let mut packets = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    while pos < b.len() {
        let h = b.get(pos..pos + 27).ok_or(MediaError::Malformed)?;
        if &h[..4] != b"OggS" || h[4] != 0 {
            return Err(MediaError::Malformed);
        }
        let s = u32::from_le_bytes([h[14], h[15], h[16], h[17]]);
        let nseg = h[26] as usize;
        let table = b.get(pos + 27..pos + 27 + nseg).ok_or(MediaError::Malformed)?;
        let body_len: usize = table.iter().map(|&x| x as usize).sum();
        let end = pos + 27 + nseg + body_len;
        let page = b.get(pos..end).ok_or(MediaError::Malformed)?;
        let mut check = page.to_vec();
        check[22..26].copy_from_slice(&[0; 4]);
        if crc32(&check) != u32::from_le_bytes([h[22], h[23], h[24], h[25]]) {
            return Err(MediaError::Malformed);
        }
        if *serial.get_or_insert(s) == s {
            let mut off = pos + 27 + nseg;
            for &l in table {
                cur.extend_from_slice(&b[off..off + l as usize]);
                off += l as usize;
                if l < 255 {
                    packets.push(std::mem::take(&mut cur));
                    if packets.len() > MAX_PACKETS + 2 {
                        return Err(MediaError::TooLarge);
                    }
                }
            }
        }
        pos = end;
    }
    Ok(packets)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip of `n` 20 ms CELT packets (TOC 0xF8: config 31, one frame).
    fn clip(n: usize) -> VoiceClip {
        VoiceClip {
            channels: 1,
            pre_skip: 312,
            packets: (0..n).map(|i| {
                let mut p = vec![0xF8];
                p.extend((0..(30 + i % 50)).map(|j| (i * 7 + j) as u8));
                p
            }).collect(),
        }
    }

    #[test]
    fn packet_durations_follow_the_toc() {
        assert_eq!(packet_samples(&[0xF8, 1]), Some(960)); // CELT 20 ms
        assert_eq!(packet_samples(&[0x08, 1]), Some(960)); // SILK NB 20 ms
        assert_eq!(packet_samples(&[0x78, 1]), Some(960)); // hybrid 20 ms
        assert_eq!(packet_samples(&[0xF9, 1, 2]), Some(1920)); // two frames
        assert_eq!(packet_samples(&[0xF9, 1]), None); // odd split
        assert_eq!(packet_samples(&[0xFB, 0x03, 1]), Some(2880)); // code 3, 3 frames
        assert_eq!(packet_samples(&[0xFB, 0x07, 1]), None); // 140 ms is too long
        assert_eq!(packet_samples(&[]), None);
    }

    #[test]
    fn ogg_roundtrip_keeps_only_the_audio() {
        let c = clip(250); // 5 s
        let ogg = c.to_ogg();
        let back = VoiceClip::from_ogg(&ogg).unwrap();
        assert_eq!(back, c);
        assert_eq!(c.duration_ms(), (250 * 960 - 312) * 1000 / 48_000);
        // The rebuilt file's tags carry only our vendor string.
        assert!(ogg.windows(9).any(|w| w == b"meshtrike"));
    }

    #[test]
    fn blob_roundtrip_and_tampering() {
        let c = clip(40);
        let b = c.to_blob();
        assert_eq!(VoiceClip::from_blob(&b).unwrap(), c);
        assert!(VoiceClip::from_blob(&b[..b.len() - 1]).is_err());
        let mut extra = b.clone();
        extra.push(0);
        assert!(VoiceClip::from_blob(&extra).is_err());
        let mut bad = b.clone();
        bad[0] = b'X';
        assert!(VoiceClip::from_blob(&bad).is_err());
        // A packet whose TOC claims too much audio is refused.
        let mut evil = clip(2);
        evil.packets[1] = vec![0xFB, 0x3F, 0];
        assert!(VoiceClip::from_blob(&evil.to_blob()).is_err());
    }

    #[test]
    fn corrupted_ogg_is_refused() {
        let mut ogg = clip(10).to_ogg();
        let n = ogg.len();
        ogg[n - 3] ^= 0xFF; // breaks the last page's CRC
        assert!(VoiceClip::from_ogg(&ogg).is_err());
        assert!(VoiceClip::from_ogg(b"not ogg").is_err());
    }

    #[test]
    fn too_long_is_refused() {
        // 5 min 3 s of 20 ms packets.
        let c = clip(15_150);
        assert!(VoiceClip::from_blob(&c.to_blob()).is_err());
        assert!(VoiceClip::from_blob(&clip(14_900).to_blob()).is_ok());
    }
}
