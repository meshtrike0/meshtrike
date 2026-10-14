//! Every protocol constant from blueprint section 16 (as amended in v1.02).
//! Changing any value here is a protocol change and needs a decision-log entry.
#![forbid(unsafe_code)]

// ===== Versions and sizes (section 7, A.1) =====
pub const PROTOCOL_VERSION: u8 = 0x02;
pub const PREVIOUS_PROTOCOL_VERSION: u8 = 0x01;
pub const ENVELOPE_HEADER_LEN: usize = 50;
pub const INNER_HEADER_LEN: usize = 35;
pub const AEAD_TAG_LEN: usize = 16;
pub const WIRE_MARGIN: usize = 192;
pub const ID_LEN: usize = 16;
pub const TAG_LEN: usize = 16;

/// Inner (plaintext) buckets in bytes, smallest first.
pub const INNER_BUCKETS: [usize; 4] = [256, 512, 1024, 4096];
pub const LORA_INNER_BUCKET: usize = 96;
/// Wire payload sizes (W) matching `INNER_BUCKETS` (inner + margin).
pub const WIRE_SIZES: [usize; 4] = [448, 704, 1216, 4288];
pub const LORA_WIRE_SIZE: usize = 167;
pub const LORA_ENVELOPE_LEN: usize = 233;
/// Ack and Cover payloads (unencrypted) are sized like an encrypted 256-bucket payload.
pub const ACK_PAYLOAD_LEN: usize = WIRE_SIZES[0] + AEAD_TAG_LEN;

pub const MAX_TEXT_BYTES: usize = 3_500;
/// A reply's quote of the message it answers (a short snippet, UTF-8).
pub const MAX_QUOTE_BYTES: usize = 120;
/// Waveform bars carried with a voice note (one byte each).
pub const MAX_WAVEFORM_BARS: usize = 64;
/// Largest voice note on the wire (5 minutes of 16 kbit/s Opus fits easily).
pub const MAX_VOICE_BYTES: usize = 1024 * 1024;
/// Internet route (amendment N1): longest Nym address accepted from a contact.
pub const MAX_REACH_BYTES: usize = 512;
/// Disguise bridges in one message, and the longest share link accepted.
pub const MAX_BRIDGES_IN_MESSAGE: usize = 4;
pub const MAX_BRIDGE_LINK_BYTES: usize = 768;
/// Bridges kept on this phone.
pub const MAX_BRIDGES: usize = 16;
pub const MAX_LORA_TEXT_BYTES: usize = LORA_INNER_BUCKET - INNER_HEADER_LEN; // 61
pub const IMAGE_CHUNK_BYTES: usize = 3_900;
pub const MAX_READ_RECEIPT_IDS: usize = 50;
pub const MAX_OTK_IN_MESSAGE: usize = 20;
pub const MAX_FILE_NAME_BYTES: usize = 64;
pub const MAX_GROUP_NAME_BYTES: usize = 64;
pub const MAX_NICKNAME_BYTES: usize = 32;

// ===== Discovery (section 5) =====
pub const EPOCH_SECS: u64 = 900;
pub const EPOCH_TOLERANCE: i64 = 1;
pub const BEACON_LEN: usize = 40;
pub const MAX_COMMUNITIES: usize = 4;
pub const COMMUNITY_ROTATION_MS: u64 = 2_000;
pub const PUBLIC_SERVICE_UUID: [u8; 16] = [
    0x6E, 0x4D, 0x00, 0x01, 0xB5, 0xA3, 0x4F, 0x39, 0x9C, 0x2E, 0x0D, 0x5A, 0x9F, 0x1C, 0x7B, 0x21,
];

