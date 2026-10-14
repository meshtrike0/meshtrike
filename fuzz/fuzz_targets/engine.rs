#![no_main]
//! Random envelopes into an unlocked engine with a contact, a group and a
//! channel: exercises tag lookup, relay storage, acks and decryption paths.
use libfuzzer_sys::fuzz_target;
use mesh_api::*;
use std::sync::{Arc, Mutex, OnceLock};

struct NullTx;
impl TransportHost for NullTx {
    fn start_advertising(&self, _: Vec<[u8; 16]>, _: Vec<u8>, _: TxPower) {}
    fn stop_advertising(&self) {}
    fn start_scanning(&self, _: Vec<[u8; 16]>) {}
    fn stop_scanning(&self) {}
    fn connect(&self, _: PeerHandle, _: TransportKind) {}
    fn send_frame(&self, _: PeerHandle, _: Vec<u8>) -> bool { true }
    fn disconnect(&self, _: PeerHandle) {}
    fn capabilities(&self) -> Capabilities { Capabilities::default() }
    fn lora_send(&self, _: Vec<u8>) -> bool { false }
}

fn engine() -> &'static Mutex<Engine> {
    static E: OnceLock<Mutex<Engine>> = OnceLock::new();
    E.get_or_init(|| {
        let host = Host {
            tx: Arc::new(NullTx),
            ks: Arc::new(SoftwareKeystore::default()),
            ev: Arc::new(VecSink::default()),
            clock: Arc::new(SystemClock),
            storage: Arc::new(MemoryStorage::default()),
        };
        let cfg = CoreConfig { argon2_m_kib: 64, argon2_t: 1, rng_seed: Some(1), max_links: 8, pow_bits_override: Some(0) };
        let mut e = Engine::open(cfg, host).unwrap();
        e.create_identity("fuzz".into(), "123456".into(), None).unwrap();
        e.join_channel("plaza".into()).unwrap();
        e.on_connected(1, 512, false);
        Mutex::new(e)
    })
}

fuzz_target!(|data: &[u8]| {
    let mut e = engine().lock().unwrap();
    e.ingest_raw(data);
    e.on_frame(1, data.to_vec());
    e.on_lora_packet(data.to_vec());
    let _ = mesh_api::rpc::call(&mut e, "handoff_push_frame", &serde_json::json!({}));
});
