//! Internet route (amendment N1): the mesh's sealed envelopes also travel
//! over a simulated mixnet with a gateway that holds mail for offline nodes.

use mesh_api::*;
use mesh_simulator::*;

const T0: u64 = 1_790_000_000_000;
const FAR: (f64, f64) = (5_000.0, 5_000.0);
const BRIDGE: &str = "vless://3f1c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6@203.0.113.7:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw&sid=6ba85179e30d4fc2&type=tcp#b1";

fn texts(w: &mut World, node: usize, conv: Id16) -> Vec<Message> {
    w.nodes[node].engine.messages(conv, None, 200).unwrap()
}

fn got(w: &mut World, node: usize, conv: Id16, text: &str) -> Option<Message> {
    texts(w, node, conv).into_iter().find(|m| m.text == text)
}

/// Two contacts paired in person, internet on for both, addresses exchanged
/// over the mesh, then moved apart.
fn paired_apart() -> (World, usize, usize, Id16, Id16) {
    let mut w = World::new(T0, 21);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.nodes[a].engine.set_internet(true);
    w.nodes[b].engine.set_internet(true);
    let (ka, kb) = (w.identity_key(a), w.identity_key(b));
    assert!(
        w.run_until(180_000, |w| {
            w.contact_by_key(a, &kb).is_some_and(|c| c.internet)
                && w.contact_by_key(b, &ka).is_some_and(|c| c.internet)
        }),
        "addresses not exchanged over the mesh"
    );
    let ca = w.conv_with(a, b).unwrap();
    let cb = w.conv_with(b, a).unwrap();
    w.nodes[b].pos = FAR;
    w.run_ms(20_000);
    assert!(
        w.contact_by_key(a, &kb).is_some_and(|c| !c.nearby),
        "still in Bluetooth range"
    );
    (w, a, b, ca, cb)
}

#[test]
fn messages_and_acks_cross_the_internet_when_apart() {
    let (mut w, a, b, ca, cb) = paired_apart();
    assert!(w.nodes[a].engine.network_status().internet_online);
    w.nodes[a]
        .engine
        .send_text(ca, "over the mixnet".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(30_000, |w| got(w, b, cb, "over the mixnet").is_some()),
        "not delivered over the internet"
    );
    assert_eq!(
        got(&mut w, b, cb, "over the mixnet").unwrap().via,
        Some(Via::Internet)
    );
    // The ack comes back over the internet too.
    assert!(
        w.run_until(30_000, |w| got(w, a, ca, "over the mixnet").is_some_and(
            |m| matches!(m.state, DeliveryState::Delivered | DeliveryState::Read)
        )),
        "sender never saw it delivered"
    );
    assert_eq!(
        got(&mut w, a, ca, "over the mixnet").unwrap().via,
        Some(Via::Internet),
        "sender records the internet as the carrier"
    );
    // And the reply.
    w.nodes[b]
        .engine
        .send_text(cb, "reply".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(30_000, |w| got(w, a, ca, "reply").is_some()));
}

#[test]
fn gateway_holds_mail_while_offline() {
    let (mut w, a, b, ca, cb) = paired_apart();
    w.set_net_cut(b, true);
    w.nodes[a]
        .engine
        .send_text(ca, "while you were offline".into(), SendOptions::default())
        .unwrap();
    w.run_ms(60_000);
    assert!(got(&mut w, b, cb, "while you were offline").is_none());
    assert!(
        w.net_mailbox.get(&b).is_some_and(|m| !m.is_empty()),
        "gateway kept nothing"
    );
    w.set_net_cut(b, false);
    assert!(
        w.run_until(30_000, |w| got(w, b, cb, "while you were offline")
            .is_some()),
        "mailbox not delivered on reconnect"
    );
}

