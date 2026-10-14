//! Noise wrappers (section 6): NN for link encryption, IK for in-person
//! pairing, XX for remote pairing.

use crate::CryptoError;
use mesh_constants::LINK_REKEY_FRAMES;

const MAX_NOISE_MSG: usize = 65_535;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    NN,
    IK,
    XX,
}

impl Pattern {
    fn params(self) -> &'static str {
        match self {
            Pattern::NN => "Noise_NN_25519_ChaChaPoly_BLAKE2s",
            Pattern::IK => "Noise_IK_25519_ChaChaPoly_BLAKE2s",
            Pattern::XX => "Noise_XX_25519_ChaChaPoly_BLAKE2s",
        }
    }
}

fn err(e: snow::Error) -> CryptoError {
    CryptoError::Noise(format!("{e:?}"))
}

pub struct Handshake {
    hs: snow::HandshakeState,
}

impl Handshake {
    pub fn initiator(
        pattern: Pattern,
        local_static: Option<&[u8; 32]>,
        remote_static: Option<&[u8; 32]>,
    ) -> Result<Self, CryptoError> {
        let mut b = snow::Builder::new(pattern.params().parse().map_err(err)?);
        if let Some(k) = local_static {
            b = b.local_private_key(k).map_err(err)?;
        }
        if let Some(k) = remote_static {
            b = b.remote_public_key(k).map_err(err)?;
        }
        Ok(Self {
            hs: b.build_initiator().map_err(err)?,
        })
    }

    pub fn responder(
        pattern: Pattern,
        local_static: Option<&[u8; 32]>,
    ) -> Result<Self, CryptoError> {
        let mut b = snow::Builder::new(pattern.params().parse().map_err(err)?);
        if let Some(k) = local_static {
            b = b.local_private_key(k).map_err(err)?;
        }
        Ok(Self {
            hs: b.build_responder().map_err(err)?,
        })
    }

    pub fn write(&mut self, payload: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; MAX_NOISE_MSG];
        let n = self.hs.write_message(payload, &mut buf).map_err(err)?;
        buf.truncate(n);
        Ok(buf)
    }

    pub fn read(&mut self, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; MAX_NOISE_MSG];
        let n = self.hs.read_message(msg, &mut buf).map_err(err)?;
        buf.truncate(n);
        Ok(buf)
    }

    pub fn is_finished(&self) -> bool {
        self.hs.is_handshake_finished()
    }

    pub fn is_my_turn(&self) -> bool {
        self.hs.is_my_turn()
    }

    pub fn remote_static(&self) -> Option<[u8; 32]> {
        self.hs.get_remote_static().and_then(|s| s.try_into().ok())
    }

    pub fn handshake_hash(&self) -> Vec<u8> {
        self.hs.get_handshake_hash().to_vec()
    }

    pub fn into_transport(self) -> Result<Transport, CryptoError> {
        Ok(Transport {
            ts: self.hs.into_transport_mode().map_err(err)?,
            sent: 0,
            received: 0,
        })
    }
}

/// Link transport. Both directions rekey every `LINK_REKEY_FRAMES` frames.
/// The 1-hour rekey of section 6 never triggers because links are torn down
/// at every 15-minute epoch change (section 5).
pub struct Transport {
    ts: snow::TransportState,
    sent: u64,
    received: u64,
}

impl Transport {
    pub fn encrypt(&mut self, payload: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; payload.len() + 16];
        let n = self.ts.write_message(payload, &mut buf).map_err(err)?;
        buf.truncate(n);
        self.sent += 1;
        if self.sent.is_multiple_of(LINK_REKEY_FRAMES) {
            self.ts.rekey_outgoing();
        }
        Ok(buf)
    }

    pub fn decrypt(&mut self, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; msg.len()];
        let n = self.ts.read_message(msg, &mut buf).map_err(err)?;
        buf.truncate(n);
        self.received += 1;
        if self.received.is_multiple_of(LINK_REKEY_FRAMES) {
            self.ts.rekey_incoming();
        }
        Ok(buf)
    }

    pub fn frames_sent(&self) -> u64 {
        self.sent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{random_array, x25519_public};

    #[test]
    fn nn_link_roundtrip_with_rekey() {
        let mut i = Handshake::initiator(Pattern::NN, None, None).unwrap();
        let mut r = Handshake::responder(Pattern::NN, None).unwrap();
        let m1 = i.write(&[]).unwrap();
        r.read(&m1).unwrap();
        let m2 = r.write(&[]).unwrap();
        i.read(&m2).unwrap();
        let (mut ti, mut tr) = (i.into_transport().unwrap(), r.into_transport().unwrap());
        for n in 0..(LINK_REKEY_FRAMES + 5) {
            let c = ti.encrypt(&n.to_be_bytes()).unwrap();
            assert_eq!(tr.decrypt(&c).unwrap(), n.to_be_bytes());
        }
    }

    #[test]
    fn ik_pairing_carries_payloads_and_statics() {
        let (a_sk, b_sk) = (random_array::<32>(), random_array::<32>());
        let b_pk = x25519_public(&b_sk);
        let mut alice = Handshake::initiator(Pattern::IK, Some(&a_sk), Some(&b_pk)).unwrap();
        let mut bob = Handshake::responder(Pattern::IK, Some(&b_sk)).unwrap();
        let m1 = alice.write(b"token+card").unwrap();
        assert_eq!(bob.read(&m1).unwrap(), b"token+card");
        assert_eq!(bob.remote_static(), Some(x25519_public(&a_sk)));
        let m2 = bob.write(b"bob card").unwrap();
        assert_eq!(alice.read(&m2).unwrap(), b"bob card");
        assert!(alice.is_finished() && bob.is_finished());
        assert_eq!(alice.handshake_hash(), bob.handshake_hash());
    }

    #[test]
    fn xx_three_messages() {
        let (a_sk, b_sk) = (random_array::<32>(), random_array::<32>());
        let mut a = Handshake::initiator(Pattern::XX, Some(&a_sk), None).unwrap();
        let mut b = Handshake::responder(Pattern::XX, Some(&b_sk)).unwrap();
        b.read(&a.write(b"ret").unwrap()).unwrap();
        assert_eq!(a.read(&b.write(b"bob").unwrap()).unwrap(), b"bob");
        assert_eq!(b.read(&a.write(b"alice").unwrap()).unwrap(), b"alice");
        assert_eq!(a.remote_static(), Some(x25519_public(&b_sk)));
        assert_eq!(b.remote_static(), Some(x25519_public(&a_sk)));
    }
}