// ===== Routing (section 8) =====
pub const MAX_TTL: u8 = 7;
pub const MIN_INITIAL_TTL: u8 = 5;
pub const SUMMARY_VECTOR_MAX: usize = 500;
pub const RELAY_MAX_ENVELOPES: usize = 2_000;
pub const RELAY_MAX_BYTES: usize = 50 * 1024 * 1024;
pub const EXPIRY_DEFAULT_SECS: u64 = 24 * 3600;
pub const EXPIRY_MAX_SECS: u64 = 72 * 3600;
pub const PER_TAG_QUOTA: usize = 200;
pub const SEEN_FILTER_SIZE: usize = 10_000;
pub const SEEN_FILTER_MS: u64 = 30 * 60 * 1000;
/// Rebroadcast probability in percent by neighbour count.
pub fn rebroadcast_percent(neighbours: usize) -> u32 {
    match neighbours {
        0..=3 => 100,
        4..=8 => 65,
        _ => 35,
    }
}
pub const LARGE_CONTENT_BYTES: usize = 256 * 1024;
pub const MAX_IMAGE_BYTES: usize = 256 * 1024;
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
pub const CHUNK_REASSEMBLY_MS: u64 = 24 * 3600 * 1000;

// ===== Link layer (sections 6, 7) =====
pub const BLE_ATT_MTU: u16 = 512;
pub const LINK_FRAME_HEADER_LEN: usize = 7;
pub const FRAGMENT_TIMEOUT_MS: u64 = 60_000;
pub const LINK_REKEY_FRAMES: u64 = 10_000;
pub const LINK_REKEY_MS: u64 = 3_600_000;
pub const NOISE_TAG_LEN: usize = 16;

// ===== Abuse resistance (section 8) =====
pub const RATE_LIMIT_FRAMES_PER_SEC: u32 = 20;
pub const RATE_LIMIT_BYTES_PER_SEC: u32 = 50 * 1024;
/// The per-link budget can absorb this many seconds of traffic at once:
/// queued frames arrive in bursts after radio stalls (Wi-Fi Direct setup
/// shares the 2.4 GHz radio with Bluetooth).
pub const RATE_LIMIT_BURST_SECS: u32 = 10;
pub const MALFORMED_LIMIT: u32 = 5;
pub const MALFORMED_WINDOW_MS: u64 = 60_000;
pub const MALFORMED_BLOCK_MS: u64 = 10 * 60_000;
pub const POW_BASE_BITS: u32 = 20;
pub const POW_MAX_BITS: u32 = 26;
pub const POW_RATE_THRESHOLD_PER_MIN: u32 = 30;

// ===== Traffic analysis (section 11) =====
pub const FORWARD_DELAY_MS: (u64, u64) = (50, 500);
pub const FORWARD_DELAY_HIGH_RISK_MS: (u64, u64) = (500, 5_000);
pub const COVER_SLOT_MS: u64 = 2_000;
pub const COVER_FRAMES_PER_SLOT: usize = 6;
pub const COVER_FRAME_LEN: usize = 180;

// ===== Keys (sections 4, 6) =====
pub const LOCAL_OTK_POOL: usize = 50;
pub const MIN_REMOTE_OTKS: usize = 5;
pub const REPLENISH_REMOTE_OTKS: usize = 10;
pub const CARD_OTKS: usize = 10;
pub const FALLBACK_ROTATION_MS: u64 = 7 * 24 * 3600 * 1000;
pub const MEGOLM_ROTATION_MESSAGES: u32 = 100;
pub const MEGOLM_ROTATION_MS: u64 = 7 * 24 * 3600 * 1000;
pub const MAX_GROUP_SIZE: usize = 50;

// ===== Pairing (section 15) =====
pub const PAIRING_WINDOW_MS: u64 = 120_000;
pub const PAIRING_QR_LEN: usize = 97;
pub const REMOTE_PAIRING_LIFETIME_MS: u64 = 30 * 60_000;

