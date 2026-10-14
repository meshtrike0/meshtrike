//! Internet route (amendment N1): the same sealed envelopes the mesh carries,
//! also sent through the Nym mixnet to contacts who told us their address.
//!
//! The host runs the network side (on Android, the separate Internet add-on:
//! Nym client behind a VLESS+REALITY disguise). This module only decides
//! which envelope goes to which address and keeps an outbox while the route
//! is down. Envelopes are already end-to-end sealed; the host never sees
//! plaintext, names or keys.
//!
//! Storage, all in the relay database so receiving and acks keep working
//! while locked (the relay database already maps contacts to tag keys):
//! - `t::NETADDR`: contact → their current Nym address.
//! - `t::NETOUT`: envelope id → contacts still owed that envelope.
//! - `t::NETSHARED`: contact → the address of ours they last received.
//! - `t::BRIDGE`: bridge id → share link.
//! - `t::SETTINGS` keys `net_on` / `net_direct`: the two switches.

use super::*;
use mesh_wire::inner::Content;
use mesh_wire::*;

const NET_ON_KEY: &[u8] = b"net_on";
const NET_DIRECT_KEY: &[u8] = b"net_direct";

/// Re-send undelivered envelopes over the internet at most this often.
const NET_RESEND_MS: u64 = 30 * 60 * 1000;

impl Engine {
    // ===== Switches =====

    pub(crate) fn net_on(&self) -> bool {
        self.relay
            .as_ref()
            .and_then(|r| records::load::<bool>(&*r.kv, t::SETTINGS, NET_ON_KEY))
            .unwrap_or(false)
    }

    pub(crate) fn net_direct(&self) -> bool {
        self.relay
            .as_ref()
            .and_then(|r| records::load::<bool>(&*r.kv, t::SETTINGS, NET_DIRECT_KEY))
            .unwrap_or(false)
    }

    /// The route may carry envelopes now: switched on, not High-Risk, and the
    /// host reports the client connected.
    pub(crate) fn net_usable(&self) -> bool {
        self.net_address.is_some() && self.net_on() && !self.settings().high_risk
    }

    /// Settings: "Internet route". Off by default; High-Risk keeps it off.
    pub fn set_internet(&mut self, on: bool) {
        if let Some(r) = self.relay.as_mut() {
            let _ = r.kv.apply(vec![records::put(t::SETTINGS, NET_ON_KEY, &on)]);
        }
        self.push_net_config();
        self.emit_network();
    }

    /// Settings: allow connecting without a disguise bridge (the network then
    /// sees that this phone uses Nym).
    pub fn set_internet_direct(&mut self, on: bool) {
        if let Some(r) = self.relay.as_mut() {
            let _ =
                r.kv.apply(vec![records::put(t::SETTINGS, NET_DIRECT_KEY, &on)]);
        }
        self.push_net_config();
        self.emit_network();
    }

    /// Tell the host whether to run the route, and with which bridges.
    pub(crate) fn push_net_config(&mut self) {
        let enabled = self.net_on() && !self.settings().high_risk;
        if !enabled {
            self.net_address = None;
        }
        let cfg = serde_json::json!({
            "bridges": self.bridge_links(),
            "allow_direct": self.net_direct(),
        });
        self.host.tx.internet_config(enabled, cfg.to_string());
    }

    // ===== Host callbacks =====

    /// The host's client is connected; `address` is ours on the mixnet.
    pub fn on_internet_ready(&mut self, address: String) -> CoreResult<()> {
        if address.is_empty() || address.len() > MAX_REACH_BYTES || !address.is_ascii() {
            return Err(CoreError::InvalidInput("address".into()));
        }
        if !self.net_on() || self.settings().high_risk {
            return Ok(());
        }
        self.log("internet route connected");
        self.net_address = Some(address);
        self.net_last_flush = 0;
        self.share_reach();
        self.flush_net_outbox();
        self.emit_network();
        self.emit_contacts_changed();
        Ok(())
    }

    /// The client lost its connection (or was stopped).
    pub fn on_internet_down(&mut self) {
        if self.net_address.take().is_some() {
            self.log("internet route down");
            self.emit_network();
            self.emit_contacts_changed();
        }
    }

    /// An envelope arrived over the mixnet. Same ingest as any carrier:
    /// duplicates of mesh copies are dropped by message id.
    pub fn on_internet_envelope(&mut self, bytes: Vec<u8>) -> bool {
        if !self.net_usable() {
            return false;
        }
        self.ingest_raw_via(&bytes, Some(Via::Internet))
    }

    // ===== Sending =====

