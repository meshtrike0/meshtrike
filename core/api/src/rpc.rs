//! JSON method surface (D35). Both bindings forward `call(method, args)` here,
//! so the TypeScript client is identical on Android and in the browser.

use crate::engine::Engine;
use crate::types::*;
use base64::Engine as _;
use serde_json::{json, Value};

fn b64e(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn arg<'a>(a: &'a Value, k: &str) -> Result<&'a Value, CoreError> {
    a.get(k)
        .ok_or_else(|| CoreError::InvalidInput(format!("missing {k}")))
}

fn s(a: &Value, k: &str) -> Result<String, CoreError> {
    arg(a, k)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| CoreError::InvalidInput(k.into()))
}

fn opt_s(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(|v| v.as_str()).map(str::to_string)
}

fn u(a: &Value, k: &str) -> Result<u64, CoreError> {
    arg(a, k)?
        .as_u64()
        .ok_or_else(|| CoreError::InvalidInput(k.into()))
}

fn b(a: &Value, k: &str) -> Result<bool, CoreError> {
    arg(a, k)?
        .as_bool()
        .ok_or_else(|| CoreError::InvalidInput(k.into()))
}

fn id(a: &Value, k: &str) -> Result<Id16, CoreError> {
    let v = hex::decode(s(a, k)?).map_err(|_| CoreError::InvalidInput(k.into()))?;
    v.try_into().map_err(|_| CoreError::InvalidInput(k.into()))
}

fn opt_id(a: &Value, k: &str) -> Result<Option<Id16>, CoreError> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => id(a, k).map(Some),
    }
}

fn bytes(a: &Value, k: &str) -> Result<Vec<u8>, CoreError> {
    base64::engine::general_purpose::STANDARD
        .decode(s(a, k)?)
        .map_err(|_| CoreError::InvalidInput(k.into()))
}

fn ids(a: &Value, k: &str) -> Result<Vec<Id16>, CoreError> {
    arg(a, k)?
        .as_array()
        .ok_or_else(|| CoreError::InvalidInput(k.into()))?
        .iter()
        .map(|v| {
            let h = v
                .as_str()
                .ok_or_else(|| CoreError::InvalidInput(k.into()))?;
            hex::decode(h)
                .ok()
                .and_then(|x| x.try_into().ok())
                .ok_or_else(|| CoreError::InvalidInput(k.into()))
        })
        .collect()
}

fn de<T: serde::de::DeserializeOwned>(a: &Value, k: &str) -> Result<T, CoreError> {
    serde_json::from_value(arg(a, k)?.clone()).map_err(|_| CoreError::InvalidInput(k.into()))
}

fn opts(a: &Value) -> SendOptions {
    a.get("options")
        .and_then(|o| serde_json::from_value(o.clone()).ok())
        .unwrap_or_default()
}

fn qr_json(m: &mesh_handoff::QrMatrix) -> Value {
    json!({ "size": m.size, "modules": b64e(&m.modules) })
}

fn to<T: serde::Serialize>(v: T) -> Result<Value, CoreError> {
    serde_json::to_value(v).map_err(|_| CoreError::Internal(90))
}

