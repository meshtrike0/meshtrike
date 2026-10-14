//! Meshtrike Internet add-on, native part: a Nym mixnet client that carries
//! Meshtrike's sealed envelopes between phones that are far apart.
//!
//! - Envelopes are already end-to-end sealed by Meshtrike. This code never
//!   sees plaintext, names or keys; it moves opaque bytes to Nym addresses.
//! - All network traffic of this process can be forced through one local
//!   SOCKS5 endpoint (the add-on's Xray client: VLESS + REALITY to a bridge):
//!   the gateway WebSocket (patched nym-gateway-client), every Nym HTTP API
//!   client (a registry hook plus ALL_PROXY), with DNS resolved at the bridge.
//! - Messages go out without reply blocks (no SURBs): replies and acks are
//!   ordinary envelopes to the sender's own address, which contacts already
//!   hold. Nothing here can reveal the sender's address to the recipient.
//! - Nym statistics reporting is switched off.

use futures::StreamExt;
use nym_sdk::mixnet::{
    self, IncludedSurbs, MixnetClient, MixnetClientSender, MixnetMessageSender, Recipient,
    StoragePaths,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// Marks a Meshtrike envelope inside a Nym message (anyone who knows a Nym
/// address can send to it; other payloads are dropped here).
const MAGIC: &[u8; 3] = b"MT\x01";
/// Largest envelope accepted either way.
pub const MAX_ENVELOPE: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);

/// The local SOCKS5 endpoint every connection must use (None: direct).
#[derive(Clone, Debug)]
pub struct Proxy {
    pub addr: SocketAddr,
    pub user: String,
    pub pass: String,
}

impl Proxy {
    fn url(&self) -> String {
        format!("socks5h://{}:{}@{}", self.user, self.pass, self.addr)
    }
}

