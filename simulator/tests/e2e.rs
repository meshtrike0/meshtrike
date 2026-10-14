//! End-to-end scenarios over the simulated radio, using real engines.

use mesh_api::*;
use mesh_simulator::*;

const T0: u64 = 1_790_000_000_000;

fn two_paired() -> (World, usize, usize) {
    let mut w = World::new(T0, 1);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a), "pairing failed");
    (w, a, b)
}

#[test]
fn in_person_pairing_is_verified_and_safety_numbers_match() {
    let (w, a, b) = two_paired();
    let ca = w.contact_by_key(a, &w.identity_key(b)).unwrap();
    let cb = w.contact_by_key(b, &w.identity_key(a)).unwrap();
    assert_eq!(ca.trust, Trust::Verified);
    assert_eq!(cb.trust, Trust::Verified);
    assert_eq!(ca.nickname, "bob");
    let sa = w.nodes[a].engine.safety_number(ca.id).unwrap();
    let sb = w.nodes[b].engine.safety_number(cb.id).unwrap();
    assert_eq!(sa, sb);
}

#[test]
fn direct_message_delivered_and_read() {
    let (mut w, a, b) = two_paired();
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let m = w.nodes[a]
        .engine
        .send_text(conv_a, "hello bob".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(10_000, |w| has_text(w, b, conv_b, "hello bob")),
        "not received"
    );
    assert!(
        w.run_until(10_000, |w| state_of(w, a, conv_a, m)
            == Some(DeliveryState::Delivered)),
        "no ack"
    );
    w.nodes[b].engine.mark_read(conv_b).unwrap();
    assert!(
        w.run_until(10_000, |w| state_of(w, a, conv_a, m)
            == Some(DeliveryState::Read)),
        "no read receipt"
    );
    // And the reply.
    w.nodes[b]
        .engine
        .send_text(conv_b, "hi alice".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(10_000, |w| has_text(w, a, conv_a, "hi alice")));
}

