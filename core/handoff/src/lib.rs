//! QR handoff (blueprint section 12.2, amended): bundle format, RaptorQ
//! fountain framing, and QR encode/decode helpers.
//!
//! Bundle: `"MQH1" ‖ count u32 BE ‖ body_len u32 BE ‖ SHA-256(body) ‖ body`,
//! body = concat of `(len u16 BE ‖ envelope)`.
//!
//! Frame (amended): `transfer_id u32 BE ‖ bundle_len u32 BE ‖ payload_id (4) ‖
//! symbol (800)` = 812 bytes. Both sides derive the RaptorQ
//! `ObjectTransmissionInformation` from `(bundle_len, 800)`.
#![forbid(unsafe_code)]

use std::collections::{HashSet, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};

use mesh_constants::{HANDOFF_BUNDLE_CAP, HANDOFF_REPAIR_PERCENT, HANDOFF_SYMBOL_SIZE};
use raptorq::{EncodingPacket, ObjectTransmissionInformation, PayloadId};

pub const BUNDLE_MAGIC: [u8; 4] = *b"MQH1";
/// magic (4) + count (4) + body length (4) + SHA-256 (32).
pub const BUNDLE_HEADER_LEN: usize = 44;
/// transfer ID (4) + bundle length (4).
pub const FRAME_HEADER_LEN: usize = 8;
/// RaptorQ payload ID (SBN 1 byte + ESI 3 bytes).
pub const PAYLOAD_ID_LEN: usize = 4;
/// Every frame is exactly this long (812).
pub const FRAME_LEN: usize = FRAME_HEADER_LEN + PAYLOAD_ID_LEN + HANDOFF_SYMBOL_SIZE as usize;
/// QR version used for handoff frames (byte mode, EC level L: 858 bytes).
pub const HANDOFF_QR_VERSION: i16 = 20;

/// Encoding symbol IDs are 24-bit.
const MAX_ESI: u32 = 1 << 24;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandoffError {
    #[error("bad bundle magic")]
    BadMagic,
    #[error("malformed bundle")]
    Malformed,
    #[error("bundle hash mismatch")]
    HashMismatch,
    #[error("bundle too large")]
    TooLarge,
    #[error("QR encoding failed: {0}")]
    QrEncode(String),
}

// ---------------------------------------------------------------------------
// Bundle
// ---------------------------------------------------------------------------

/// Packs envelopes, in order, into a bundle of at most `HANDOFF_BUNDLE_CAP`
/// bytes. Envelopes that do not fit (or exceed 65535 bytes) are skipped.
pub fn build_bundle(envelopes: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut count: u32 = 0;
    for env in envelopes {
        let Ok(len) = u16::try_from(env.len()) else {
            continue;
        };
        if BUNDLE_HEADER_LEN + body.len() + 2 + env.len() > HANDOFF_BUNDLE_CAP {
            continue;
        }
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(env);
        count += 1;
    }
    let hash = mesh_crypto::sha256(&[&body]);
    let mut out = Vec::with_capacity(BUNDLE_HEADER_LEN + body.len());
    out.extend_from_slice(&BUNDLE_MAGIC);
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&hash);
    out.extend_from_slice(&body);
    out
}

