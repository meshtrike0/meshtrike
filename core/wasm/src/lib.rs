//! Browser binding (D28): the same engine compiled to WebAssembly for the
//! multi-phone browser harness. Storage and keystore are in-memory software
//! implementations; radio commands and UI events are buffered and drained by
//! JavaScript after each call, so there are no re-entrant callbacks.
#![forbid(unsafe_code)]

use mesh_api::*;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

/// Wall clock plus an adjustable offset (the harness can fast-forward time).
#[derive(Default)]
struct JsClock {
    offset_ms: AtomicI64,
}

impl Clock for JsClock {
    fn now_ms(&self) -> u64 {
        (js_sys::Date::now() as i64 + self.offset_ms.load(Ordering::Relaxed)) as u64
    }
}

#[derive(Default)]
struct BufferedTransport {
    cmds: Mutex<Vec<serde_json::Value>>,
    caps: Mutex<Capabilities>,
    /// The harness's simulated internet route is connected.
    net_online: Mutex<bool>,
}

fn hex(u: &[u8; 16]) -> String {
    u.iter().map(|b| format!("{b:02x}")).collect()
}

impl BufferedTransport {
    fn push(&self, v: serde_json::Value) {
        self.cmds.lock().unwrap().push(v);
    }
}

impl TransportHost for BufferedTransport {
    fn start_advertising(&self, uuids: Vec<[u8; 16]>, beacon: Vec<u8>, power: TxPower) {
        self.push(serde_json::json!({"cmd": "advertise", "uuids": uuids.iter().map(hex).collect::<Vec<_>>(), "beacon_len": beacon.len(), "power": power}));
    }
    fn stop_advertising(&self) {
        self.push(serde_json::json!({"cmd": "stop_advertise"}));
    }
    fn start_scanning(&self, uuids: Vec<[u8; 16]>) {
        self.push(
            serde_json::json!({"cmd": "scan", "uuids": uuids.iter().map(hex).collect::<Vec<_>>()}),
        );
    }
    fn stop_scanning(&self) {
        self.push(serde_json::json!({"cmd": "stop_scan"}));
    }
    fn connect(&self, p: PeerHandle, t: TransportKind) {
        self.push(serde_json::json!({"cmd": "connect", "peer": p, "transport": t}));
    }
    fn send_frame(&self, p: PeerHandle, bytes: Vec<u8>) -> bool {
        self.push(serde_json::json!({"cmd": "frame", "peer": p, "bytes": bytes}));
        true
    }
    fn disconnect(&self, p: PeerHandle) {
        self.push(serde_json::json!({"cmd": "disconnect", "peer": p}));
    }
    fn capabilities(&self) -> Capabilities {
        *self.caps.lock().unwrap()
    }
    fn lora_send(&self, bytes: Vec<u8>) -> bool {
        let ok = self.caps.lock().unwrap().lora_paired;
        if ok {
            self.push(serde_json::json!({"cmd": "lora", "bytes": bytes}));
        }
        ok
    }
    fn internet_config(&self, enabled: bool, config_json: String) {
        self.push(
            serde_json::json!({"cmd": "net_config", "enabled": enabled, "config": config_json}),
        );
    }
    fn internet_send(&self, address: String, envelope: Vec<u8>) -> bool {
        let ok = *self.net_online.lock().unwrap();
        if ok {
            self.push(
                serde_json::json!({"cmd": "net_send", "address": address, "bytes": envelope}),
            );
        }
        ok
    }
}

#[derive(Default)]
struct BufferedEvents(Mutex<Vec<Event>>);

impl EventSink for BufferedEvents {
    fn emit(&self, e: Event) {
        self.0.lock().unwrap().push(e);
    }
}

#[wasm_bindgen]
pub struct WasmCore {
    engine: Engine,
    tx: Arc<BufferedTransport>,
    ev: Arc<BufferedEvents>,
    clock: Arc<JsClock>,
    storage: Arc<MemoryStorage>,
}

#[wasm_bindgen]
impl WasmCore {
    /// `config_json`: {"fast_kdf": bool, "lora": bool, "wifi": bool}
    #[wasm_bindgen(constructor)]
    pub fn new(config_json: &str) -> Result<WasmCore, JsValue> {
        console_hook();
        let cfg: serde_json::Value = serde_json::from_str(config_json).unwrap_or_default();
        let mut core_cfg = CoreConfig::default();
        if cfg
            .get("fast_kdf")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            core_cfg.argon2_m_kib = 1024;
            core_cfg.argon2_t = 1;
        }
        if let Some(b) = cfg.get("pow_bits").and_then(|v| v.as_u64()) {
            core_cfg.pow_bits_override = Some(b as u32);
        }
        let tx = Arc::new(BufferedTransport::default());
        *tx.caps.lock().unwrap() = Capabilities {
            wifi_p2p: cfg.get("wifi").and_then(|v| v.as_bool()).unwrap_or(false),
            lora_paired: cfg.get("lora").and_then(|v| v.as_bool()).unwrap_or(false),
            tx_power_control: true,
        };
        let ev = Arc::new(BufferedEvents::default());
        let clock = Arc::new(JsClock::default());
        let storage = Arc::new(MemoryStorage::default());
        let host = Host {
            tx: tx.clone(),
            ks: Arc::new(SoftwareKeystore::default()),
            ev: ev.clone(),
            clock: clock.clone(),
            storage: storage.clone(),
        };
        let engine = Engine::open(core_cfg, host).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(WasmCore {
            engine,
            tx,
            ev,
            clock,
            storage,
        })
    }

    /// JSON method call; returns {"ok": ...} or {"err": ...}.
    pub fn call(&mut self, method: &str, args_json: &str) -> String {
        mesh_api::rpc::call_json(&mut self.engine, method, args_json)
    }

    /// Fast path for radio frames (no base64).
    pub fn on_frame(&mut self, peer: f64, bytes: &[u8]) {
        self.engine.on_frame(peer as PeerHandle, bytes.to_vec());
    }

    pub fn tick(&mut self) {
        self.engine.tick();
    }

    /// Radio commands since the last drain (JSON array).
    pub fn drain_commands(&self) -> String {
        let v = std::mem::take(&mut *self.tx.cmds.lock().unwrap());
        serde_json::Value::Array(v).to_string()
    }

    /// UI events since the last drain (JSON array).
    pub fn drain_events(&self) -> String {
        let v = std::mem::take(&mut *self.ev.0.lock().unwrap());
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }

    /// Fast-forward this phone's clock (harness time travel).
    pub fn advance_clock(&self, ms: f64) {
        self.clock.offset_ms.fetch_add(ms as i64, Ordering::Relaxed);
    }

    pub fn set_lora(&self, on: bool) {
        self.tx.caps.lock().unwrap().lora_paired = on;
    }

    /// The harness's simulated internet route came up or went down.
    pub fn set_net_online(&self, on: bool) {
        *self.tx.net_online.lock().unwrap() = on;
    }

    /// True if no identity is stored (fresh install).
    pub fn is_fresh(&self) -> bool {
        self.storage.read_meta().is_none()
    }

    pub fn debug_state(&self) -> String {
        self.engine.debug_state()
    }
}

fn console_hook() {
    std::panic::set_hook(Box::new(|info| {
        web_sys_log(&format!("mesh-core panic: {info}"));
    }));
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn web_sys_log(s: &str);
}
