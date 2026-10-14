//! Test tool, not shipped: gives the Mac browser test node (app/src/mac) a
//! real internet route. The page connects to ws://127.0.0.1:47900 and
//! exchanges JSON text messages:
//!
//!   page -> helper  {"t":"config","enabled":bool,"bridges":[...],"allow_direct":bool}
//!                   {"t":"send","address":"...","bytes":"<base64>"}
//!   helper -> page  {"t":"ready","address":"..."} | {"t":"down","reason":"..."}
//!                   {"t":"env","bytes":"<base64>"}
//!
//!   mac_helper <dir> [--port N] [<socks-addr> <user> <pass>]
//!
//! The browser test page (harness) can run one helper per simulated phone on
//! ports 47900, 47901, ... to put each on the real mixnet.
//!
//! With a SOCKS address, everything goes through that proxy (e.g. a local
//! Xray client with a REALITY outbound), like the Android add-on.

use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use meshtrike_internet::{set_network_policy, Event, Node, Proxy};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    let mut port: u16 = 47900;
    if let Some(i) = args.iter().position(|a| a == "--port") {
        port = args[i + 1].parse().expect("port");
        args.drain(i..i + 2);
    }
    let dir = std::path::PathBuf::from(&args[1]);
    std::fs::create_dir_all(&dir).unwrap();
    let proxy = args.get(2).map(|a| Proxy {
        addr: a.parse().expect("proxy addr"),
        user: args[3].clone(),
        pass: args[4].clone(),
    });
    println!("mac_helper: proxy {}", if proxy.is_some() { "on" } else { "off" });
    set_network_policy(proxy);
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind helper port");
    println!("mac_helper: ws://127.0.0.1:{port}");
    let node: Arc<Mutex<Option<Node>>> = Arc::new(Mutex::new(None));
    let address: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    // The page side that receives events; replaced when the page reconnects.
    let page: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>> = Arc::new(Mutex::new(None));
    loop {
        let Ok((sock, _)) = listener.accept().await else { continue };
        let Ok(ws) = tokio_tungstenite::accept_async(sock).await else { continue };
        println!("page connected");
        let (mut tx, mut rx) = ws.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        let writer = tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                if tx.send(Message::Text(m)).await.is_err() {
                    break;
                }
            }
        });
        *page.lock().unwrap() = Some(out_tx.clone());
        if let Some(a) = address.lock().unwrap().clone() {
            let _ = out_tx.send(serde_json::json!({"t": "ready", "address": a}).to_string());
        }
        while let Some(Ok(msg)) = rx.next().await {
            let Message::Text(t) = msg else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
            match v["t"].as_str() {
                Some("config") => {
                    let on = v["enabled"].as_bool().unwrap_or(false);
                    let mut n = node.lock().unwrap();
                    if !on {
                        if let Some(x) = n.take() {
                            x.stop();
                        }
                        let _ = out_tx.send(serde_json::json!({"t": "down", "reason": "off"}).to_string());
                    } else if n.is_none() {
                        let out = page.clone();
                        let addr = address.clone();
                        *n = Node::start(
                            dir.clone(),
                            Arc::new(move |e| {
                                let m = match e {
                                    Event::Ready(a) => {
                                        println!("ready");
                                        *addr.lock().unwrap() = Some(a.clone());
                                        serde_json::json!({"t": "ready", "address": a})
                                    }
                                    Event::Down(r) => {
                                        println!("down: {r}");
                                        serde_json::json!({"t": "down", "reason": r})
                                    }
                                    Event::Envelope(b) => {
                                        println!("envelope in ({} bytes)", b.len());
                                        serde_json::json!({"t": "env", "bytes": base64::engine::general_purpose::STANDARD.encode(b)})
                                    }
                                    Event::Gateway(g) => {
                                        println!("gateway {g}");
                                        return;
                                    }
                                };
                                if let Some(o) = out.lock().unwrap().as_ref() {
                                    let _ = o.send(m.to_string());
                                }
                            }),
                        )
                        .ok();
                    }
                }
                Some("send") => {
                    let (Some(a), Some(b)) = (v["address"].as_str(), v["bytes"].as_str()) else { continue };
                    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b) else { continue };
                    let ok = node.lock().unwrap().as_ref().is_some_and(|n| n.send(a, &bytes));
                    println!("envelope out ({} bytes): {}", bytes.len(), if ok { "queued" } else { "refused" });
                }
                _ => {}
            }
        }
        println!("page left");
        writer.abort();
    }
}