/// Verifies and unpacks a bundle. Never panics.
pub fn parse_bundle(bytes: &[u8]) -> Result<Vec<Vec<u8>>, HandoffError> {
    if bytes.len() > HANDOFF_BUNDLE_CAP {
        return Err(HandoffError::TooLarge);
    }
    if bytes.len() < BUNDLE_HEADER_LEN {
        return Err(HandoffError::Malformed);
    }
    if bytes[..4] != BUNDLE_MAGIC {
        return Err(HandoffError::BadMagic);
    }
    let count = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let body_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let body = &bytes[BUNDLE_HEADER_LEN..];
    if body.len() != body_len {
        return Err(HandoffError::Malformed);
    }
    if mesh_crypto::sha256(&[body]) != bytes[12..44] {
        return Err(HandoffError::HashMismatch);
    }
    // Each envelope costs at least 2 bytes, which bounds the allocation.
    if count > body.len() / 2 {
        return Err(HandoffError::Malformed);
    }
    let mut out = Vec::with_capacity(count);
    let mut pos = 0usize;
    while pos < body.len() {
        if body.len() - pos < 2 {
            return Err(HandoffError::Malformed);
        }
        let len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
        pos += 2;
        if body.len() - pos < len {
            return Err(HandoffError::Malformed);
        }
        out.push(body[pos..pos + len].to_vec());
        pos += len;
    }
    if out.len() != count {
        return Err(HandoffError::Malformed);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Fountain framing
// ---------------------------------------------------------------------------

/// Deterministic RaptorQ parameters shared by sender and receiver.
fn oti_for(bundle_len: u32) -> ObjectTransmissionInformation {
    ObjectTransmissionInformation::with_defaults(u64::from(bundle_len), HANDOFF_SYMBOL_SIZE)
}

fn valid_bundle_len(len: usize) -> bool {
    (BUNDLE_HEADER_LEN..=HANDOFF_BUNDLE_CAP).contains(&len)
}

/// Emits all source packets, then repair packets forever (in batches of
/// `HANDOFF_REPAIR_PERCENT`% of the source count, never repeating an ESI
/// until the 24-bit ESI space is exhausted).
pub struct HandoffSender {
    transfer_id: u32,
    bundle_len: u32,
    encoder: raptorq::Encoder,
    queue: VecDeque<EncodingPacket>,
    source_packets: usize,
    max_block_symbols: u32,
    repair_batch: u32,
    next_repair_id: u32,
}

impl HandoffSender {
    /// # Panics
    /// If the bundle is shorter than `BUNDLE_HEADER_LEN` or longer than
    /// `HANDOFF_BUNDLE_CAP` (never the case for [`build_bundle`] output).
    /// Use [`HandoffSender::try_new`] to get an error instead.
    pub fn new(bundle: Vec<u8>, transfer_id: u32) -> Self {
        match Self::try_new(bundle, transfer_id) {
            Ok(s) => s,
            Err(e) => panic!("invalid handoff bundle: {e}"),
        }
    }

    pub fn try_new(bundle: Vec<u8>, transfer_id: u32) -> Result<Self, HandoffError> {
        if bundle.len() > HANDOFF_BUNDLE_CAP {
            return Err(HandoffError::TooLarge);
        }
        if !valid_bundle_len(bundle.len()) {
            return Err(HandoffError::Malformed);
        }
        let bundle_len = bundle.len() as u32;
        let encoder = raptorq::Encoder::new(&bundle, oti_for(bundle_len));
        let mut queue = VecDeque::new();
        let mut max_block_symbols = 0u32;
        for block in encoder.get_block_encoders() {
            let src = block.source_packets();
            max_block_symbols = max_block_symbols.max(src.len() as u32);
            queue.extend(src);
        }
        let source_packets = queue.len();
        let repair_batch = ((max_block_symbols * HANDOFF_REPAIR_PERCENT).div_ceil(100)).max(1);
        Ok(HandoffSender {
            transfer_id,
            bundle_len,
            encoder,
            queue,
            source_packets,
            max_block_symbols,
            repair_batch,
            next_repair_id: 0,
        })
    }

    /// Number of source packets (frames needed with zero loss).
    pub fn source_packets(&self) -> usize {
        self.source_packets
    }

    /// The next 812-byte frame.
    pub fn next_frame(&mut self) -> Vec<u8> {
        if self.queue.is_empty() {
            self.refill_repair();
        }
        let packet = self
            .queue
            .pop_front()
            .expect("repair refill always yields packets");
        let mut frame = Vec::with_capacity(FRAME_LEN);
        frame.extend_from_slice(&self.transfer_id.to_be_bytes());
        frame.extend_from_slice(&self.bundle_len.to_be_bytes());
        frame.extend_from_slice(&packet.serialize());
        frame
    }

    fn refill_repair(&mut self) {
        // ESI = K + repair_id must stay below 2^24; wrap around if exhausted.
        if self.max_block_symbols + self.next_repair_id + self.repair_batch >= MAX_ESI {
            self.next_repair_id = 0;
        }
        for block in self.encoder.get_block_encoders() {
            self.queue
                .extend(block.repair_packets(self.next_repair_id, self.repair_batch));
        }
        self.next_repair_id += self.repair_batch;
    }
}

/// Receiver progress after each frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Progress {
    /// Distinct packets accepted for the locked transfer.
    pub received: u32,
    /// Source packets in the transfer (minimum needed to decode).
    pub needed: u32,
    /// Decoding finished (successfully or with `error`).
    pub done: bool,
    /// The envelopes, returned exactly once: on the frame that completes decoding.
    pub envelopes: Option<Vec<Vec<u8>>>,
    pub error: Option<String>,
}

struct Locked {
    transfer_id: u32,
    bundle_len: u32,
    decoder: raptorq::Decoder,
    source_blocks: u8,
    seen: HashSet<(u8, u32)>,
    needed: u32,
    max_packets: usize,
    done: bool,
    error: Option<String>,
}

/// Collects frames for a single transfer (the first valid transfer ID seen).
#[derive(Default)]
pub struct HandoffReceiver {
    locked: Option<Locked>,
}

impl HandoffReceiver {
    pub fn new() -> Self {
        HandoffReceiver { locked: None }
    }

    /// Forget the locked transfer and start over.
    pub fn reset(&mut self) {
        self.locked = None;
    }

    /// The transfer ID this receiver has locked onto, if any.
    pub fn transfer_id(&self) -> Option<u32> {
        self.locked.as_ref().map(|l| l.transfer_id)
    }

    fn progress(&self) -> Progress {
        match &self.locked {
            None => Progress::default(),
            Some(l) => Progress {
                received: l.seen.len() as u32,
                needed: l.needed,
                done: l.done,
                envelopes: None,
                error: l.error.clone(),
            },
        }
    }

    /// Feeds one scanned frame. Malformed frames, frames of other transfers and
    /// duplicates are ignored. Never panics.
    pub fn push_frame(&mut self, frame: &[u8]) -> Progress {
        if frame.len() != FRAME_LEN {
            return self.progress();
        }
        let tid = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
        let blen = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]);
        if !valid_bundle_len(blen as usize) {
            return self.progress();
        }

        if self.locked.is_none() {
            let oti = oti_for(blen);
            let needed = u64::from(blen).div_ceil(u64::from(oti.symbol_size())) as u32;
            let decoder = match catch_unwind(|| raptorq::Decoder::new(oti)) {
                Ok(d) => d,
                Err(_) => return self.progress(),
            };
            self.locked = Some(Locked {
                transfer_id: tid,
                bundle_len: blen,
                decoder,
                source_blocks: oti.source_blocks(),
                seen: HashSet::new(),
                needed,
                // Honest streams decode within a few packets of `needed`;
                // bound memory against floods of bogus unique packets.
                max_packets: needed as usize * 2 + 64,
                done: false,
                error: None,
            });
        }
        let Some(l) = self.locked.as_mut() else {
            return Progress::default();
        };
        if l.done || tid != l.transfer_id || blen != l.bundle_len {
            return self.progress();
        }

        let sbn = frame[8];
        let esi = u32::from_be_bytes([0, frame[9], frame[10], frame[11]]);
        if sbn >= l.source_blocks || !l.seen.insert((sbn, esi)) {
            return self.progress();
        }
        if l.seen.len() > l.max_packets {
            l.done = true;
            l.error = Some("too many packets without completing".into());
            return self.progress();
        }

        let packet = EncodingPacket::new(PayloadId::new(sbn, esi), frame[12..].to_vec());
        let decoder = &mut l.decoder;
        let result = match catch_unwind(AssertUnwindSafe(|| decoder.decode(packet))) {
            Ok(r) => r,
            Err(_) => {
                l.done = true;
                l.error = Some("fountain decoder failure".into());
                return self.progress();
            }
        };
        let Some(bundle) = result else {
            return self.progress();
        };

        l.done = true;
        let mut p;
        match parse_bundle(&bundle) {
            Ok(envs) => {
                p = self.progress();
                p.envelopes = Some(envs);
            }
            Err(e) => {
                l.error = Some(e.to_string());
                p = self.progress();
            }
        }
        p
    }
}

