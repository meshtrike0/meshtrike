//! Interfaces implemented by the platform (section 13, amendment C), plus
//! software implementations used by the simulator and the browser harness.

use crate::types::*;
use mesh_storage::{Kv, MemoryKv, Op, StorageError};
use std::sync::{Arc, Mutex};

pub trait TransportHost: Send + Sync {
    fn start_advertising(&self, service_uuids: Vec<[u8; 16]>, beacon: Vec<u8>, power: TxPower);
    fn stop_advertising(&self);
    fn start_scanning(&self, service_uuids: Vec<[u8; 16]>);
    fn stop_scanning(&self);
    fn connect(&self, p: PeerHandle, transport: TransportKind);
    fn send_frame(&self, p: PeerHandle, bytes: Vec<u8>) -> bool;
    fn disconnect(&self, p: PeerHandle);
    fn capabilities(&self) -> Capabilities;
    fn lora_send(&self, bytes: Vec<u8>) -> bool;
    /// Open a Wi-Fi peer-to-peer link to the neighbour behind BLE handle
    /// `peer` (section 9). The group owner creates the group; the other side
    /// joins it. Both ends then prove knowledge of `auth_token` over the new
    /// connection before any frame flows. The token is independent of the
    /// Wi-Fi credentials, so whatever handles the group (on Android, the
    /// Wi-Fi Link add-on) cannot pass that check. When the link is up the
    /// host reports it with `on_connected(.., TransportKind::WifiP2p)`.
    fn wifi_link(
        &self,
        _peer: PeerHandle,
        _group_owner: bool,
        _network_name: String,
        _passphrase: String,
        _auth_token: [u8; 32],
    ) {
    }
    /// Internet route (amendment N1): run it or stop it. `config_json` is
    /// `{"bridges": [share links], "allow_direct": bool}`. The host reports
    /// back through `on_internet_ready` / `on_internet_down`.
    fn internet_config(&self, _enabled: bool, _config_json: String) {}
    /// Send one sealed envelope to a Nym address. False when the route
    /// cannot take it now (the core keeps it owed).
    fn internet_send(&self, _address: String, _envelope: Vec<u8>) -> bool {
        false
    }
    /// One decrypted bundle of call audio (Opus packets, see `calls`) for
    /// the speaker. Called often (about 12 times a second) during a call.
    fn call_audio(&self, _data: Vec<u8>) {}
}

pub trait Keystore: Send + Sync {
    fn wrap(&self, slot: KeySlot, plaintext: Vec<u8>) -> Result<Vec<u8>, KeystoreError>;
    fn unwrap(&self, slot: KeySlot, wrapped: Vec<u8>) -> Result<Vec<u8>, KeystoreError>;
    /// Hardware-held HMAC-SHA256 (amendment C; duress verifier).
    fn hmac(&self, slot: KeySlot, data: Vec<u8>) -> Result<Vec<u8>, KeystoreError>;
    fn store_biometric(&self, secret: Vec<u8>) -> Result<(), KeystoreError>;
    /// Shows the OS prompt.
    fn load_biometric(&self) -> Result<Vec<u8>, KeystoreError>;
    fn counter_get(&self) -> u32;
    fn counter_set(&self, v: u32);
    fn destroy_all(&self);
}

/// The UI event channel. One method carrying `Event` replaces the per-event
/// methods of section 13 so the same sink serves Kotlin and JavaScript.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        }
    }
}

/// Where the two databases and the small key-metadata blob live.
pub trait StorageProvider: Send + Sync {
    /// Wrapped keys, salt and duress verifier (stored beside the databases).
    fn read_meta(&self) -> Option<Vec<u8>>;
    fn write_meta(&self, bytes: &[u8]);
    fn open_relay(&self, key: &[u8; 32], create: bool) -> Result<Box<dyn Kv>, StorageError>;
    fn open_message(&self, key: &[u8; 32], create: bool) -> Result<Box<dyn Kv>, StorageError>;
    /// Delete both databases and the metadata.
    fn wipe_all(&self);
}

// ===== Memory storage provider =====

pub struct SharedKv(pub Arc<Mutex<MemoryKv>>);

impl Kv for SharedKv {
    fn get(&self, table: &str, key: &[u8]) -> mesh_storage::Result<Option<Vec<u8>>> {
        self.0.lock().unwrap().get(table, key)
    }
    fn scan(&self, table: &str) -> mesh_storage::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.0.lock().unwrap().scan(table)
    }
    fn scan_prefix(
        &self,
        table: &str,
        prefix: &[u8],
    ) -> mesh_storage::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.0.lock().unwrap().scan_prefix(table, prefix)
    }
    fn apply(&mut self, ops: Vec<Op>) -> mesh_storage::Result<()> {
        self.0.lock().unwrap().apply(ops)
    }
    fn count(&self, table: &str) -> mesh_storage::Result<usize> {
        self.0.lock().unwrap().count(table)
    }
}

fn key_check(key: &[u8; 32]) -> [u8; 32] {
    mesh_crypto::sha256(&[b"kv-check-v1", key])
}

#[derive(Default)]
pub struct MemoryStorage {
    meta: Mutex<Option<Vec<u8>>>,
    relay: Mutex<Option<Arc<Mutex<MemoryKv>>>>,
    message: Mutex<Option<Arc<Mutex<MemoryKv>>>>,
}

impl MemoryStorage {
    fn open(
        slot: &Mutex<Option<Arc<Mutex<MemoryKv>>>>,
        key: &[u8; 32],
        create: bool,
    ) -> Result<Box<dyn Kv>, StorageError> {
        let mut s = slot.lock().unwrap();
        match (&*s, create) {
            (Some(db), _) => {
                db.lock().unwrap().verify(&key_check(key))?;
                Ok(Box::new(SharedKv(db.clone())))
            }
            (None, true) => {
                let db = Arc::new(Mutex::new(MemoryKv::create(key_check(key))));
                *s = Some(db.clone());
                Ok(Box::new(SharedKv(db)))
            }
            (None, false) => Err(StorageError::Io("missing database".into())),
        }
    }
}