#[test]
fn sender_offline_queues_then_flushes() {
    let (mut w, a, b, ca, cb) = paired_apart();
    w.set_net_cut(a, true);
    let sent_before = w.net_sent;
    w.nodes[a]
        .engine
        .send_text(ca, "owed".into(), SendOptions::default())
        .unwrap();
    w.run_ms(10_000);
    assert_eq!(w.net_sent, sent_before, "sent while the route was down");
    w.set_net_cut(a, false);
    assert!(
        w.run_until(30_000, |w| got(w, b, cb, "owed").is_some()),
        "owed envelope not sent after reconnect"
    );
}

#[test]
fn both_routes_deliver_once() {
    let mut w = World::new(T0, 22);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.nodes[a].engine.set_internet(true);
    w.nodes[b].engine.set_internet(true);
    let kb = w.identity_key(b);
    assert!(w.run_until(180_000, |w| w
        .contact_by_key(a, &kb)
        .is_some_and(|c| c.internet)));
    let ca = w.conv_with(a, b).unwrap();
    let cb = w.conv_with(b, a).unwrap();
    w.nodes[a]
        .engine
        .send_text(ca, "twice carried".into(), SendOptions::default())
        .unwrap();
    w.run_ms(30_000);
    let copies = texts(&mut w, b, cb)
        .into_iter()
        .filter(|m| m.text == "twice carried")
        .count();
    assert_eq!(copies, 1, "duplicate message from two routes");
}