// ---------------------------------------------------------------------------
// QR
// ---------------------------------------------------------------------------

/// A QR symbol without quiet zone: `size * size` modules, row-major, 1 = dark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMatrix {
    pub size: u32,
    pub modules: Vec<u8>,
}

fn encode_byte_mode(
    data: &[u8],
    version: qrcode::Version,
    ec: qrcode::EcLevel,
) -> Result<QrMatrix, qrcode::types::QrError> {
    let mut bits = qrcode::bits::Bits::new(version);
    bits.push_byte_data(data)?;
    bits.push_terminator(ec)?;
    let code = qrcode::QrCode::with_bits(bits, ec)?;
    let size = code.width() as u32;
    let modules = code
        .to_colors()
        .into_iter()
        .map(|c| u8::from(c == qrcode::Color::Dark))
        .collect();
    Ok(QrMatrix { size, modules })
}

/// Handoff frame QR: version 20, EC level L, byte mode (fits 812 bytes).
pub fn qr_encode_handoff(data: &[u8]) -> Result<QrMatrix, HandoffError> {
    encode_byte_mode(
        data,
        qrcode::Version::Normal(HANDOFF_QR_VERSION),
        qrcode::EcLevel::L,
    )
    .map_err(|e| HandoffError::QrEncode(e.to_string()))
}