/// Dispatch one method. Unknown methods are `InvalidInput`.
pub fn call(e: &mut Engine, method: &str, a: &Value) -> Result<Value, CoreError> {
    let ok = || Ok(Value::Null);
    match method {
        // ===== Lifecycle =====
        "has_identity" => to(e.has_identity()),
        "is_unlocked" => to(e.is_unlocked()),
        "create_identity" => {
            to(e.create_identity(s(a, "nickname")?, s(a, "pin")?, opt_s(a, "duress_pin"))?)
        }
        "unlock" => to(e.unlock(s(a, "pin")?)?),
        "unlock_biometric" => to(e.unlock_biometric()?),
        "enable_biometric" => to(e.enable_biometric(s(a, "pin")?)?),
        "lock" => {
            e.lock();
            ok()
        }
        "panic_wipe" => {
            e.panic_wipe();
            ok()
        }
        "identity" => to(e.identity()?),
        "set_foreground" => {
            e.set_foreground(b(a, "foreground")?);
            ok()
        }
        // ===== Contacts and pairing =====
        "pairing_qr" => {
            let p = e.pairing_qr()?;
            let m = e.qr_matrix(p.clone())?;
            Ok(json!({ "payload": b64e(&p), "qr": qr_json(&m) }))
        }
        "pair_from_qr" => to(hex::encode(e.pair_from_qr(bytes(a, "payload")?)?)),
        "pair_from_post" => to(hex::encode(e.pair_from_post(id(a, "message")?)?)),
        "accept_pairing" => to(e.accept_pairing(id(a, "pairing")?)?),
        "decline_pairing" => to(e.decline_pairing(id(a, "pairing")?)?),
        "contacts" => to(e.contacts()),
        "safety_number" => to(e.safety_number(id(a, "contact")?)?),
        "mark_verified" => to(e.mark_verified(id(a, "contact")?)?),
        "remove_contact" => to(e.remove_contact(id(a, "contact")?)?),
        "reset_sessions" => to(e.reset_sessions()?),
        // ===== Conversations =====
        "conversations" => to(e.conversations()),
        "messages" => to(e.messages(
            id(a, "conversation")?,
            opt_id(a, "before")?,
            a.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as u32,
        )?),
        "send_text" => to(hex::encode(match opt_id(a, "reply_to")? {
            Some(r) => e.send_text_reply(id(a, "conversation")?, s(a, "text")?, r, opts(a))?,
            None => e.send_text(id(a, "conversation")?, s(a, "text")?, opts(a))?,
        })),
        "send_image" => to(hex::encode(e.send_image(
            id(a, "conversation")?,
            bytes(a, "rgba")?,
            u(a, "width")? as u32,
            u(a, "height")? as u32,
            opts(a),
        )?)),
        "send_jpeg" => to(hex::encode(e.send_jpeg_bytes(
            id(a, "conversation")?,
            bytes(a, "jpeg")?,
            opts(a),
        )?)),
        "send_file" => to(hex::encode(e.send_file(
            id(a, "conversation")?,
            s(a, "name")?,
            bytes(a, "bytes")?,
            opts(a),
        )?)),
        "send_voice" => to(hex::encode(e.send_voice(
            id(a, "conversation")?,
            bytes(a, "ogg")?,
            bytes(a, "waveform").unwrap_or_default(),
            opt_id(a, "reply_to")?,
            opts(a),
        )?)),
        "voice_audio" => Ok(json!({ "ogg": b64e(&e.voice_audio(id(a, "message")?)?) })),
        "voice_listened" => to(e.voice_listened(id(a, "message")?)?),
        "start_call" => to(e.start_call(id(a, "contact")?)?),
        "answer_call" => to(e.answer_call(&s(a, "call")?)?),
        "end_call" => to(e.end_call(&s(a, "call")?)?),
        "current_call" => to(e.current_call()),
        "release_held" => to(e.release_held(id(a, "message")?)?),
        "image_pixels" => to(e.image_pixels(id(a, "message")?)?),
        "file_bytes" => {
            let (name, data) = e.file_bytes(id(a, "message")?)?;
            Ok(json!({ "name": name, "bytes": b64e(&data) }))
        }
        "delete_message" => to(e.delete_message(
            id(a, "message")?,
            a.get("for_everyone")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        )?),
        "mark_read" => to(e.mark_read(id(a, "conversation")?)?),
        "set_disappearing" => to(e.set_disappearing(id(a, "conversation")?, u(a, "secs")? as u32)?),
        // ===== Groups and channels =====
        "create_group" => to(hex::encode(
            e.create_group(s(a, "name")?, ids(a, "members")?)?,
        )),
        "add_members" => to(e.add_members(id(a, "conversation")?, ids(a, "members")?)?),
        "remove_member" => to(e.remove_member(id(a, "conversation")?, id(a, "member")?)?),
        "leave_group" => to(e.leave_group(id(a, "conversation")?)?),
        "group_members" => to(e.group_members(id(a, "conversation")?)?),
        "join_channel" => to(hex::encode(e.join_channel(s(a, "name")?)?)),
        "leave_channel" => to(e.leave_channel(id(a, "conversation")?)?),
        // ===== Settings and modes =====
        "settings" => to(e.get_settings()),
        "set_radio_mode" => to(e.set_radio_mode(de(a, "mode")?)?),
        "set_high_risk" => to(e.set_high_risk(b(a, "on")?)?),
        "set_wifi_direct" => {
            e.set_wifi_direct(b(a, "on")?);
            ok()
        }
        // ===== Internet route (amendment N1) =====
        "set_internet" => {
            e.set_internet(b(a, "on")?);
            ok()
        }
        "set_internet_direct" => {
            e.set_internet_direct(b(a, "on")?);
            ok()
        }
        "bridges" => to(e.bridges()),
        "add_bridge" => to(e.add_bridge(&s(a, "link")?)?),
        "remove_bridge" => to(e.remove_bridge(&s(a, "id")?)?),
        "bridge_link" => to(e.bridge_link(&s(a, "id")?)?),
        "bridge_qr" => {
            let link = e.bridge_link(&s(a, "id")?)?;
            let m = e.qr_matrix(link.into_bytes())?;
            Ok(json!({ "qr": qr_json(&m) }))
        }
        "share_bridges" => to(e.share_bridges(&id(a, "contact")?)?),
        "on_internet_ready" => to(e.on_internet_ready(s(a, "address")?)?),
        "on_internet_down" => {
            e.on_internet_down();
            ok()
        }
        "on_internet_envelope" => to(e.on_internet_envelope(bytes(a, "bytes")?)),
        "set_read_receipts" => {
            e.set_read_receipts(b(a, "on")?);
            ok()
        }
        "set_disappear_default" => to(e.set_disappear_default(u(a, "secs")? as u32)?),
        "set_auto_lock" => {
            e.set_auto_lock(u(a, "ms")?);
            ok()
        }
        "set_duress_pin" => to(e.set_duress_pin(s(a, "pin")?, opt_s(a, "duress"))?),
        "set_relay_enabled" => {
            let mut st = e.get_settings();
            st.relay_enabled = b(a, "on")?;
            e.save_settings_pub(st);
            ok()
        }
        "create_community" => to(hex::encode(e.create_community(s(a, "name")?)?)),
        "communities" => to(e.communities()),
        "community_qr" => {
            let p = e.community_qr(id(a, "community")?)?;
            let m = e.qr_matrix(p.clone())?;
            Ok(json!({ "payload": b64e(&p), "qr": qr_json(&m) }))
        }
        "join_community" => to(hex::encode(e.join_community(bytes(a, "payload")?)?)),
        "leave_community" => to(e.leave_community(id(a, "community")?)?),
        // ===== Handoff and QR =====
        "handoff_start_send" => to(e.handoff_start_send()?),
        "handoff_next_frame" => {
            let (f, m) = e.handoff_next_frame()?;
            Ok(json!({ "frame": b64e(&f), "qr": qr_json(&m) }))
        }
        "handoff_start_receive" => {
            e.handoff_start_receive();
            ok()
        }
        "handoff_push_frame" => to(e.handoff_push_frame(bytes(a, "frame")?)?),
        "handoff_push_luma" => to(e.handoff_push_luma(
            u(a, "width")? as u32,
            u(a, "height")? as u32,
            bytes(a, "luma")?,
        )?),
        "handoff_stop" => {
            e.handoff_stop();
            ok()
        }
        "decode_qr_luma" => {
            let codes = e.decode_qr_luma(
                u(a, "width")? as u32,
                u(a, "height")? as u32,
                bytes(a, "luma")?,
            );
            to(codes.iter().map(|c| b64e(c)).collect::<Vec<_>>())
        }
        "qr_matrix" => Ok(qr_json(&e.qr_matrix(bytes(a, "data")?)?)),
        "network_status" => to(e.network_status()),
        "biometric_enabled" => to(e.biometric_enabled()),
        "version" => to(serde_json::json!({
            "core": env!("CARGO_PKG_VERSION"),
            "protocol": mesh_constants::PROTOCOL_VERSION,
        })),
        "debug_log" => to(e.debug_log()),
        "integrity_report" => {
            let chain: Option<Vec<Vec<u8>>> =
                a.get("chain").and_then(|c| c.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
                        .collect()
                });
            to(e.integrity_report(chain))
        }
        // ===== Transport callbacks =====
        "on_peer_found" => {
            let uuids = a
                .get("uuids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|h| hex::decode(h.replace('-', "")).ok()?.try_into().ok())
                        .collect()
                })
                .unwrap_or_default();
            let kind = a
                .get("transport")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(TransportKind::Ble);
            e.on_peer_found(
                u(a, "peer")?,
                uuids,
                a.get("rssi").and_then(|v| v.as_i64()).unwrap_or(-60) as i16,
                kind,
            );
            ok()
        }
        "on_connected" => {
            let kind = a
                .get("transport")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            e.on_connected_kind(
                u(a, "peer")?,
                a.get("mtu").and_then(|v| v.as_u64()).unwrap_or(185) as u16,
                a.get("outgoing").and_then(|v| v.as_bool()).unwrap_or(false),
                kind,
            );
            ok()
        }
        "on_frame" => {
            e.on_frame(u(a, "peer")?, bytes(a, "bytes")?);
            ok()
        }
        "on_disconnected" => {
            e.on_disconnected(u(a, "peer")?);
            ok()
        }
        "on_radio_sample" => {
            e.on_radio_sample(
                u(a, "adverts")? as u32,
                u(a, "failed")? as u32,
                b(a, "stationary")?,
            );
            ok()
        }
        "on_lora_packet" => {
            e.on_lora_packet(bytes(a, "bytes")?);
            ok()
        }
        "radio_restart" => {
            e.restart_radio();
            ok()
        }
        "tick" => {
            e.tick();
            ok()
        }
        _ => Err(CoreError::InvalidInput(format!("unknown method {method}"))),
    }
}

/// Wrap a result as the JSON envelope both bindings return.
pub fn call_json(e: &mut Engine, method: &str, args_json: &str) -> String {
    let args: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(e, method, &args)));
    let v = match r {
        Ok(Ok(v)) => json!({ "ok": v }),
        Ok(Err(err)) => json!({ "err": err }),
        Err(_) => json!({ "err": CoreError::Internal(99) }),
    };
    v.to_string()
}