    /// Send one of our envelopes to each of `peers` that has an address, or
    /// remember it for later. A message written while the route is off, or
    /// before a contact's address has arrived, still goes over the internet
    /// once both are there (until it is acked or expires). High-Risk never
    /// records anything for the internet.
    pub(crate) fn net_route(&mut self, env: &Envelope, peers: &[Id16]) {
        if peers.is_empty() || self.settings().high_risk {
            return;
        }
        let Some(r) = self.relay.as_ref() else { return };
        let targets: Vec<(Id16, Option<String>)> = peers
            .iter()
            .map(|p| {
                let a = records::load::<String>(&*r.kv, t::NETADDR, p).filter(|a| !a.is_empty());
                (*p, a)
            })
            .collect();
        let bytes = env.encode();
        let usable = self.net_usable();
        let mut owed = Vec::new();
        let mut sent = false;
        for (peer, addr) in targets {
            let ok = match addr {
                Some(a) if usable => self.host.tx.internet_send(a, bytes.clone()),
                _ => false,
            };
            if ok {
                sent = true;
            } else {
                owed.push(peer);
            }
        }
        if sent && self.msg.is_some() {
            self.net_sent_pending.push(env.message_id);
        }
        if sent {
            let kind = format!("{:?}", env.typ).to_lowercase();
            self.log(format!("sealed {kind} envelope handed to the internet route"));
        }
        // Kept until acked or expired even when sent, so a lost copy is re-sent
        // on reconnect; only direct envelopes are acked, so others are kept
        // only while owed.
        let keep: Vec<Id16> = if env.typ == EnvType::Direct {
            peers.to_vec()
        } else {
            owed
        };
        if !keep.is_empty() {
            if let Some(r) = self.relay.as_mut() {
                let _ =
                    r.kv.apply(vec![records::put(t::NETOUT, &env.message_id, &keep)]);
            }
        }
    }

    /// Send owed envelopes: on reconnect, then every `NET_RESEND_MS` while
    /// they stay undelivered. The receiver drops duplicates.
    pub(crate) fn flush_net_outbox(&mut self) {
        let now = self.now();
        if self.net_last_flush != 0 && now.saturating_sub(self.net_last_flush) < NET_RESEND_MS {
            return;
        }
        self.net_last_flush = now;
        self.net_send_owed(None);
    }

    /// Send owed envelopes (to every peer, or only to `only`), and drop
    /// entries whose envelope is gone. Pruning also runs while the route is
    /// off, so the outbox never outgrows the envelope store.
    fn net_send_owed(&mut self, only: Option<&Id16>) {
        let usable = self.net_usable();
        let Some(r) = self.relay.as_ref() else { return };
        let owed = records::load_all::<Vec<Id16>>(&*r.kv, t::NETOUT);
        let mut drop = Vec::new();
        let mut sends = Vec::new();
        for (k, peers) in owed {
            let Ok(id) = MessageId::try_from(k.as_slice()) else {
                drop.push(k);
                continue;
            };
            let Some(s) = r.store.get(&id) else {
                // Acked, expired or evicted: nothing left to send.
                drop.push(k);
                continue;
            };
            if !usable {
                continue;
            }
            let bytes = s.env.encode();
            for p in peers.iter().filter(|p| only.is_none_or(|o| o == *p)) {
                if let Some(a) = records::load::<String>(&*r.kv, t::NETADDR, p).filter(|a| !a.is_empty()) {
                    sends.push((id, a, bytes.clone()));
                }
            }
        }
        let n = sends.len();
        for (id, a, b) in sends {
            if self.host.tx.internet_send(a, b) && self.msg.is_some() {
                self.net_sent_pending.push(id);
            }
        }
        if let Some(r) = self.relay.as_mut() {
            let _ =
                r.kv.apply(drop.into_iter().map(|k| Op::Delete(t::NETOUT, k)).collect());
        }
        if n > 0 {
            self.log(format!("internet route: re-sent {n} envelope copies"));
        }
    }

    pub(crate) fn net_forget_envelope(&mut self, id: &MessageId) {
        if let Some(r) = self.relay.as_mut() {
            let _ = r.kv.delete(t::NETOUT, id);
        }
    }

    // ===== Addresses =====

    pub(crate) fn net_address_of(&self, peer: &Id16) -> Option<String> {
        let r = self.relay.as_ref()?;
        records::load::<String>(&*r.kv, t::NETADDR, peer).filter(|a| !a.is_empty())
    }