pub enum Event {
    /// Connected; our address on the mixnet.
    Ready(String),
    /// Not connected (reason for the status line; no identifiers).
    Down(String),
    /// A Meshtrike envelope arrived.
    Envelope(Vec<u8>),
    /// Right after Ready: the gateway this client is connected to
    /// ("host:port"; behind a bridge, the address the bridge was asked to
    /// reach). For opt-in tracing.
    Gateway(String),
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

// ===== Process-wide network policy =====

static HTTP_PROXY: RwLock<Option<String>> = RwLock::new(None);

fn tls_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("rustls protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// Applied last to every Nym HTTP client: built-in web PKI roots (no
/// platform verifier, which needs a JVM on Android) and the SOCKS proxy.
fn apply_http_policy(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let b = b.use_preconfigured_tls(tls_config());
    match HTTP_PROXY.read().unwrap_or_else(|p| p.into_inner()).clone() {
        Some(url) => match reqwest::Proxy::all(url) {
            Ok(p) => b.proxy(p),
            // A proxy that cannot be expressed must not fall back to direct.
            Err(_) => b.proxy(reqwest::Proxy::all("socks5h://127.0.0.1:9").expect("static proxy url")),
        },
        None => b.no_proxy(),
    }
}

nym_http_api_client::inventory::submit! {
    nym_http_api_client::registry::ConfigRecord { priority: i32::MAX, apply: apply_http_policy }
}

/// Must run before the first Nym call in this process: the Nym HTTP client is
/// built once and keeps whatever proxy it saw then.
pub fn set_network_policy(proxy: Option<Proxy>) {
    *HTTP_PROXY.write().unwrap_or_else(|p| p.into_inner()) = proxy.as_ref().map(Proxy::url);
    // Plain reqwest clients elsewhere in Nym honour ALL_PROXY.
    match &proxy {
        Some(p) => {
            std::env::set_var("ALL_PROXY", p.url());
            std::env::set_var("all_proxy", p.url());
            std::env::remove_var("NO_PROXY");
            std::env::remove_var("no_proxy");
        }
        None => {
            std::env::remove_var("ALL_PROXY");
            std::env::remove_var("all_proxy");
        }
    }
    nym_gateway_client::client::socks::set_socks5_proxy(proxy.map(|p| {
        nym_gateway_client::client::socks::Socks5 {
            addr: p.addr,
            user: p.user,
            pass: p.pass,
        }
    }));
}

// ===== Node =====

enum Cmd {
    Send(Recipient, Vec<u8>),
}

/// One running client. Dropping it (or `stop`) disconnects.
pub struct Node {
    rt: Option<tokio::runtime::Runtime>,
    cmds: mpsc::UnboundedSender<Cmd>,
    stop: Arc<tokio::sync::Notify>,
    done: Arc<Mutex<bool>>,
}

impl Node {
    /// Connect (keys and gateway kept in `dir`, so the address is stable) and
    /// report through `sink`. Keeps reconnecting until stopped.
    pub fn start(dir: PathBuf, sink: Sink) -> std::io::Result<Node> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("meshtrike-net")
            .enable_all()
            .build()?;
        let (tx, rx) = mpsc::unbounded_channel();
        let stop = Arc::new(tokio::sync::Notify::new());
        let done = Arc::new(Mutex::new(false));
        rt.spawn(run(dir, sink, rx, stop.clone(), done.clone()));
        Ok(Node {
            rt: Some(rt),
            cmds: tx,
            stop,
            done,
        })
    }

    /// Queue an envelope for a Nym address. False if the address is invalid
    /// or the envelope too large.
    pub fn send(&self, address: &str, envelope: &[u8]) -> bool {
        if envelope.len() > MAX_ENVELOPE {
            return false;
        }
        let Ok(r) = address.parse::<Recipient>() else {
            return false;
        };
        let mut m = Vec::with_capacity(MAGIC.len() + envelope.len());
        m.extend_from_slice(MAGIC);
        m.extend_from_slice(envelope);
        self.cmds.send(Cmd::Send(r, m)).is_ok()
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        *self.done.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.stop.notify_waiters();
        self.stop.notify_one();
        if let Some(rt) = self.rt.take() {
            // Blocking shutdown panics inside another runtime (an async
            // caller); wait for it on a plain thread then.
            if tokio::runtime::Handle::try_current().is_ok() {
                std::thread::spawn(move || rt.shutdown_timeout(Duration::from_secs(5)));
            } else {
                rt.shutdown_timeout(Duration::from_secs(5));
            }
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn connect(dir: &PathBuf) -> Result<MixnetClient, String> {
    let paths = StoragePaths::new_from_dir(dir).map_err(|e| e.to_string())?;
    let stats = nym_client_core::config::StatsReporting {
        enabled: false,
        provider_address: None,
        ..Default::default()
    };
    let builder = mixnet::MixnetClientBuilder::new_with_default_storage(paths)
        .await
        .map_err(|e| e.to_string())?
        .with_statistics_reporting(stats)
        .build()
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(CONNECT_TIMEOUT, builder.connect_to_mixnet())
        .await
        .map_err(|_| "timed out connecting to the mixnet".to_string())?
        .map_err(|e| e.to_string())
}

async fn run(
    dir: PathBuf,
    sink: Sink,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    stop: Arc<tokio::sync::Notify>,
    done: Arc<Mutex<bool>>,
) {
    let stopped = || *done.lock().unwrap_or_else(|p| p.into_inner());
    let mut backoff = Duration::from_secs(5);
    while !stopped() {
        let client = tokio::select! {
            c = connect(&dir) => c,
            _ = stop.notified() => return,
        };
        let mut client = match client {
            Ok(c) => c,
            Err(e) => {
                sink(Event::Down(short_reason(&e)));
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = stop.notified() => return,
                }
                backoff = (backoff * 2).min(Duration::from_secs(120));
                continue;
            }
        };
        backoff = Duration::from_secs(5);
        sink(Event::Ready(client.nym_address().to_string()));
        if let Some(g) = gateway_peer(&client) {
            sink(Event::Gateway(g));
        }
        let sender: MixnetClientSender = client.split_sender();
        loop {
            tokio::select! {
                _ = stop.notified() => {
                    client.disconnect().await;
                    return;
                }
                cmd = cmds.recv() => match cmd {
                    Some(Cmd::Send(to, bytes)) => {
                        if sender.send_message(to, bytes, IncludedSurbs::Amount(0)).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        client.disconnect().await;
                        return;
                    }
                },
                msg = client.next() => match msg {
                    Some(m) => {
                        let b = m.message;
                        if b.len() > MAGIC.len() && b.len() <= MAGIC.len() + MAX_ENVELOPE && b.starts_with(MAGIC) {
                            sink(Event::Envelope(b[MAGIC.len()..].to_vec()));
                        }
                    }
                    // The client's stream ended: reconnect.
                    None => break,
                },
            }
        }
        client.disconnect().await;
        sink(Event::Down("connection lost".into()));
    }
}

/// The gateway this client talks to: behind a bridge, the target the proxy
/// was asked to reach; direct, the far end of the gateway WebSocket.
fn gateway_peer(client: &MixnetClient) -> Option<String> {
    if HTTP_PROXY.read().unwrap_or_else(|p| p.into_inner()).is_some() {
        return nym_gateway_client::client::socks::last_socks5_target();
    }
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        let fd = client.gateway_connection().gateway_ws_fd?;
        // Borrow the socket without taking ownership: never close it here.
        let s = std::mem::ManuallyDrop::new(unsafe { std::net::TcpStream::from_raw_fd(fd) });
        s.peer_addr().ok().map(|a| a.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = client;
        None
    }
}

/// A status line without addresses or hostnames.
fn short_reason(e: &str) -> String {
    let e = e.to_lowercase();
    if e.contains("timed out") || e.contains("timeout") {
        "timed out".into()
    } else if e.contains("socks") || e.contains("proxy") {
        "bridge unreachable".into()
    } else if e.contains("gateway") {
        "no gateway reachable".into()
    } else {
        "network unreachable".into()
    }
}

#[cfg(target_os = "android")]
mod android;