/// General QR (pairing / community codes): smallest version, EC level M, byte mode.
pub fn qr_encode(data: &[u8]) -> Result<QrMatrix, HandoffError> {
    let mut last = None;
    for v in 1..=40 {
        match encode_byte_mode(data, qrcode::Version::Normal(v), qrcode::EcLevel::M) {
            Ok(m) => return Ok(m),
            Err(e) => last = Some(e),
        }
    }
    Err(HandoffError::QrEncode(
        last.map(|e| e.to_string()).unwrap_or_default(),
    ))
}

/// Decodes every QR code found in an 8-bit grayscale image, returning raw
/// payload bytes (byte-mode payloads are arbitrary binary). Never panics;
/// returns nothing if `luma.len() != width * height`.
pub fn qr_decode_luma(width: u32, height: u32, luma: &[u8]) -> Vec<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || w.checked_mul(h) != Some(luma.len()) {
        return Vec::new();
    }
    catch_unwind(|| {
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| luma[y * w + x]);
        let mut out: Vec<Vec<u8>> = Vec::new();
        for grid in img.detect_grids() {
            let mut buf = Vec::new();
            if grid.decode_to(&mut buf).is_ok() && !out.contains(&buf) {
                out.push(buf);
            }
        }
        out
    })
    .unwrap_or_default()
}