#[test]
fn groups_reach_far_members_over_the_internet() {
    let mut w = World::new(T0, 23);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    let c = w.add_user("carol", (15.0, 18.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    assert!(w.pair_in_person(c, a));
    for n in [a, b, c] {
        w.nodes[n].engine.set_internet(true);
    }
    let (kb, kc) = (w.identity_key(b), w.identity_key(c));
    assert!(w.run_until(180_000, |w| {
        w.contact_by_key(a, &kb).is_some_and(|x| x.internet)
            && w.contact_by_key(a, &kc).is_some_and(|x| x.internet)
    }));
    let bid = w.contact_by_key(a, &kb).unwrap().id;
    let cid = w.contact_by_key(a, &kc).unwrap().id;
    let g = w.nodes[a]
        .engine
        .create_group("crew".into(), vec![bid, cid])
        .unwrap();
    // Key shares go out while everyone is still near.
    w.run_ms(30_000);
    w.nodes[b].pos = FAR;
    w.run_ms(20_000);
    w.nodes[a]
        .engine
        .send_text(g, "group over internet".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(60_000, |w| got(w, b, g, "group over internet").is_some()),
        "far member did not get the group message"
    );
    assert!(w.run_until(60_000, |w| got(w, c, g, "group over internet").is_some()));
}

#[test]
fn high_risk_never_uses_the_internet() {
    let (mut w, a, _b, ca, _cb) = paired_apart();
    w.nodes[a].engine.set_high_risk(true).unwrap();
    w.run_ms(2_000);
    let st = w.nodes[a].engine.network_status();
    assert!(!st.internet_on && !st.internet_online);
    let before = w.net_sent;
    let _ = w.nodes[a]
        .engine
        .send_text(ca, "stay local".into(), SendOptions::default());
    w.run_ms(20_000);
    assert_eq!(w.net_sent, before, "internet used in High-Risk");
}

#[test]
fn switch_off_stops_the_route() {
    let (mut w, a, _b, ca, _cb) = paired_apart();
    w.nodes[a].engine.set_internet(false);
    w.run_ms(1_000);
    assert!(!w.nodes[a].engine.network_status().internet_online);
    let before = w.net_sent;
    w.nodes[a]
        .engine
        .send_text(ca, "offline only".into(), SendOptions::default())
        .unwrap();
    w.run_ms(20_000);
    assert_eq!(w.net_sent, before);
}

#[test]
fn bridges_validate_persist_and_share_with_verified_contacts() {
    let mut w = World::new(T0, 24);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    let e = &mut w.nodes[a].engine;
    assert!(e
        .add_bridge("vless://x@1.2.3.4:443?security=tls&sni=a&pbk=b")
        .is_err());
    assert!(e.add_bridge("https://example.com").is_err());
    let br = e.add_bridge(BRIDGE).unwrap();
    assert_eq!(
        (br.host.as_str(), br.port, br.disguise.as_str()),
        ("203.0.113.7", 443, "www.microsoft.com")
    );
    // Same link twice is one bridge.
    e.add_bridge(BRIDGE).unwrap();
    assert_eq!(e.bridges().len(), 1);
    assert_eq!(e.network_status().bridges, 1);
    assert_eq!(e.bridge_link(&br.id).unwrap(), BRIDGE);
    // In-person pairing verifies: bob accepts alice's bridges.
    let kb = w.identity_key(b);
    let bid = w.contact_by_key(a, &kb).unwrap().id;
    assert_eq!(w.nodes[a].engine.share_bridges(&bid).unwrap(), 1);
    assert!(
        w.run_until(30_000, |w| w.nodes[b].engine.bridges().len() == 1),
        "verified contact's bridge not received"
    );
    assert!(w.nodes[b].engine.bridges()[0].from_contact);
    // Removing works.
    let id = w.nodes[a].engine.bridges()[0].id.clone();
    w.nodes[a].engine.remove_bridge(&id).unwrap();
    assert!(w.nodes[a].engine.bridges().is_empty());
}

/// Pair in person with only `on` running the route, wait until the other
/// side has stored its address, then move apart.
fn paired_one_on(on_alice: bool) -> (World, usize, usize, Id16, Id16) {
    let mut w = World::new(T0, 22);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    let (on, off) = if on_alice { (a, b) } else { (b, a) };
    w.nodes[on].engine.set_internet(true);
    let _ = off;
    // `Contact.internet` needs the viewer's own route, so give the address
    // time to cross the mesh (shared on connect and every minute).
    w.run_ms(150_000);
    let ca = w.conv_with(a, b).unwrap();
    let cb = w.conv_with(b, a).unwrap();
    w.nodes[b].pos = FAR;
    w.run_ms(20_000);
    (w, a, b, ca, cb)
}

#[test]
fn message_written_while_route_off_goes_once_route_is_on() {
    // Bob runs the route; Alice knows his address but has hers off.
    let (mut w, a, b, ca, cb) = paired_one_on(false);
    w.nodes[a]
        .engine
        .send_text(ca, "written offline".into(), SendOptions::default())
        .unwrap();
    w.run_ms(10_000);
    assert!(got(&mut w, b, cb, "written offline").is_none());
    w.nodes[a].engine.set_internet(true);
    assert!(
        w.run_until(60_000, |w| got(w, b, cb, "written offline").is_some()),
        "queued message never used the internet after the route came on"
    );
    assert_eq!(got(&mut w, b, cb, "written offline").unwrap().via, Some(Via::Internet));
}

#[test]
fn message_waits_for_the_contacts_address() {
    // Alice runs the route but has no address for Bob yet.
    let (mut w, a, b, ca, cb) = paired_one_on(true);
    w.nodes[a]
        .engine
        .send_text(ca, "before your address".into(), SendOptions::default())
        .unwrap();
    w.run_ms(10_000);
    assert!(got(&mut w, b, cb, "before your address").is_none());
    // Bob turns his route on: his address reaches Alice over the internet,
    // and she sends what he is owed straight away.
    w.nodes[b].engine.set_internet(true);
    assert!(
        w.run_until(120_000, |w| got(w, b, cb, "before your address").is_some()),
        "owed message not sent when the address arrived"
    );
}

#[test]
fn high_risk_records_nothing_for_the_internet() {
    let (mut w, a, b, ca, cb) = paired_one_on(false);
    w.nodes[a].engine.set_high_risk(true).unwrap();
    w.nodes[a]
        .engine
        .send_text(ca, "high risk".into(), SendOptions::default())
        .unwrap();
    w.nodes[a].engine.set_high_risk(false).unwrap();
    w.nodes[a].engine.set_internet(true);
    w.run_ms(60_000);
    assert!(
        got(&mut w, b, cb, "high risk").is_none(),
        "a High-Risk message went over the internet later"
    );
}
