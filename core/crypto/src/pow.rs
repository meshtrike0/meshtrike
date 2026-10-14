//! Broadcast proof-of-work (section 8): SHA-256 hashcash over the header with
//! TTL 0 and the payload; the 8-byte proof field holds the nonce.

use crate::sha256;

pub fn leading_zero_bits(h: &[u8]) -> u32 {
    let mut bits = 0;
    for &b in h {
        if b == 0 {
            bits += 8;
        } else {
            bits += b.leading_zeros();
            break;
        }
    }
    bits
}

/// `prefix` is the header with TTL and proof zeroed, `suffix` the payload.
pub fn pow_hash(prefix: &[u8], nonce: u64, suffix: &[u8]) -> [u8; 32] {
    sha256(&[prefix, &nonce.to_be_bytes(), suffix])
}

pub fn pow_check(prefix: &[u8], nonce: u64, suffix: &[u8], bits: u32) -> bool {
    leading_zero_bits(&pow_hash(prefix, nonce, suffix)) >= bits
}

/// Search nonces from `start`; returns the first valid one.
pub fn pow_solve(prefix: &[u8], suffix: &[u8], bits: u32, start: u64) -> u64 {
    let mut n = start;
    loop {
        if pow_check(prefix, n, suffix, bits) {
            return n;
        }
        n = n.wrapping_add(1);
    }
}

/// Required difficulty given the observed broadcast rate (section 16):
/// 20 bits, +2 per doubling above 30/min, max 26.
pub fn required_bits(broadcasts_per_min: u32) -> u32 {
    use mesh_constants::{POW_BASE_BITS, POW_MAX_BITS, POW_RATE_THRESHOLD_PER_MIN};
    let mut bits = POW_BASE_BITS;
    let mut threshold = POW_RATE_THRESHOLD_PER_MIN;
    while broadcasts_per_min > threshold && bits < POW_MAX_BITS {
        bits += 2;
        threshold = threshold.saturating_mul(2);
    }
    bits.min(POW_MAX_BITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_and_check() {
        let n = pow_solve(b"hdr", b"body", 12, 0);
        assert!(pow_check(b"hdr", n, b"body", 12));
    }

    #[test]
    fn difficulty_schedule() {
        assert_eq!(required_bits(0), 20);
        assert_eq!(required_bits(30), 20);
        assert_eq!(required_bits(31), 22);
        assert_eq!(required_bits(61), 24);
        assert_eq!(required_bits(10_000), 26);
    }
}
