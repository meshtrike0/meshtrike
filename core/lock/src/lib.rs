//! App lock (section 12.3): PIN stretching, the PIN-bound message-store key,
//! the duress verifier input, and the wrong-PIN policy.
#![forbid(unsafe_code)]

use argon2::{Algorithm, Argon2, Params, Version};
use mesh_constants::*;
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LockError {
    #[error("PIN must be {PIN_DIGITS} digits")]
    BadPin,
    #[error("argon2")]
    Argon2,
}

pub fn validate_pin(pin: &str) -> Result<(), LockError> {
    if pin.len() == PIN_DIGITS && pin.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(LockError::BadPin)
    }
}

/// P = Argon2id(PIN, salt, 64 MiB, t = 3, p = 1).
pub fn stretch_pin(pin: &str, salt: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, LockError> {
    stretch_pin_with(pin, salt, ARGON2_MEMORY_KIB, ARGON2_ITERATIONS)
}

/// Same with explicit cost (tests and the simulator use small costs).
pub fn stretch_pin_with(
    pin: &str,
    salt: &[u8; 16],
    m_kib: u32,
    t: u32,
) -> Result<Zeroizing<[u8; 32]>, LockError> {
    validate_pin(pin)?;
    let params = Params::new(m_kib, t, ARGON2_LANES, Some(32)).map_err(|_| LockError::Argon2)?;
    let a = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; 32]);
    a.hash_password_into(pin.as_bytes(), salt, out.as_mut())
        .map_err(|_| LockError::Argon2)?;
    Ok(out)
}

/// Message-store key = HKDF-SHA256(hardware-unwrapped key ‖ P, "msgdb-v1").
pub fn message_store_key(hw_key: &[u8; 32], p: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let ikm = Zeroizing::new([hw_key.as_slice(), p.as_slice()].concat());
    Zeroizing::new(mesh_crypto::hkdf32(&ikm, &[b"msgdb-v1"]))
}

/// Outcome of the wrong-PIN policy after a failed attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedAttempt {
    Remaining(u32),
    Wipe,
}

/// The counter is incremented before Argon2id runs and reset on success.
pub fn after_failure(counter_after_increment: u32) -> FailedAttempt {
    if counter_after_increment >= WRONG_PIN_WIPE {
        FailedAttempt::Wipe
    } else {
        FailedAttempt::Remaining(WRONG_PIN_WIPE - counter_after_increment)
    }
}

/// Show remaining attempts only after 5 failures (section 17).
pub fn show_remaining(counter: u32) -> bool {
    counter >= WRONG_PIN_SHOW_REMAINING
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_rules() {
        assert!(validate_pin("123456").is_ok());
        assert!(validate_pin("12345").is_err());
        assert!(validate_pin("12345a").is_err());
    }

    #[test]
    fn stretch_is_deterministic_and_salted() {
        let a = stretch_pin_with("123456", &[1; 16], 256, 1).unwrap();
        let b = stretch_pin_with("123456", &[1; 16], 256, 1).unwrap();
        let c = stretch_pin_with("123456", &[2; 16], 256, 1).unwrap();
        assert_eq!(*a, *b);
        assert_ne!(*a, *c);
        assert_ne!(
            *message_store_key(&[0; 32], &a),
            *message_store_key(&[0; 32], &c)
        );
    }

    #[test]
    fn full_cost_runs() {
        // The real parameters (64 MiB, t = 3) must work on this platform.
        stretch_pin("000000", &[0; 16]).unwrap();
    }

    #[test]
    fn wipe_after_ten() {
        assert_eq!(after_failure(1), FailedAttempt::Remaining(9));
        assert_eq!(after_failure(10), FailedAttempt::Wipe);
        assert!(!show_remaining(4));
        assert!(show_remaining(5));
    }
}