#[test]
fn multi_hop_relay_and_ack_return() {
    let mut w = World::new(T0, 2);
    let a = w.add_user("alice", (0.0, 0.0));
    let b = w.add_user("bob", (10.0, 0.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    // Let the session-init settle, then separate them with a 3-relay chain.
    w.run_ms(5_000);
    w.nodes[b].pos = (100.0, 0.0);
    for x in [25.0, 50.0, 75.0] {
        w.add_user("relay", (x, 0.0));
    }
    w.run_ms(3_000);
    assert!(w.dist(a, b) > w.range_m);
    let m = w.nodes[a]
        .engine
        .send_text(conv_a, "across the chain".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(30_000, |w| has_text(w, b, conv_b, "across the chain")),
        "not relayed"
    );
    assert!(
        w.run_until(30_000, |w| state_of(w, a, conv_a, m)
            == Some(DeliveryState::Delivered)),
        "ack did not return"
    );
    // Relays dropped their copies after the ack.
    let left: u32 = (2..5)
        .map(|r| w.nodes[r].engine.network_status().relay_envelopes)
        .sum();
    assert!(left < 6, "relays still hold {left} envelopes");
}

#[test]
fn store_and_forward_by_data_mule() {
    let mut w = World::new(T0, 3);
    let a = w.add_user("alice", (0.0, 0.0));
    let b = w.add_user("bob", (10.0, 0.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(5_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    w.nodes[b].pos = (180.0, 180.0);
    let mule = w.add_user("mule", (5.0, 5.0));
    w.run_ms(2_000);
    w.nodes[a]
        .engine
        .send_text(conv_a, "carried".into(), SendOptions::default())
        .unwrap();
    w.run_ms(5_000);
    assert!(!has_text(&mut w, b, conv_b, "carried"));
    // The mule walks over to Bob.
    w.nodes[mule].pos = (175.0, 175.0);
    assert!(
        w.run_until(30_000, |w| has_text(w, b, conv_b, "carried")),
        "mule did not deliver"
    );
}

#[test]
fn messages_for_locked_phone_are_acked_and_decrypted_after_unlock() {
    let (mut w, a, b) = two_paired();
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    w.nodes[b].engine.lock();
    let m = w.nodes[a]
        .engine
        .send_text(conv_a, "while locked".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(10_000, |w| state_of(w, a, conv_a, m)
            == Some(DeliveryState::Delivered)),
        "locked phone did not ack"
    );
    assert_eq!(
        w.nodes[b].engine.messages(conv_b, None, 10).err(),
        Some(CoreError::Locked)
    );
    assert_eq!(
        w.nodes[b].engine.unlock("000000".into()).unwrap(),
        UnlockResult::WrongPin { remaining: None }
    );
    assert_eq!(
        w.nodes[b].engine.unlock("123456".into()).unwrap(),
        UnlockResult::Ok
    );
    assert!(has_text(&mut w, b, conv_b, "while locked"));
}

#[test]
fn duress_pin_and_wrong_pin_wipe() {
    let mut w = World::new(T0, 4);
    let a = w.add_node((0.0, 0.0));
    w.nodes[a]
        .engine
        .create_identity("x".into(), "111111".into(), Some("999999".into()))
        .unwrap();
    w.nodes[a].engine.lock();
    assert_eq!(
        w.nodes[a].engine.unlock("999999".into()).unwrap(),
        UnlockResult::Wiped
    );
    assert!(!w.nodes[a].engine.has_identity());

    let b = w.add_node((0.0, 0.0));
    w.nodes[b]
        .engine
        .create_identity("y".into(), "111111".into(), None)
        .unwrap();
    w.nodes[b].engine.lock();
    let mut last = None;
    for _ in 0..10 {
        last = Some(w.nodes[b].engine.unlock("222222".into()).unwrap());
    }
    assert_eq!(last, Some(UnlockResult::Wiped));
    assert!(!w.nodes[b].engine.has_identity());
}

#[test]
fn group_messaging_with_non_contact_members() {
    let mut w = World::new(T0, 5);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (15.0, 10.0));
    let c = w.add_user("carol", (20.0, 10.0));
    w.run_ms(2_000);
    // Alice knows Bob and Carol; Bob and Carol do not know each other.
    assert!(w.pair_in_person(b, a));
    assert!(w.pair_in_person(c, a));
    w.run_ms(5_000);
    let cb = w.contact_by_key(a, &w.identity_key(b)).unwrap().id;
    let cc = w.contact_by_key(a, &w.identity_key(c)).unwrap().id;
    let g = w.nodes[a]
        .engine
        .create_group("trio".into(), vec![cb, cc])
        .unwrap();
    w.nodes[a]
        .engine
        .send_text(g, "hello group".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(30_000, |w| has_text(w, b, g, "hello group")
            && has_text(w, c, g, "hello group")),
        "group msg not received"
    );
    // Bob replies; Carol (not his contact) must read it.
    w.nodes[b]
        .engine
        .send_text(g, "from bob".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(30_000, |w| has_text(w, c, g, "from bob")
            && has_text(w, a, g, "from bob")),
        "member-to-member failed"
    );
    // Remove Carol: she must not read later messages.
    w.nodes[a].engine.remove_member(g, cc).unwrap();
    w.run_ms(10_000);
    w.nodes[a]
        .engine
        .send_text(g, "after removal".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(30_000, |w| has_text(w, b, g, "after removal")));
    w.run_ms(5_000);
    assert!(
        !has_text(&mut w, c, g, "after removal"),
        "removed member still reads"
    );
}

#[test]
fn channel_posts_and_remote_pairing() {
    let mut w = World::new(T0, 6);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (40.0, 10.0));
    let _r = w.add_user("relay", (25.0, 10.0));
    w.run_ms(2_000);
    let ca = w.nodes[a].engine.join_channel("Plaza".into()).unwrap();
    let cb = w.nodes[b].engine.join_channel("plaza".into()).unwrap();
    assert_eq!(ca, cb);
    w.nodes[b]
        .engine
        .send_text(cb, "anyone here?".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(20_000, |w| has_text(w, a, ca, "anyone here?")),
        "post not received"
    );
    let post = w.nodes[a]
        .engine
        .messages(ca, None, 10)
        .unwrap()
        .into_iter()
        .find(|m| m.text == "anyone here?")
        .unwrap();
    assert!(post.can_start_chat);
    assert!(!post.sender.contains("bob"), "pseudonym leaked nickname");
    w.events(b);
    w.nodes[a].engine.pair_from_post(post.id).unwrap();
    let mut req = None;
    assert!(w.run_until(20_000, |w| {
        for e in w.events(b) {
            if let Event::PairingResult {
                pairing,
                outcome: PairingOutcome::Request { .. },
            } = e
            {
                req = Some(pairing);
            }
        }
        req.is_some()
    }));
    w.nodes[b].engine.accept_pairing(req.unwrap()).unwrap();
    let (ka, kb) = (w.identity_key(a), w.identity_key(b));
    assert!(
        w.run_until(30_000, |w| w.contact_by_key(a, &kb).is_some()
            && w.contact_by_key(b, &ka).is_some()),
        "remote pairing failed"
    );
    assert_eq!(w.contact_by_key(a, &kb).unwrap().trust, Trust::Unverified);
    let conv = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    w.nodes[a]
        .engine
        .send_text(conv, "private now".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(20_000, |w| has_text(w, b, conv_b, "private now")));
}

#[test]
fn delayed_send_holds_until_release() {
    let (mut w, a, b) = two_paired();
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let held = w.nodes[a]
        .engine
        .send_text(
            conv_a,
            "held".into(),
            SendOptions {
                when: SendWhen::Hold,
                expiry_secs: 0,
            },
        )
        .unwrap();
    let later = w.nodes[a]
        .engine
        .send_text(
            conv_a,
            "ten minutes".into(),
            SendOptions {
                when: SendWhen::In10Min,
                expiry_secs: 0,
            },
        )
        .unwrap();
    w.run_ms(5_000);
    assert!(!has_text(&mut w, b, conv_b, "held"));
    assert_eq!(state_of(&mut w, a, conv_a, held), Some(DeliveryState::Held));
    assert_eq!(
        state_of(&mut w, a, conv_a, later),
        Some(DeliveryState::Held)
    );
    w.nodes[a].engine.release_held(held).unwrap();
    assert!(w.run_until(10_000, |w| has_text(w, b, conv_b, "held")));
    assert!(w.run_until(11 * 60_000, |w| has_text(w, b, conv_b, "ten minutes")));
}

#[test]
fn image_and_file_transfer() {
    let (mut w, a, b) = two_paired();
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let (wd, ht) = (400u32, 300u32);
    let rgba: Vec<u8> = (0..wd * ht)
        .flat_map(|i| [(i % 251) as u8, (i / 400) as u8, 128, 255])
        .collect();
    w.nodes[a]
        .engine
        .send_image(conv_a, rgba, wd, ht, SendOptions::default())
        .unwrap();
    let file: Vec<u8> = (0..20_000u32).map(|i| (i * 7) as u8).collect();
    w.nodes[a]
        .engine
        .send_file(
            conv_a,
            "../../etc/notes.txt".into(),
            file.clone(),
            SendOptions::default(),
        )
        .unwrap();
    assert!(
        w.run_until(60_000, |w| {
            let m = w.nodes[b].engine.messages(conv_b, None, 10).unwrap();
            m.iter()
                .filter(|m| m.chunks_have == m.chunks_total && m.kind != MessageKind::Text)
                .count()
                == 2
        }),
        "media not complete"
    );
    let msgs = w.nodes[b].engine.messages(conv_b, None, 10).unwrap();
    let img = msgs.iter().find(|m| m.kind == MessageKind::Image).unwrap();
    let px = w.nodes[b].engine.image_pixels(img.id).unwrap();
    assert_eq!((px.width, px.height), (wd, ht));
    let f = msgs.iter().find(|m| m.kind == MessageKind::File).unwrap();
    assert!(
        !f.text.contains('/') && f.text.ends_with("notes.txt"),
        "{}",
        f.text
    );
    let (name, data) = w.nodes[b].engine.file_bytes(f.id).unwrap();
    assert!(!name.contains('/'));
    assert_eq!(data, file);
}

#[test]
fn high_risk_mode_uses_fixed_frames_and_still_delivers() {
    let (mut w, a, b) = two_paired();
    w.run_ms(3_000);
    w.nodes[a].engine.set_high_risk(true).unwrap();
    w.nodes[b].engine.set_high_risk(true).unwrap();
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    // Images are refused in High-Risk mode.
    assert!(w.nodes[a]
        .engine
        .send_image(conv_a, vec![0; 16], 2, 2, SendOptions::default())
        .is_err());
    let before = w.nodes[a].frames_sent;
    w.nodes[a]
        .engine
        .send_text(conv_a, "quiet".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(60_000, |w| has_text(w, b, conv_b, "quiet")),
        "high-risk delivery failed"
    );
    // Cover traffic: frames keep flowing at 3 per second per link with nothing to send.
    let mid = w.nodes[a].frames_sent;
    w.run_ms(10_000);
    let rate = (w.nodes[a].frames_sent - mid) as f64 / 10.0;
    assert!((2.5..=3.5).contains(&rate), "cover rate {rate}/s");
    assert!(mid > before);
}

#[test]
fn listen_only_phone_never_advertises_but_receives() {
    let (mut w, a, b) = two_paired();
    w.run_ms(3_000);
    w.nodes[b]
        .engine
        .set_radio_mode(RadioMode::ListenOnly)
        .unwrap();
    w.run_ms(1_000);
    assert!(w.nodes[b].advertising.is_empty());
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    w.nodes[a]
        .engine
        .send_text(conv_a, "to the listener".into(), SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(6 * 60_000, |w| has_text(w, b, conv_b, "to the listener")),
        "listen-only did not pull"
    );
}

#[test]
fn community_mode_hides_the_public_uuid() {
    let (mut w, a, b) = two_paired();
    let id = w.nodes[a].engine.create_community("north".into()).unwrap();
    let code = w.nodes[a].engine.community_qr(id).unwrap();
    w.nodes[b].engine.join_community(code).unwrap();
    w.nodes[a]
        .engine
        .set_radio_mode(RadioMode::Community)
        .unwrap();
    w.nodes[b]
        .engine
        .set_radio_mode(RadioMode::Community)
        .unwrap();
    w.run_ms(3_000);
    let public = mesh_constants::PUBLIC_SERVICE_UUID;
    assert!(!w.nodes[a].advertising.contains(&public));
    assert_eq!(w.nodes[a].advertising.len(), 1);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    w.nodes[a]
        .engine
        .send_text(conv_a, "community".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(20_000, |w| has_text(w, b, conv_b, "community")));
}

#[test]
fn jamming_is_detected_and_qr_handoff_moves_messages() {
    let (mut w, a, b) = two_paired();
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    // Feed a calm baseline, then a jammed radio.
    for _ in 0..30 {
        w.run_ms(10_000);
        w.nodes[b].engine.on_radio_sample(40, 0, true);
    }
    w.jammers.push((15.0, 10.0, 50.0));
    w.run_ms(2_000);
    w.events(b);
    for _ in 0..8 {
        w.run_ms(10_000);
        w.nodes[b].engine.on_radio_sample(1, 0, true);
    }
    assert!(w
        .events(b)
        .iter()
        .any(|e| matches!(e, Event::Interference { suspected: true })));
    // Alice writes while jammed; the message moves by QR.
    w.nodes[a]
        .engine
        .send_text(conv_a, "by camera".into(), SendOptions::default())
        .unwrap();
    w.run_ms(2_000);
    assert!(!has_text(&mut w, b, conv_b, "by camera"));
    w.nodes[a].engine.handoff_start_send().unwrap();
    w.nodes[b].engine.handoff_start_receive();
    let mut done = false;
    for _ in 0..200 {
        let (frame, qr) = w.nodes[a].engine.handoff_next_frame().unwrap();
        // Go through real QR rendering and decoding every few frames.
        let p = if done {
            break;
        } else if frame.len() % 2 == 0 {
            let (side, luma) = mesh_handoff::render_luma(&qr, 2, 4);
            w.nodes[b]
                .engine
                .handoff_push_luma(side, side, luma)
                .unwrap()
        } else {
            w.nodes[b].engine.handoff_push_frame(frame).unwrap()
        };
        done = p.done;
    }
    assert!(done);
    assert!(has_text(&mut w, b, conv_b, "by camera"));
}

#[test]
fn lora_fallback_after_five_minutes() {
    let mut w = World::new(T0, 9);
    let a = w.add_user_lora("alice", (0.0, 0.0), true);
    let b = w.add_user_lora("bob", (10.0, 0.0), true);
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    // Establish both directions so LoRa can use a normal Olm message.
    w.nodes[b]
        .engine
        .send_text(conv_b, "ping".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(10_000, |w| has_text(w, a, conv_a, "ping")));
    w.nodes[a]
        .engine
        .send_text(conv_a, "pong".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(10_000, |w| has_text(w, b, conv_b, "pong")));
    // 2 km apart: only LoRa reaches.
    w.nodes[b].pos = (2_000.0, 0.0);
    w.area = (3_000.0, 3_000.0);
    w.run_ms(2_000);
    w.nodes[a]
        .engine
        .send_text(conv_a, "over lora".into(), SendOptions::default())
        .unwrap();
    w.run_ms(60_000);
    assert!(!has_text(&mut w, b, conv_b, "over lora"));
    assert!(
        w.run_until(6 * 60_000, |w| has_text(w, b, conv_b, "over lora")),
        "LoRa fallback did not deliver"
    );
}

#[test]
fn large_file_moves_over_wifi_p2p() {
    let mut w = World::new(T0, 11);
    let a = w.add_user_wifi("alice", (10.0, 10.0));
    let b = w.add_user_wifi("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let file: Vec<u8> = (0..600_000u32).map(|i| (i * 31 % 251) as u8).collect();
    w.nodes[a]
        .engine
        .send_file(
            conv_a,
            "map.bin".into(),
            file.clone(),
            SendOptions::default(),
        )
        .unwrap();
    let t0 = w.now();
    assert!(
        w.run_until(120_000, |w| w.nodes[b]
            .engine
            .messages(conv_b, None, 10)
            .unwrap()
            .iter()
            .any(
                |m| m.kind == MessageKind::File && m.chunks_have == m.chunks_total
            )),
        "large file not delivered"
    );
    let secs = (w.now() - t0) as f64 / 1000.0;
    assert!(
        w.wifi_frames > 50,
        "Wi-Fi link not used ({} frames)",
        w.wifi_frames
    );
    let m = w.nodes[b]
        .engine
        .messages(conv_b, None, 10)
        .unwrap()
        .into_iter()
        .find(|m| m.kind == MessageKind::File)
        .unwrap();
    assert_eq!(w.nodes[b].engine.file_bytes(m.id).unwrap().1, file);
    // 600 KB over BLE alone (~8 KB/s paced) would take over a minute.
    assert!(secs < 30.0, "took {secs:.1}s");
}

#[test]
fn group_photo_survives_reordering_and_frame_loss_across_hops() {
    let mut w = World::new(T0, 12);
    let a = w.add_user("alice", (0.0, 0.0));
    let b = w.add_user("bob", (10.0, 0.0));
    let c = w.add_user("carol", (5.0, 5.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    assert!(w.pair_in_person(c, a));
    w.run_ms(3_000);
    let cb = w.contact_by_key(a, &w.identity_key(b)).unwrap().id;
    let cc = w.contact_by_key(a, &w.identity_key(c)).unwrap().id;
    let g = w.nodes[a]
        .engine
        .create_group("photos".into(), vec![cb, cc])
        .unwrap();
    w.nodes[a]
        .engine
        .send_text(g, "warmup".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(30_000, |w| has_text(w, b, g, "warmup")
        && has_text(w, c, g, "warmup")));
    // Spread out: Bob 3 hops away; lossy radio.
    w.nodes[b].pos = (100.0, 0.0);
    w.nodes[c].pos = (5.0, 5.0);
    for x in [25.0, 50.0, 75.0] {
        w.add_user("relay", (x, 0.0));
    }
    w.loss = 0.05;
    w.run_ms(3_000);
    // Noisy pixels make a large JPEG (many chunks).
    let (wd, ht) = (900u32, 700u32);
    let mut x = 7u32;
    let rgba: Vec<u8> = (0..wd * ht)
        .flat_map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            [
                (x & 255) as u8,
                ((x >> 8) & 255) as u8,
                ((x >> 16) & 255) as u8,
                255,
            ]
        })
        .collect();
    w.nodes[a]
        .engine
        .send_image(g, rgba, wd, ht, SendOptions::default())
        .unwrap();
    let complete = |w: &mut World, n: usize| {
        w.nodes[n]
            .engine
            .messages(g, None, 50)
            .unwrap()
            .iter()
            .any(|m| {
                m.kind == MessageKind::Image
                    && m.chunks_have == m.chunks_total
                    && m.chunks_total > 40
            })
    };
    let prog = |w: &mut World, n: usize| {
        w.nodes[n]
            .engine
            .messages(g, None, 50)
            .unwrap()
            .iter()
            .find(|m| m.kind == MessageKind::Image)
            .map(|m| (m.chunks_have, m.chunks_total))
    };
    let ok = w.run_until(15 * 60_000, |w| complete(w, b) && complete(w, c));
    if !ok {
        println!(
            "bob={:?} carol={:?} crypto_b={} crypto_c={}",
            prog(&mut w, b),
            prog(&mut w, c),
            w.nodes[b].engine.network_status().crypto_failures,
            w.nodes[c].engine.network_status().crypto_failures
        );
        for n in 0..w.nodes.len() {
            let st = w.nodes[n].engine.debug_state();
            let small = st
                .lines()
                .filter(|l| l.starts_with("env Group") && l.contains("len=514"))
                .count();
            let big = st
                .lines()
                .filter(|l| l.starts_with("env Group") && l.contains("len=4354"))
                .count();
            let fm = st
                .lines()
                .filter(|l| l.starts_with("env Group") && l.contains("for_me=true"))
                .count();
            println!(
                "node {n}: small_group={small} chunk_group={big} for_me={fm} pos={:?}",
                w.nodes[n].pos
            );
        }
        println!("connected={:?}", w.connected);
        println!(
            "BOB LOG:\n{}",
            w.nodes[b]
                .engine
                .debug_log()
                .lines()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .join("\n")
        );
        println!(
            "NODE5 links:\n{}",
            w.nodes[5]
                .engine
                .debug_state()
                .lines()
                .filter(|l| l.starts_with("link"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        println!(
            "ALICE:\n{}",
            w.nodes[a]
                .engine
                .debug_log()
                .lines()
                .rev()
                .take(14)
                .collect::<Vec<_>>()
                .join("\n")
        );
        println!(
            "ALICE STATE links:\n{}",
            w.nodes[a]
                .engine
                .debug_state()
                .lines()
                .filter(|l| l.starts_with("link"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    assert!(ok, "photo did not reassemble on both members");
    let m = w.nodes[b]
        .engine
        .messages(g, None, 50)
        .unwrap()
        .into_iter()
        .find(|m| m.kind == MessageKind::Image)
        .unwrap();
    let px = w.nodes[b].engine.image_pixels(m.id).unwrap();
    assert!(px.width <= 1600 && px.height <= 1600 && px.width > 0);
}

#[test]
fn carrier_is_recorded_on_both_sides_and_contact_link_is_reported() {
    use mesh_api::types::{TransportKind, Via};
    let (mut w, a, b) = two_paired();
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let ca = w.contact_by_key(a, &w.identity_key(b)).unwrap();
    assert_eq!(
        ca.link,
        Some(TransportKind::Ble),
        "live Bluetooth link to the contact"
    );
    let m = w.nodes[a]
        .engine
        .send_text(conv_a, "via test".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(10_000, |w| has_text(w, b, conv_b, "via test")));
    let out = w.nodes[a]
        .engine
        .messages(conv_a, None, 100)
        .unwrap()
        .into_iter()
        .find(|x| x.id == m)
        .unwrap();
    assert_eq!(
        out.via,
        Some(Via::Ble),
        "sent message records the link it left on"
    );
    let inc = w.nodes[b]
        .engine
        .messages(conv_b, None, 100)
        .unwrap()
        .into_iter()
        .find(|x| x.text == "via test")
        .unwrap();
    assert_eq!(
        inc.via,
        Some(Via::Ble),
        "received message records the link it arrived on"
    );
    assert_eq!(
        w.nodes[a].engine.network_status().wifi_p2p_capable,
        w.nodes[a].tx.wifi
    );
}

/// The Settings switch: with Wi-Fi Direct off on one phone, a large file
/// still arrives, entirely over BLE, and no Wi-Fi link is ever opened.
#[test]
fn wifi_direct_switch_off_keeps_large_file_on_ble() {
    let mut w = World::new(T0, 11);
    let a = w.add_user_wifi("alice", (10.0, 10.0));
    let b = w.add_user_wifi("bob", (20.0, 10.0));
    w.nodes[b].engine.set_wifi_direct(false);
    assert!(!w.nodes[b].engine.network_status().wifi_direct_on);
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let file: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
    w.nodes[a]
        .engine
        .send_file(conv_a, "map.bin".into(), file, SendOptions::default())
        .unwrap();
    assert!(
        w.run_until(600_000, |w| w.nodes[b]
            .engine
            .messages(conv_b, None, 10)
            .unwrap()
            .iter()
            .any(
                |m| m.kind == MessageKind::File && m.chunks_have == m.chunks_total
            )),
        "large file not delivered over BLE"
    );
    assert_eq!(w.wifi_frames, 0, "Wi-Fi used although switched off");
    assert!(w.wifi_links.is_empty());
    // Back on: the setting persists and reports on.
    w.nodes[b].engine.set_wifi_direct(true);
    assert!(w.nodes[b].engine.network_status().wifi_direct_on);
}

/// "Carrying for others" counts only other people's envelopes: our own
/// outbound traffic and mail for us are not counted, a relay's are.
#[test]
fn carrying_for_others_counts_only_others() {
    let mut w = World::new(T0, 31);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(5_000);
    let ca = w.conv_with(a, b).unwrap();
    for i in 0..5 {
        w.nodes[a]
            .engine
            .send_text(ca, format!("m{i}"), SendOptions::default())
            .unwrap();
    }
    w.run_ms(20_000);
    // Alice and Bob only hold their own traffic and mail for themselves.
    assert_eq!(w.nodes[a].engine.network_status().relay_envelopes, 0);
    assert_eq!(w.nodes[b].engine.network_status().relay_envelopes, 0);
    // Carol, a stranger in range, carries copies for them.
    let c = w.add_user("carol", (15.0, 15.0));
    w.run_ms(2_000);
    w.nodes[a]
        .engine
        .send_text(ca, "carried by carol".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(60_000, |w| w.nodes[c]
        .engine
        .network_status()
        .relay_envelopes
        > 0));
}

/// A beacon-only change (here the Wi-Fi Direct capability bit) must not
/// restart advertising: on Android a restart can change the Bluetooth address
/// and break links that are being set up.
#[test]
fn capability_change_does_not_restart_advertising() {
    let mut w = World::new(T0, 41);
    let a = w.add_user_wifi("alice", (10.0, 10.0));
    w.run_ms(3_000);
    let before = w.nodes[a].advertise_starts;
    assert!(before >= 1, "never advertised");
    for on in [false, true, false, true] {
        w.nodes[a].engine.set_wifi_direct(on);
        w.run_ms(1_000);
    }
    assert_eq!(
        w.nodes[a].advertise_starts, before,
        "advertising restarted for a beacon-only change"
    );
    // A real change (radio mode) still re-advertises.
    w.nodes[a]
        .engine
        .set_radio_mode(RadioMode::ListenOnly)
        .unwrap();
    w.run_ms(1_000);
    assert!(w.nodes[a].advertise_starts > before || w.nodes[a].advertising.is_empty());
}

#[test]
fn channel_post_written_alone_turns_sent_when_a_phone_arrives() {
    let mut w = World::new(T0, 31);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (5_000.0, 5_000.0));
    w.run_ms(2_000);
    let ca = w.nodes[a].engine.join_channel("plaza".into()).unwrap();
    let cb = w.nodes[b].engine.join_channel("plaza".into()).unwrap();
    w.nodes[a]
        .engine
        .send_text(ca, "anyone?".into(), SendOptions::default())
        .unwrap();
    w.run_ms(5_000);
    let state = |w: &mut World| {
        w.nodes[a].engine.messages(ca, None, 10).unwrap().into_iter().find(|m| m.text == "anyone?").unwrap().state
    };
    assert_eq!(state(&mut w), DeliveryState::Queued);
    w.nodes[b].pos = (20.0, 10.0);
    assert!(w.run_until(30_000, |w| has_text(w, b, cb, "anyone?")), "post not carried");
    assert_eq!(state(&mut w), DeliveryState::Sent, "post still shown as waiting after a phone took it");
}

/// A file of many parts turns Delivered on the sender once every part is
/// acked, not before (acks are counted down without rewriting the record).
#[test]
fn many_part_file_turns_delivered_after_every_part_is_acked() {
    let mut w = World::new(T0, 47);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let file: Vec<u8> = (0..120_000u32).map(|i| (i * 17 % 253) as u8).collect();
    let m = w.nodes[a]
        .engine
        .send_file(conv_a, "notes.bin".into(), file, SendOptions::default())
        .unwrap();
    let total = w.nodes[a]
        .engine
        .messages(conv_a, None, 10)
        .unwrap()
        .iter()
        .find(|x| x.id == m)
        .map(|x| x.chunks_total)
        .unwrap();
    assert!(total > 10, "expected a many-part file, got {total} parts");
    let mut early = false;
    let delivered = w.run_until(600_000, |w| {
        let st = state_of(w, a, conv_a, m);
        let have = w.nodes[b]
            .engine
            .messages(conv_b, None, 10)
            .unwrap()
            .iter()
            .find(|x| x.kind == MessageKind::File)
            .map(|x| x.chunks_have)
            .unwrap_or(0);
        if st == Some(DeliveryState::Delivered) && have < total {
            early = true;
        }
        matches!(st, Some(DeliveryState::Delivered | DeliveryState::Read))
    });
    assert!(delivered, "sender never saw the file Delivered");
    assert!(!early, "Delivered before the receiver had every part");
}

/// Swipe to reply: the reply arrives with its quote and the original's
/// author; the quote survives when the original is gone, and a reply to a
/// message of another chat is refused.
#[test]
fn replies_carry_their_quote_and_author() {
    let mut w = World::new(T0, 53);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let q = w.nodes[a]
        .engine
        .send_text(conv_a, "Meet at the north gate at five?".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(30_000, |w| has_text(w, b, conv_b, "Meet at the north gate at five?")));
    // Bob answers Alice's message.
    let r = w.nodes[b]
        .engine
        .send_text_reply(conv_b, "Yes, see you there".into(), q, SendOptions::default())
        .unwrap();
    let mine = w.nodes[b]
        .engine
        .messages(conv_b, None, 20)
        .unwrap()
        .into_iter()
        .find(|m| m.id == r)
        .unwrap();
    let rv = mine.reply.expect("sender shows the quote");
    assert_eq!(rv.to, q);
    assert_eq!(rv.quote, "Meet at the north gate at five?");
    assert_eq!(rv.author.as_deref(), Some("alice"));
    assert!(rv.found);
    assert!(w.run_until(30_000, |w| has_text(w, a, conv_a, "Yes, see you there")));
    let got = w.nodes[a]
        .engine
        .messages(conv_a, None, 20)
        .unwrap()
        .into_iter()
        .find(|m| m.text == "Yes, see you there")
        .unwrap();
    let gv = got.reply.expect("receiver shows the quote");
    assert_eq!(gv.to, q);
    assert_eq!(gv.quote, "Meet at the north gate at five?");
    assert_eq!(gv.author.as_deref(), Some("You"));
    assert!(gv.found);
    // Alice deletes her original on her phone: the quote stays, not tappable.
    w.nodes[a].engine.delete_message(q, false).unwrap();
    let after = w.nodes[a]
        .engine
        .messages(conv_a, None, 20)
        .unwrap()
        .into_iter()
        .find(|m| m.id == got.id)
        .unwrap();
    let av = after.reply.unwrap();
    assert_eq!(av.quote, "Meet at the north gate at five?");
    assert!(!av.found);
    // A long original is cut to a short one-line quote.
    let long = w.nodes[a]
        .engine
        .send_text(conv_a, format!("line one\nline two {}", "x".repeat(300)), SendOptions::default())
        .unwrap();
    let rl = w.nodes[a]
        .engine
        .send_text_reply(conv_a, "about that".into(), long, SendOptions::default())
        .unwrap();
    let lv = w.nodes[a]
        .engine
        .messages(conv_a, None, 50)
        .unwrap()
        .into_iter()
        .find(|m| m.id == rl)
        .unwrap()
        .reply
        .unwrap();
    assert!(lv.quote.starts_with("line one line two x"));
    assert!(lv.quote.ends_with('…'));
    assert!(lv.quote.len() <= 120);
    // Not a message of this chat: refused.
    let c = w.add_user("carol", (15.0, 15.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(c, a));
    w.run_ms(3_000);
    let conv_ac = w.conv_with(a, c).unwrap();
    assert!(w.nodes[a]
        .engine
        .send_text_reply(conv_ac, "wrong chat".into(), q, SendOptions::default())
        .is_err());
}

/// Replies work in groups too: a member who is not the original author's
/// contact still sees the quote, with the author's group name.
#[test]
fn group_replies_carry_their_quote() {
    let mut w = World::new(T0, 59);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (15.0, 10.0));
    let c = w.add_user("carol", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    assert!(w.pair_in_person(c, a));
    w.run_ms(5_000);
    let cb = w.contact_by_key(a, &w.identity_key(b)).unwrap().id;
    let cc = w.contact_by_key(a, &w.identity_key(c)).unwrap().id;
    let g = w.nodes[a]
        .engine
        .create_group("trio".into(), vec![cb, cc])
        .unwrap();
    let q = w.nodes[a]
        .engine
        .send_text(g, "water at the clinic".into(), SendOptions::default())
        .unwrap();
    assert!(w.run_until(30_000, |w| has_text(w, b, g, "water at the clinic")
        && has_text(w, c, g, "water at the clinic")));
    w.nodes[b]
        .engine
        .send_text_reply(g, "how many bottles?".into(), q, SendOptions::default())
        .unwrap();
    assert!(w.run_until(30_000, |w| has_text(w, c, g, "how many bottles?")));
    let m = w.nodes[c]
        .engine
        .messages(g, None, 20)
        .unwrap()
        .into_iter()
        .find(|m| m.text == "how many bottles?")
        .unwrap();
    let rv = m.reply.expect("quote in group");
    assert_eq!(rv.to, q);
    assert_eq!(rv.quote, "water at the clinic");
    assert_eq!(rv.author.as_deref(), Some("alice"));
    assert!(rv.found);
}

/// A recorded Ogg Opus clip of `n` 20 ms packets, as a phone's recorder
/// would hand it over.
fn voice_ogg(n: usize) -> Vec<u8> {
    mesh_media::voice::VoiceClip {
        channels: 1,
        pre_skip: 312,
        packets: (0..n)
            .map(|i| {
                let mut p = vec![0xF8];
                p.extend((0..40).map(|j| (i * 13 + j) as u8));
                p
            })
            .collect(),
    }
    .to_ogg()
}

/// Voice notes: the audio arrives in parts, is checked, its length is taken
/// from the audio itself, it plays back as a fresh Ogg file, and "listened"
/// is tracked; High-Risk mode and junk recordings are refused.
#[test]
fn voice_notes_arrive_checked_and_playable() {
    let mut w = World::new(T0, 61);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    w.run_ms(3_000);
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    let ogg = voice_ogg(500); // 10 s, about 21 KB: several parts
    let wave: Vec<u8> = (0..64).map(|i| (i * 4) as u8).collect();
    let m = w.nodes[a]
        .engine
        .send_voice(conv_a, ogg.clone(), wave.clone(), None, SendOptions::default())
        .unwrap();
    let mine = w.nodes[a].engine.messages(conv_a, None, 10).unwrap();
    let mv = mine.iter().find(|x| x.id == m).unwrap();
    assert_eq!(mv.kind, MessageKind::Voice);
    assert!(mv.chunks_total > 1, "expected several parts");
    let v = mv.voice.as_ref().unwrap();
    assert_eq!(v.duration_ms, (500 * 960 - 312) * 1000 / 48_000);
    assert!(v.listened);
    assert!(w.run_until(120_000, |w| w.nodes[b]
        .engine
        .messages(conv_b, None, 10)
        .unwrap()
        .iter()
        .any(|x| x.kind == MessageKind::Voice && x.chunks_have == x.chunks_total)));
    let got = w.nodes[b]
        .engine
        .messages(conv_b, None, 10)
        .unwrap()
        .into_iter()
        .find(|x| x.kind == MessageKind::Voice)
        .unwrap();
    let gv = got.voice.clone().unwrap();
    assert_eq!(gv.duration_ms, v.duration_ms);
    assert_eq!(gv.waveform, wave);
    assert!(!gv.listened);
    // Playback: a rebuilt Ogg with exactly the sent audio packets.
    let played = w.nodes[b].engine.voice_audio(got.id).unwrap();
    let sent = mesh_media::voice::VoiceClip::from_ogg(&ogg).unwrap();
    assert_eq!(mesh_media::voice::VoiceClip::from_ogg(&played).unwrap(), sent);
    w.nodes[b].engine.voice_listened(got.id).unwrap();
    let after = w.nodes[b]
        .engine
        .messages(conv_b, None, 10)
        .unwrap()
        .into_iter()
        .find(|x| x.id == got.id)
        .unwrap();
    assert!(after.voice.unwrap().listened);
    // Bob answers with a voice note: it carries the quote of Alice's.
    let r = w.nodes[b]
        .engine
        .send_voice(conv_b, voice_ogg(50), vec![3; 10], Some(got.id), SendOptions::default())
        .unwrap();
    assert!(w.run_until(60_000, |w| w.nodes[a]
        .engine
        .messages(conv_a, None, 10)
        .unwrap()
        .iter()
        .any(|x| x.id == r && x.chunks_have == x.chunks_total)));
    let ra = w.nodes[a].engine.messages(conv_a, None, 10).unwrap().into_iter().find(|x| x.id == r).unwrap();
    let q = ra.reply.unwrap();
    assert_eq!((q.to, q.quote.as_str(), q.author.as_deref(), q.found), (m, "Voice note · 0:10", Some("You"), true));
    assert_eq!(ra.voice.unwrap().duration_ms, (50 * 960 - 312) * 1000 / 48_000);
    assert!(w.run_until(60_000, |w| matches!(
        state_of(w, a, conv_a, m),
        Some(DeliveryState::Delivered | DeliveryState::Read)
    )));
    // Not audio: refused before anything is sent.
    assert!(w.nodes[a]
        .engine
        .send_voice(conv_a, b"not a recording".to_vec(), vec![], None, SendOptions::default())
        .is_err());
    // High-Risk mode is text only.
    w.nodes[a].engine.set_high_risk(true).unwrap();
    assert!(w.nodes[a]
        .engine
        .send_voice(conv_a, voice_ogg(10), vec![], None, SendOptions::default())
        .is_err());
}

fn call_of(w: &World, i: usize) -> Option<CallView> {
    w.nodes[i].engine.current_call()
}

fn last_system_text(w: &mut World, i: usize, conv: Id16) -> Option<String> {
    w.nodes[i]
        .engine
        .messages(conv, None, 50)
        .unwrap()
        .into_iter()
        .rev()
        .find(|m| m.kind == MessageKind::Call)
        .map(|m| m.text)
}

/// Nearby calls: ring, answer, audio both ways (sealed, in order), hang up
/// with a note in both chats; a missed call; busy; High-Risk and
/// out-of-range refusals.
#[test]
fn nearby_calls_ring_connect_carry_audio_and_end() {
    let mut w = World::new(T0, 67);
    let a = w.add_user("alice", (10.0, 10.0));
    let b = w.add_user("bob", (20.0, 10.0));
    let c = w.add_user("carol", (15.0, 15.0));
    w.run_ms(2_000);
    assert!(w.pair_in_person(b, a));
    assert!(w.pair_in_person(c, b));
    w.run_ms(5_000);
    let cb = w.contact_by_key(a, &w.identity_key(b)).unwrap().id;
    let ca = w.contact_by_key(b, &w.identity_key(a)).unwrap().id;
    let conv_a = w.conv_with(a, b).unwrap();
    let conv_b = w.conv_with(b, a).unwrap();
    // Alice rings Bob.
    let id = w.nodes[a].engine.start_call(cb).unwrap();
    assert_eq!(call_of(&w, a).unwrap().state, CallState::Calling);
    assert!(w.run_until(10_000, |w| call_of(w, b).is_some_and(|c| c.state == CallState::Ringing)));
    let rb = call_of(&w, b).unwrap();
    assert_eq!(rb.id, id);
    assert_eq!(rb.contact, ca);
    assert!(!rb.outgoing);
    // Carol cannot get through while Bob is ringing: busy.
    let cc_b = w.contact_by_key(c, &w.identity_key(b)).unwrap().id;
    w.nodes[c].engine.start_call(cc_b).unwrap();
    assert!(w.run_until(10_000, |w| call_of(w, c).is_none()));
    // Bob answers; both become active.
    w.nodes[b].engine.answer_call(&id).unwrap();
    assert!(w.run_until(10_000, |w| call_of(w, a).is_some_and(|c| c.state == CallState::Active)));
    // Audio both ways, in order and intact.
    let sent: Vec<Vec<u8>> = (0..20u8).map(|i| vec![i; 120]).collect();
    for s in &sent {
        w.nodes[a].engine.call_send_audio(s);
        w.run_ms(80);
    }
    w.run_ms(1_000);
    assert_eq!(*w.nodes[b].tx.heard.lock().unwrap(), sent, "Bob heard Alice");
    // Every bundle got through: full quality, not reconnecting, over Bluetooth.
    let qb = call_of(&w, b).unwrap();
    assert_eq!((qb.quality, qb.reconnecting, qb.link), (4, false, Some(mesh_api::types::TransportKind::Ble)));
    w.nodes[b].engine.call_send_audio(&[7; 90]);
    w.run_ms(1_000);
    assert_eq!(w.nodes[a].tx.heard.lock().unwrap().last().unwrap(), &vec![7; 90]);
    // Nobody else hears it: Carol is in range but not in the call.
    assert!(w.nodes[c].tx.heard.lock().unwrap().is_empty());
    // Alice goes quiet: Bob's side shows the call recovering.
    w.run_ms(3_000);
    assert!(call_of(&w, b).unwrap().reconnecting);
    // Muted: empty bundles keep the call alive and play nothing.
    let before = w.nodes[b].tx.heard.lock().unwrap().len();
    for _ in 0..40 {
        w.nodes[a].engine.call_send_audio(&[0]);
        w.run_ms(80);
    }
    let qb = call_of(&w, b).unwrap();
    assert!(!qb.reconnecting && qb.quality >= 3, "{qb:?}");
    assert_eq!(w.nodes[b].tx.heard.lock().unwrap().len(), before);
    // Alice hangs up: both end, both chats get a note.
    w.nodes[a].engine.end_call(&id).unwrap();
    assert!(call_of(&w, a).is_none());
    assert!(w.run_until(10_000, |w| call_of(w, b).is_none()));
    for (i, conv) in [(a, conv_a), (b, conv_b)] {
        let note = last_system_text(&mut w, i, conv).unwrap();
        assert!(note.starts_with("Call · 0:") && note.ends_with(" · over Bluetooth"), "{note}");
    }
    // A call nobody answers: missed on Bob's side.
    w.nodes[a].engine.start_call(cb).unwrap();
    assert!(w.run_until(10_000, |w| call_of(w, b).is_some()));
    w.run_ms(50_000);
    assert!(call_of(&w, a).is_none() && call_of(&w, b).is_none());
    assert_eq!(last_system_text(&mut w, b, conv_b).as_deref(), Some("Missed call"));
    assert_eq!(last_system_text(&mut w, a, conv_a).as_deref(), Some("Call not answered · no answer"));
    // Declined.
    let id2 = w.nodes[a].engine.start_call(cb).unwrap();
    assert!(w.run_until(10_000, |w| call_of(w, b).is_some()));
    w.nodes[b].engine.end_call(&id2).unwrap();
    assert!(w.run_until(10_000, |w| call_of(w, a).is_none()));
    assert_eq!(last_system_text(&mut w, a, conv_a).as_deref(), Some("Call not answered · declined"));
    assert_eq!(last_system_text(&mut w, b, conv_b).as_deref(), Some("Call declined"));
    // High-Risk refuses calls.
    w.nodes[a].engine.set_high_risk(true).unwrap();
    assert!(w.nodes[a].engine.start_call(cb).is_err());
    w.nodes[a].engine.set_high_risk(false).unwrap();
    // Out of range: no call.
    w.nodes[b].pos = (900.0, 900.0);
    w.run_ms(5_000);
    assert!(w.nodes[a].engine.start_call(cb).is_err());
}
