//! Desktop check for the add-on's network code (not shipped): two clients,
//! one message each way, through an optional SOCKS5 proxy.
//!
//!   nettest <dir> [<proxy-addr> <user> <pass>]
//!
//! Run it under `lsof` to confirm every TCP connection goes to the proxy.

use meshtrike_internet::{set_network_policy, Event, Node, Proxy};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::PathBuf::from(&args[1]);
    let proxy = args.get(2).map(|a| Proxy {
        addr: a.parse().expect("proxy addr"),
        user: args[3].clone(),
        pass: args[4].clone(),
    });
    println!("proxy: {}", if proxy.is_some() { "on" } else { "off" });
    set_network_policy(proxy);
    let (tx, rx) = mpsc::channel::<(&'static str, Event)>();
    let mk = |name: &'static str| {
        let tx = tx.clone();
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        Node::start(d, Arc::new(move |e| {
            let _ = tx.send((name, e));
        }))
        .unwrap()
    };
    let t0 = Instant::now();
    let alice = mk("alice");
    let bob = mk("bob");
    let (mut a_addr, mut b_addr) = (None, None);
    while a_addr.is_none() || b_addr.is_none() {
        match rx.recv_timeout(Duration::from_secs(180)).expect("no ready within 180 s") {
            ("alice", Event::Ready(a)) => a_addr = Some(a),
            ("bob", Event::Ready(a)) => b_addr = Some(a),
            (n, Event::Down(r)) => println!("{n} down: {r}"),
            _ => {}
        }
    }
    println!("both connected in {:?}", t0.elapsed());
    let t1 = Instant::now();
    assert!(alice.send(b_addr.as_ref().unwrap(), b"envelope from alice"));
    loop {
        if let ("bob", Event::Envelope(b)) = rx.recv_timeout(Duration::from_secs(60)).expect("bob got nothing") {
            assert_eq!(b, b"envelope from alice");
            break;
        }
    }
    println!("alice -> bob in {:?}", t1.elapsed());
    let t2 = Instant::now();
    assert!(bob.send(a_addr.as_ref().unwrap(), b"reply from bob"));
    loop {
        if let ("alice", Event::Envelope(b)) = rx.recv_timeout(Duration::from_secs(60)).expect("alice got nothing") {
            assert_eq!(b, b"reply from bob");
            break;
        }
    }
    println!("bob -> alice in {:?}", t2.elapsed());
    alice.stop();
    bob.stop();
    println!("PASS");
}