/// Renders a matrix as a square grayscale image (0 = dark, 255 = light) with
/// `scale` pixels per module and a `quiet`-module border. Returns (side, pixels).
pub fn render_luma(m: &QrMatrix, scale: u32, quiet: u32) -> (u32, Vec<u8>) {
    let scale = scale.max(1) as usize;
    let quiet = quiet as usize;
    let n = m.size as usize;
    let side = (n + 2 * quiet) * scale;
    let mut img = vec![255u8; side * side];
    for my in 0..n {
        for mx in 0..n {
            if m.modules.get(my * n + mx).copied().unwrap_or(0) == 0 {
                continue;
            }
            let (px, py) = ((mx + quiet) * scale, (my + quiet) * scale);
            for y in py..py + scale {
                img[y * side + px..y * side + px + scale].fill(0);
            }
        }
    }
    (side as u32, img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Small deterministic PRNG for tests (xorshift64*).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| self.next() as u8).collect()
        }
    }

    fn envelopes(rng: &mut Rng, n: usize, size: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| rng.bytes(size + i % 17)).collect()
    }

    #[test]
    fn bundle_roundtrip() {
        let mut rng = Rng(1);
        let envs = envelopes(&mut rng, 50, 700);
        let b = build_bundle(&envs);
        assert_eq!(&b[..4], b"MQH1");
        assert_eq!(parse_bundle(&b).unwrap(), envs);
        let empty = build_bundle(&[]);
        assert_eq!(empty.len(), BUNDLE_HEADER_LEN);
        assert_eq!(parse_bundle(&empty).unwrap(), Vec::<Vec<u8>>::new());
        assert_eq!(
            parse_bundle(&build_bundle(&[vec![]])).unwrap(),
            vec![Vec::<u8>::new()]
        );
    }

    #[test]
    fn bundle_respects_cap_and_skips() {
        let big = vec![7u8; 60_000];
        let envs: Vec<Vec<u8>> = (0..20).map(|_| big.clone()).collect();
        let mut with_small = envs.clone();
        with_small.push(vec![1, 2, 3]);
        with_small.insert(0, vec![0u8; 70_000]); // > u16::MAX, skipped
        let b = build_bundle(&with_small);
        assert!(b.len() <= HANDOFF_BUNDLE_CAP);
        let parsed = parse_bundle(&b).unwrap();
        // 17 * 60002 + 44 fits under 1 MiB, 18 does not; small one still fits.
        assert_eq!(parsed.len(), 18);
        assert_eq!(parsed.last().unwrap(), &vec![1, 2, 3]);
    }

    #[test]
    fn bundle_tamper_detection() {
        let mut rng = Rng(2);
        let b = build_bundle(&envelopes(&mut rng, 5, 100));
        let mut t = b.clone();
        t[BUNDLE_HEADER_LEN + 10] ^= 1;
        assert_eq!(parse_bundle(&t), Err(HandoffError::HashMismatch));
        let mut t = b.clone();
        t[20] ^= 1;
        assert_eq!(parse_bundle(&t), Err(HandoffError::HashMismatch));
        let mut t = b.clone();
        t[0] = b'X';
        assert_eq!(parse_bundle(&t), Err(HandoffError::BadMagic));
        let mut t = b.clone();
        t[7] ^= 1; // count
        assert_eq!(parse_bundle(&t), Err(HandoffError::Malformed));
        let mut t = b.clone();
        t.push(0);
        assert_eq!(parse_bundle(&t), Err(HandoffError::Malformed));
        assert_eq!(
            parse_bundle(&b[..b.len() - 1]),
            Err(HandoffError::Malformed)
        );
        assert!(parse_bundle(&[]).is_err());
    }

    #[test]
    fn oti_is_deterministic_from_length() {
        let mut rng = Rng(3);
        let bundle = build_bundle(&envelopes(&mut rng, 300, 650));
        let enc = raptorq::Encoder::new(&bundle, oti_for(bundle.len() as u32));
        let oti = enc.get_config();
        assert_eq!(oti, oti_for(bundle.len() as u32));
        assert_eq!(oti.symbol_size(), HANDOFF_SYMBOL_SIZE);
        // Serialized OTI identical too (transfer length, symbol size, blocks, alignment).
        assert_eq!(oti.serialize(), oti_for(bundle.len() as u32).serialize());
    }

    fn roundtrip_with_loss(
        bundle_envs: &[Vec<u8>],
        drop_percent: u64,
        seed: u64,
    ) -> (usize, usize) {
        let bundle = build_bundle(bundle_envs);
        let mut tx = HandoffSender::new(bundle, 0xDEAD_BEEF);
        let mut rx = HandoffReceiver::new();
        let mut rng = Rng(seed);
        let k = tx.source_packets();
        for sent in 1..=(k * 3) {
            let f = tx.next_frame();
            assert_eq!(f.len(), FRAME_LEN);
            if rng.next() % 100 < drop_percent {
                continue;
            }
            let p = rx.push_frame(&f);
            assert_eq!(p.needed as usize, k);
            assert!(p.error.is_none(), "{:?}", p.error);
            if p.done {
                assert_eq!(p.envelopes.as_deref(), Some(bundle_envs));
                // Further frames: done, no re-delivery.
                let again = rx.push_frame(&tx.next_frame());
                assert!(again.done && again.envelopes.is_none());
                return (k, sent);
            }
        }
        panic!("did not complete");
    }

    #[test]
    fn sender_receiver_200kb_with_15_percent_loss() {
        let mut rng = Rng(4);
        let envs = envelopes(&mut rng, 250, 790); // ~200 KB
        let total: usize = envs.iter().map(|e| e.len() + 2).sum();
        assert!((190_000..230_000).contains(&total));
        for seed in [11, 12, 13] {
            let (k, sent) = roundtrip_with_loss(&envs, 15, seed);
            assert!(k >= 240);
            assert!(sent < k * 3 / 2, "k={k} sent={sent}");
        }
    }

    #[test]
    fn sender_receiver_full_cap_bundle() {
        let mut rng = Rng(14);
        let envs: Vec<Vec<u8>> = (0..1400).map(|_| rng.bytes(4288)).collect();
        let bundle_len = build_bundle(&envs).len();
        assert!(bundle_len > HANDOFF_BUNDLE_CAP - 4300);
        let kept = parse_bundle(&build_bundle(&envs)).unwrap();
        let (k, _sent) = roundtrip_with_loss(&kept, 10, 15);
        assert_eq!(k, bundle_len.div_ceil(800));
    }

    #[test]
    fn sender_receiver_lossless_and_tiny() {
        let (k, sent) = roundtrip_with_loss(&[vec![1, 2, 3]], 0, 5);
        assert_eq!(k, 1);
        assert_eq!(sent, 1);
        let (_k, _sent) = roundtrip_with_loss(&[], 30, 6);
    }

    #[test]
    fn sender_repair_symbols_never_repeat() {
        let mut rng = Rng(7);
        let bundle = build_bundle(&envelopes(&mut rng, 10, 700));
        let mut tx = HandoffSender::new(bundle, 1);
        let mut ids = HashSet::new();
        for _ in 0..(tx.source_packets() * 10) {
            let f = tx.next_frame();
            assert!(ids.insert(f[8..12].to_vec()));
        }
    }

    #[test]
    fn receiver_ignores_garbage_and_other_transfers() {
        let mut rng = Rng(8);
        let envs = envelopes(&mut rng, 40, 500);
        let mut tx = HandoffSender::new(build_bundle(&envs), 42);
        let mut other = HandoffSender::new(build_bundle(&envelopes(&mut rng, 40, 500)), 43);
        let mut rx = HandoffReceiver::new();
        // Garbage of wrong lengths never locks.
        for n in [0usize, 1, 11, 811, 813, 5000] {
            let p = rx.push_frame(&rng.bytes(n));
            assert_eq!(p, Progress::default());
        }
        // Right length but impossible bundle length never locks.
        let mut bad = rng.bytes(FRAME_LEN);
        bad[4..8].copy_from_slice(&(HANDOFF_BUNDLE_CAP as u32 + 1).to_be_bytes());
        assert_eq!(rx.push_frame(&bad), Progress::default());
        bad[4..8].copy_from_slice(&3u32.to_be_bytes());
        assert_eq!(rx.push_frame(&bad), Progress::default());
        assert_eq!(rx.transfer_id(), None);

        let first = tx.next_frame();
        let p = rx.push_frame(&first);
        assert_eq!(p.received, 1);
        assert_eq!(rx.transfer_id(), Some(42));
        // Duplicate ignored.
        assert_eq!(rx.push_frame(&first).received, 1);
        // Other transfer ignored.
        for _ in 0..10 {
            assert_eq!(rx.push_frame(&other.next_frame()).received, 1);
        }
        // Bad source block number ignored.
        let mut f = tx.next_frame();
        f[8] = 9;
        assert_eq!(rx.push_frame(&f).received, 1);
        loop {
            let p = rx.push_frame(&tx.next_frame());
            if p.done {
                assert_eq!(p.envelopes.unwrap(), envs);
                break;
            }
        }
    }

    #[test]
    fn receiver_reports_hash_failure() {
        // Forge a bundle-sized object that is not a valid bundle.
        let mut fake = build_bundle(&[vec![9u8; 2000]]);
        let last = fake.len() - 1;
        fake[last] ^= 0xFF;
        let mut tx = HandoffSender::new(fake, 5);
        let mut rx = HandoffReceiver::new();
        loop {
            let p = rx.push_frame(&tx.next_frame());
            if p.done {
                assert!(p.envelopes.is_none());
                assert!(p.error.unwrap().contains("hash"));
                break;
            }
        }
    }

    #[test]
    fn qr_roundtrip_pairing_97_bytes() {
        let mut rng = Rng(9);
        for _ in 0..5 {
            let data = rng.bytes(97);
            let m = qr_encode(&data).unwrap();
            assert_eq!(m.modules.len(), (m.size * m.size) as usize);
            let (side, img) = render_luma(&m, 4, 4);
            assert_eq!(img.len(), (side * side) as usize);
            assert_eq!(qr_decode_luma(side, side, &img), vec![data]);
        }
    }

    #[test]
    fn qr_roundtrip_handoff_frame_v20() {
        let mut rng = Rng(10);
        let bundle = build_bundle(&envelopes(&mut rng, 20, 600));
        let mut tx = HandoffSender::new(bundle, 77);
        let frame = tx.next_frame();
        assert_eq!(frame.len(), 812);
        let m = qr_encode_handoff(&frame).unwrap();
        assert_eq!(m.size, 97); // version 20 = 17 + 4*20
        let (side, img) = render_luma(&m, 3, 4);
        assert_eq!(qr_decode_luma(side, side, &img), vec![frame]);
    }

    #[test]
    fn qr_encode_limits() {
        assert!(qr_encode_handoff(&[0u8; 859]).is_err());
        assert!(qr_encode_handoff(&[0u8; 858]).is_ok());
        assert_eq!(qr_encode(&[1u8; 10]).unwrap().size, 21);
        assert!(qr_encode(&[0u8; 4000]).is_err());
    }

    #[test]
    fn qr_decode_bad_inputs() {
        assert!(qr_decode_luma(0, 0, &[]).is_empty());
        assert!(qr_decode_luma(10, 10, &[0; 99]).is_empty());
        assert!(qr_decode_luma(u32::MAX, u32::MAX, &[0; 4]).is_empty());
        assert!(qr_decode_luma(64, 64, &[128; 64 * 64]).is_empty());
    }

    proptest! {
        #[test]
        fn parse_bundle_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = parse_bundle(&bytes);
            let mut b = b"MQH1".to_vec();
            b.extend_from_slice(&bytes);
            let _ = parse_bundle(&b);
        }

        #[test]
        fn receiver_never_panics(
            frames in proptest::collection::vec(proptest::collection::vec(any::<u8>(), FRAME_LEN..=FRAME_LEN), 1..20),
            blen in BUNDLE_HEADER_LEN as u32..20_000u32,
        ) {
            let mut rx = HandoffReceiver::new();
            for mut f in frames {
                f[0..4].copy_from_slice(&1u32.to_be_bytes());
                f[4..8].copy_from_slice(&blen.to_be_bytes());
                f[8] = 0;
                let _ = rx.push_frame(&f);
            }
        }

        #[test]
        fn qr_decode_never_panics(w in 1u32..64, h in 1u32..64, seed in any::<u64>()) {
            let mut rng = Rng(seed | 1);
            let img = rng.bytes((w * h) as usize);
            let _ = qr_decode_luma(w, h, &img);
        }
    }
}
