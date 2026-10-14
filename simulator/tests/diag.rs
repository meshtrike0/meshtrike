use mesh_api::*;
use mesh_simulator::*;
#[test]
fn diag_wifi() {
    let mut w = World::new(1_790_000_000_000, 11);
    let a = w.add_user_wifi("alice", (10.0, 10.0));
    let b = w.add_user_wifi("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let file: Vec<u8> = (0..600_000u32).map(|i| (i * 31 % 251) as u8).collect();
    w.nodes[a]
        .engine
        .send_file(conv_a, "map.bin".into(), file, SendOptions::default())
        .unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    for i in 0..12 {
        w.run_ms(10_000);
        let m = w.nodes[b].engine.messages(conv_b, None, 10).unwrap();
        let f = m
            .iter()
            .find(|m| m.kind == MessageKind::File)
            .map(|m| (m.chunks_have, m.chunks_total));
        println!(
            "t={} wifi_frames={} wifi_links={:?} ble={:?} chunks={:?}",
            i * 10,
            w.wifi_frames,
            w.wifi_links,
            w.connected,
            f
        );
    }
    println!(
        "A STATE:\n{}",
        w.nodes[a]
            .engine
            .debug_state()
            .lines()
            .filter(|l| !l.starts_with("env Ack"))
            .take(12)
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!("A LOG:\n{}", w.nodes[a].engine.debug_log());
    println!(
        "B LOG:\n{}",
        w.nodes[b]
            .engine
            .debug_log()
            .lines()
            .filter(|l| !l.contains("stored direct"))
            .take(25)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The Android host reports Wi-Fi Direct links through the JSON RPC with
/// `"transport":"WifiP2p"`; the core must record the link kind from it.
#[test]
fn rpc_on_connected_carries_transport_kind() {
    let mut w = World::new(1_790_000_000_000, 12);
    let a = w.add_user("alice", (10.0, 10.0));
    let e = &mut w.nodes[a].engine;
    let r = mesh_api::rpc::call_json(
        e,
        "on_connected",
        r#"{"peer":1099511627776,"mtu":16384,"outgoing":true,"transport":"WifiP2p"}"#,
    );
    assert!(!r.contains("err"), "{r}");
    assert!(e
        .network_status()
        .transports
        .contains(&TransportKind::WifiP2p));
    // No transport field: BLE, as before.
    mesh_api::rpc::call_json(e, "on_connected", r#"{"peer":7,"mtu":185,"outgoing":true}"#);
    assert!(e.network_status().transports.contains(&TransportKind::Ble));
}