// ===== Countermeasures (section 12) =====
pub const LISTEN_ONLY_SUMMARY_MS: u64 = 5 * 60_000;
pub const LISTEN_ONLY_CONNECTION_MS: u64 = 30_000;
pub const TX_STEP_UP_MS: u64 = 60_000;
pub const TX_STEP_DOWN_MS: u64 = 5 * 60_000;
pub const TX_STEP_DOWN_NEIGHBOURS: usize = 3;
pub const TX_STEP_DOWN_RSSI: i16 = -70;
pub const LORA_DEFAULT_TX_DBM: i8 = 10;
pub const INTERFERENCE_SAMPLE_MS: u64 = 10_000;
pub const INTERFERENCE_BASELINE_MS: u64 = 10 * 60_000;
pub const INTERFERENCE_MIN_BASELINE: u32 = 10;
pub const INTERFERENCE_LOW_PERCENT: u32 = 20;
pub const INTERFERENCE_LOW_SAMPLES: usize = 6;
pub const INTERFERENCE_CLEAR_PERCENT: u32 = 50;
pub const INTERFERENCE_CLEAR_MS: u64 = 60_000;
pub const INTERFERENCE_CONNECT_MIN: u32 = 10;
pub const INTERFERENCE_CONNECT_FAIL_PERCENT: u32 = 70;
pub const HANDOFF_SYMBOL_SIZE: u16 = 800;
pub const HANDOFF_BUNDLE_CAP: usize = 1024 * 1024;
pub const HANDOFF_REPAIR_PERCENT: u32 = 20;
pub const HANDOFF_FPS: u32 = 8;

// ===== Lock (section 12.3) =====
pub const PIN_DIGITS: usize = 6;
pub const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
pub const ARGON2_ITERATIONS: u32 = 3;
pub const ARGON2_LANES: u32 = 1;
pub const WRONG_PIN_WIPE: u32 = 10;
pub const WRONG_PIN_SHOW_REMAINING: u32 = 5;
pub const AUTO_LOCK_MS: u64 = 5 * 60_000;
pub const CLIPBOARD_EXPIRY_MS: u64 = 60_000;
pub const INTEGRITY_RECHECK_MS: u64 = 7 * 24 * 3600 * 1000;

// ===== Storage (section 10) =====
pub const DISAPPEARING_DEFAULT_SECS: u32 = 7 * 24 * 3600;
pub const DISAPPEARING_HIGH_RISK_SECS: u32 = 24 * 3600;
pub const DISAPPEARING_MIN_SECS: u32 = 5 * 60;
pub const DISAPPEARING_MAX_SECS: u32 = 4 * 7 * 24 * 3600;

// ===== Clock (section 19) =====
pub const CLOCK_SKEW_EPOCHS: i64 = 2;
pub const CLOCK_SKEW_PEERS: usize = 3;
pub const CLOCK_SKEW_WINDOW_MS: u64 = 10 * 60_000;

/// Wire size for an inner bucket.
pub fn wire_size_for_inner(inner: usize) -> Option<usize> {
    if inner == LORA_INNER_BUCKET {
        return Some(LORA_WIRE_SIZE);
    }
    INNER_BUCKETS
        .iter()
        .position(|&b| b == inner)
        .map(|i| WIRE_SIZES[i])
}

/// Smallest inner bucket holding `len` bytes.
pub fn inner_bucket_for(len: usize) -> Option<usize> {
    INNER_BUCKETS.iter().copied().find(|&b| len <= b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_arithmetic_matches_spec() {
        for (i, w) in WIRE_SIZES.iter().enumerate() {
            assert_eq!(*w, INNER_BUCKETS[i] + WIRE_MARGIN);
        }
        assert_eq!(
            ENVELOPE_HEADER_LEN + LORA_WIRE_SIZE + AEAD_TAG_LEN,
            LORA_ENVELOPE_LEN
        );
        assert_eq!(MAX_LORA_TEXT_BYTES, 61);
        assert_eq!(COVER_FRAME_LEN - NOISE_TAG_LEN - LINK_FRAME_HEADER_LEN, 157);
        assert_eq!(inner_bucket_for(300), Some(512));
        assert_eq!(inner_bucket_for(5000), None);
    }
}