    /// Tell every contact our current address once (needs the message
    /// database for Olm, so it runs again after unlock).
    pub(crate) fn share_reach(&mut self) {
        let Some(addr) = self.net_address.clone() else {
            return;
        };
        if self.msg.is_none() {
            return;
        }
        let peers: Vec<Id16> = self
            .msg
            .as_ref()
            .map(|m| {
                m.peers
                    .iter()
                    .filter(|(_, p)| p.listed)
                    .map(|(id, _)| *id)
                    .collect()
            })
            .unwrap_or_default();
        let now = self.now();
        for p in peers {
            let shared = self
                .relay
                .as_ref()
                .and_then(|r| records::load::<String>(&*r.kv, t::NETSHARED, &p));
            if shared.as_deref() == Some(addr.as_str()) {
                continue;
            }
            if self
                .send_control(&p, Content::Reach(addr.clone()), now)
                .is_ok()
            {
                if let Some(r) = self.relay.as_mut() {
                    let _ = r.kv.apply(vec![records::put(t::NETSHARED, &p, &addr)]);
                }
            }
        }
    }

    /// A contact told us their address (or withdrew it with an empty one).
    pub(crate) fn on_reach(&mut self, peer: &Id16, address: String) {
        if address.len() > MAX_REACH_BYTES || !address.is_ascii() {
            return;
        }
        if let Some(r) = self.relay.as_mut() {
            let op = if address.is_empty() {
                Op::Delete(t::NETADDR, peer.to_vec())
            } else {
                records::put(t::NETADDR, peer, &address)
            };
            let _ = r.kv.apply(vec![op]);
        }
        // A new address: send what this contact is still owed right away.
        if !address.is_empty() && !self.settings().high_risk {
            self.net_send_owed(Some(peer));
        }
        if let Some(c) = self.contact_view(peer) {
            self.emit(Event::ContactChanged { contact: c });
        }
    }

    pub(crate) fn net_forget_peer(&mut self, peer: &Id16) {
        if let Some(r) = self.relay.as_mut() {
            let _ = r.kv.apply(vec![
                Op::Delete(t::NETADDR, peer.to_vec()),
                Op::Delete(t::NETSHARED, peer.to_vec()),
            ]);
        }
    }

    fn emit_contacts_changed(&self) {
        for c in self.contacts() {
            self.emit(Event::ContactChanged { contact: c });
        }
    }

    // ===== Bridges =====

    pub(crate) fn bridge_links(&self) -> Vec<String> {
        let Some(r) = self.relay.as_ref() else {
            return vec![];
        };
        records::load_all::<BridgeRec>(&*r.kv, t::BRIDGE)
            .into_iter()
            .map(|(_, b)| b.link)
            .collect()
    }

    pub fn bridges(&self) -> Vec<Bridge> {
        let Some(r) = self.relay.as_ref() else {
            return vec![];
        };
        let mut v: Vec<Bridge> = records::load_all::<BridgeRec>(&*r.kv, t::BRIDGE)
            .into_iter()
            .filter_map(|(k, b)| {
                let p = parse_bridge(&b.link)?;
                Some(Bridge {
                    id: hex::encode(k),
                    host: p.host,
                    port: p.port,
                    disguise: p.sni,
                    from_contact: b.from.is_some(),
                    added_ms: b.added_ms,
                })
            })
            .collect();
        v.sort_by_key(|b| b.added_ms);
        v
    }

    /// Add a bridge from a share link (typed, scanned or from a contact).
    pub fn add_bridge(&mut self, link: &str) -> CoreResult<Bridge> {
        self.add_bridge_from(link, None)
    }

    fn add_bridge_from(&mut self, link: &str, from: Option<Id16>) -> CoreResult<Bridge> {
        let link = link.trim();
        let p = parse_bridge(link).ok_or_else(|| CoreError::InvalidInput("bridge link".into()))?;
        let id: [u8; 16] = mesh_crypto::sha256(&[b"bridge-v1", link.as_bytes()])[..16]
            .try_into()
            .unwrap_or([0; 16]);
        let now = self.now();
        let r = self.relay.as_mut().ok_or(CoreError::Locked)?;
        let existing = records::load_all::<BridgeRec>(&*r.kv, t::BRIDGE);
        if !existing.iter().any(|(k, _)| k.as_slice() == id) && existing.len() >= MAX_BRIDGES {
            return Err(CoreError::InvalidInput("too many bridges".into()));
        }
        let rec = BridgeRec {
            link: link.to_string(),
            from,
            added_ms: now,
        };
        let _ = r.kv.apply(vec![records::put(t::BRIDGE, &id, &rec)]);
        self.push_net_config();
        self.emit_network();
        Ok(Bridge {
            id: hex::encode(id),
            host: p.host,
            port: p.port,
            disguise: p.sni,
            from_contact: from.is_some(),
            added_ms: now,
        })
    }

    pub fn remove_bridge(&mut self, id_hex: &str) -> CoreResult<()> {
        let id = hex::decode(id_hex).map_err(|_| CoreError::InvalidInput("id".into()))?;
        let r = self.relay.as_mut().ok_or(CoreError::Locked)?;
        let _ = r.kv.apply(vec![Op::Delete(t::BRIDGE, id)]);
        self.push_net_config();
        self.emit_network();
        Ok(())
    }