impl StorageProvider for MemoryStorage {
    fn read_meta(&self) -> Option<Vec<u8>> {
        self.meta.lock().unwrap().clone()
    }
    fn write_meta(&self, bytes: &[u8]) {
        *self.meta.lock().unwrap() = Some(bytes.to_vec());
    }
    fn open_relay(&self, key: &[u8; 32], create: bool) -> Result<Box<dyn Kv>, StorageError> {
        Self::open(&self.relay, key, create)
    }
    fn open_message(&self, key: &[u8; 32], create: bool) -> Result<Box<dyn Kv>, StorageError> {
        Self::open(&self.message, key, create)
    }
    fn wipe_all(&self) {
        *self.meta.lock().unwrap() = None;
        *self.relay.lock().unwrap() = None;
        *self.message.lock().unwrap() = None;
    }
}

// ===== File-backed SQLCipher provider (device builds) =====

#[cfg(feature = "sqlcipher")]
pub struct FileStorage {
    pub dir: String,
}

#[cfg(feature = "sqlcipher")]
impl StorageProvider for FileStorage {
    fn read_meta(&self) -> Option<Vec<u8>> {
        std::fs::read(format!("{}/keys.bin", self.dir)).ok()
    }
    fn write_meta(&self, bytes: &[u8]) {
        let tmp = format!("{}/keys.bin.tmp", self.dir);
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, format!("{}/keys.bin", self.dir));
        }
    }
    fn open_relay(&self, key: &[u8; 32], _create: bool) -> Result<Box<dyn Kv>, StorageError> {
        Ok(Box::new(mesh_storage::sqlcipher::SqlCipherKv::open(
            &format!("{}/relay.db", self.dir),
            key,
        )?))
    }
    fn open_message(&self, key: &[u8; 32], _create: bool) -> Result<Box<dyn Kv>, StorageError> {
        Ok(Box::new(mesh_storage::sqlcipher::SqlCipherKv::open(
            &format!("{}/message.db", self.dir),
            key,
        )?))
    }
    fn wipe_all(&self) {
        mesh_storage::sqlcipher::wipe(&format!("{}/relay.db", self.dir));
        mesh_storage::sqlcipher::wipe(&format!("{}/message.db", self.dir));
        let _ = std::fs::remove_file(format!("{}/keys.bin", self.dir));
    }
}

// ===== Software keystore (simulator / browser harness only) =====

/// Keys live in process memory. This is NOT a hardware keystore; it exists so
/// the simulator and the browser harness exercise the same unlock logic.
pub struct SoftwareKeystore {
    inner: Mutex<SoftKs>,
}

struct SoftKs {
    keys: std::collections::HashMap<KeySlot, [u8; 32]>,
    counter: u32,
    biometric: Option<Vec<u8>>,
}

impl Default for SoftwareKeystore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SoftKs {
                keys: Default::default(),
                counter: 0,
                biometric: None,
            }),
        }
    }
}

impl SoftwareKeystore {
    fn key(&self, slot: KeySlot) -> [u8; 32] {
        *self
            .inner
            .lock()
            .unwrap()
            .keys
            .entry(slot)
            .or_insert_with(mesh_crypto::random_array)
    }
}

impl Keystore for SoftwareKeystore {
    fn wrap(&self, slot: KeySlot, plaintext: Vec<u8>) -> Result<Vec<u8>, KeystoreError> {
        let k = mesh_crypto::SymKey(self.key(slot));
        let nonce: [u8; 16] = mesh_crypto::random_array();
        let mut out = nonce.to_vec();
        out.extend(mesh_crypto::seal(&k, &nonce, b"wrap", &plaintext));
        Ok(out)
    }
    fn unwrap(&self, slot: KeySlot, wrapped: Vec<u8>) -> Result<Vec<u8>, KeystoreError> {
        if wrapped.len() < 32 {
            return Err(KeystoreError::Unavailable);
        }
        let k = mesh_crypto::SymKey(self.key(slot));
        let nonce: [u8; 16] = wrapped[..16].try_into().unwrap();
        mesh_crypto::open(&k, &nonce, b"wrap", &wrapped[16..])
            .map_err(|_| KeystoreError::Unavailable)
    }
    fn hmac(&self, slot: KeySlot, data: Vec<u8>) -> Result<Vec<u8>, KeystoreError> {
        Ok(mesh_crypto::hmac_sha256(&self.key(slot), &[&data]).to_vec())
    }
    fn store_biometric(&self, secret: Vec<u8>) -> Result<(), KeystoreError> {
        self.inner.lock().unwrap().biometric = Some(secret);
        Ok(())
    }
    fn load_biometric(&self) -> Result<Vec<u8>, KeystoreError> {
        self.inner
            .lock()
            .unwrap()
            .biometric
            .clone()
            .ok_or(KeystoreError::Unavailable)
    }
    fn counter_get(&self) -> u32 {
        self.inner.lock().unwrap().counter
    }
    fn counter_set(&self, v: u32) {
        self.inner.lock().unwrap().counter = v;
    }
    fn destroy_all(&self) {
        let mut s = self.inner.lock().unwrap();
        s.keys.clear();
        s.biometric = None;
        s.counter = 0;
    }
}

/// Collects events (tests, simulator).
#[derive(Default)]
pub struct VecSink(pub Mutex<Vec<Event>>);

impl EventSink for VecSink {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

impl VecSink {
    pub fn drain(&self) -> Vec<Event> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}