    /// The share link of one bridge, for a QR code.
    pub fn bridge_link(&self, id_hex: &str) -> CoreResult<String> {
        let id = hex::decode(id_hex).map_err(|_| CoreError::InvalidInput("id".into()))?;
        let r = self.relay.as_ref().ok_or(CoreError::Locked)?;
        records::load::<BridgeRec>(&*r.kv, t::BRIDGE, &id)
            .map(|b| b.link)
            .ok_or(CoreError::NotFound)
    }

    /// Offer our bridges to a contact over the encrypted channel.
    pub fn share_bridges(&mut self, peer: &Id16) -> CoreResult<u32> {
        let links: Vec<String> = self
            .bridge_links()
            .into_iter()
            .take(MAX_BRIDGES_IN_MESSAGE)
            .collect();
        if links.is_empty() {
            return Err(CoreError::InvalidInput("no bridges".into()));
        }
        let n = links.len() as u32;
        let now = self.now();
        self.send_control(peer, Content::Bridges(links), now)?;
        Ok(n)
    }

    /// Bridges from a contact are only taken from a contact verified in
    /// person: a bridge sees the phone's IP address, so an unverified contact
    /// must not be able to steer us to theirs.
    pub(crate) fn on_bridges(&mut self, peer: &Id16, links: Vec<String>) {
        let verified = self
            .msg
            .as_ref()
            .and_then(|m| m.peers.get(peer))
            .is_some_and(|p| p.trust == Trust::Verified);
        if !verified {
            self.log("bridges offered by an unverified contact: ignored");
            return;
        }
        let mut added = 0;
        for l in links.iter().take(MAX_BRIDGES_IN_MESSAGE) {
            if self.add_bridge_from(l, Some(*peer)).is_ok() {
                added += 1;
            }
        }
        if added > 0 {
            self.log(format!(
                "{added} bridge(s) received from a verified contact"
            ));
        }
    }
}

/// What the app shows about a bridge (never the secret parts of the link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedBridge {
    pub host: String,
    pub port: u16,
    pub sni: String,
}

/// Accepts `vless://<uuid>@<host>:<port>?...security=reality...&pbk=<key>&sni=<name>`.
/// Only REALITY bridges: plain TLS or no security would show the network an
/// unusual connection with no disguise.
pub(crate) fn parse_bridge(link: &str) -> Option<ParsedBridge> {
    if link.len() > MAX_BRIDGE_LINK_BYTES || !link.is_ascii() {
        return None;
    }
    let rest = link.strip_prefix("vless://")?;
    let rest = rest.split('#').next()?;
    let (user_host, query) = rest.split_once('?')?;
    let (uuid, hostport) = user_host.split_once('@')?;
    if uuid.len() < 8 || uuid.len() > 64 {
        return None;
    }
    let (host, port) = if let Some(h) = hostport.strip_prefix('[') {
        let (h, p) = h.split_once("]:")?;
        (h.to_string(), p)
    } else {
        let (h, p) = hostport.rsplit_once(':')?;
        (h.to_string(), p)
    };
    let port: u16 = port.trim_end_matches('/').parse().ok().filter(|p| *p > 0)?;
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    let mut security = "";
    let mut pbk = "";
    let mut sni = "";
    for kv in query.split('&') {
        match kv.split_once('=') {
            Some(("security", v)) => security = v,
            Some(("pbk", v)) => pbk = v,
            Some(("sni", v)) => sni = v,
            _ => {}
        }
    }
    if security != "reality" || pbk.len() < 32 || sni.is_empty() {
        return None;
    }
    Some(ParsedBridge {
        host,
        port,
        sni: sni.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "vless://3f1c2a9e-1b2c-4d5e-8f90-a1b2c3d4e5f6@203.0.113.7:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw&sid=6ba85179e30d4fc2&type=tcp#bridge";

    #[test]
    fn parses_reality_links_only() {
        let p = parse_bridge(GOOD).unwrap();
        assert_eq!(
            (p.host.as_str(), p.port, p.sni.as_str()),
            ("203.0.113.7", 443, "www.microsoft.com")
        );
        assert!(parse_bridge(&GOOD.replace("security=reality", "security=tls")).is_none());
        assert!(parse_bridge(&GOOD.replace("pbk=", "pbx=")).is_none());
        assert!(parse_bridge("vmess://abc").is_none());
        assert!(parse_bridge(&GOOD.replace(":443?", ":0?")).is_none());
        let v6 = GOOD.replace("203.0.113.7", "[2001:db8::7]");
        assert_eq!(parse_bridge(&v6).unwrap().host, "2001:db8::7");
    }
}
