# Blueprint v1.02 — amendments

Base documents: `blueprint-v1.0.md` (full text) and `blueprint-v1.01-revisions.txt`
(revisions to sections 6, 7, 8, 10, 11, 12.1, 12.3). This file is the delta from
v1.01 to v1.02. Where they conflict: **v1.02 > v1.01 > v1.0**. Every change has a
decision-log row (D17+, section 24).

## A. Wire format (revision 2) — consolidated, normative

### A.1 Envelope

| Field | Bytes | Notes |
| --- | --- | --- |
| Version | 1 | `0x02` |
| Type | 1 | 0x01 Direct, 0x02 Group, 0x03 Broadcast, 0x04 Handshake, 0x05 Ack, 0x06 Cover |
| TTL | 1 | random start 5–7; the only field relays change; set to 0 in associated data |
| Flags | 1 | bit 0 LoRa-eligible; bits 1–7 zero |
| Message ID | 16 | random; dedup, acks, outer nonce |
| Recipient tag | 16 | A.3 |
| Expiry | 4 | Unix seconds, multiple of 3,600 |
| Proof | 8 | Broadcast: PoW nonce. Direct: SHA-256(ack secret)[0..8]. Others: zero |
| Reserved | 2 | zero |
| Payload | W + 16 | XChaCha20-Poly1305(envelope key, nonce = message ID ‖ 0⁸, AD = header with TTL = 0) |

Wire plaintext = `len (2, BE) ‖ inner message ‖ zero padding to W`.

### A.2 Buckets (D33: margin 192 bytes, replaces 160)

| Inner bucket | Wire size W | Envelope total |
| --- | --- | --- |
| LoRa 96 | 167 | 233 |
| 256 | 448 | 514 |
| 512 | 704 | 770 |
| 1,024 | 1,216 | 1,282 |
| 4,096 | 4,288 | 4,354 |

Rationale: an Olm PreKey message carrying a 4,096-byte padded plaintext is up to
4,270 bytes; with the type byte and length prefix that is 4,273 (margin 177). Every
message an Olm initiator sends before its first reply is a PreKey message, so the
margin must cover it for all buckets. LoRa never carries PreKey messages.

The wire size is chosen from the inner bucket; if an inner message still does not
fit (cannot happen with vodozemac v0.9 limits) the next bucket is used.

### A.3 Inner-message framing (R2)

| Envelope type | Inner message |
| --- | --- |
| Direct | `olm type (1: 0 = PreKey, 1 = Normal) ‖ Olm message` |
| Group | `Megolm session ID (32) ‖ Megolm message` |
| Broadcast | `pseudonym pk (32) ‖ signature (64) ‖ reply tag (16) ‖ inner plaintext` |
| Handshake | `kind (1: 1 = XX msg 1, 2 = XX msg 2, 3 = XX msg 3) ‖ Noise message` |
| Ack (not encrypted) | `acked message ID (16) ‖ ack secret (16) ‖ zero pad`; payload length W(256)+16 = 464 |
| Cover (not encrypted) | 464 random bytes |

### A.4 Keys and tags (unchanged from v1.01, restated)

- Direction `dir` = 0x00 if sender identity key < recipient identity key (byte-wise), else 0x01.
- Direct tag = HMAC-SHA256(pairwise tag key, "tag-v2" ‖ dir ‖ expiry BE32)[0..16].
- Direct envelope key = HKDF-SHA256(ikm = pairwise tag key, salt = none, info = "env-v2" ‖ dir).
- Ack secret = HMAC-SHA256(envelope key, "ack-v1" ‖ message ID)[0..16]; proof = SHA-256(ack secret)[0..8].
- Group tag = HMAC-SHA256(group tag key, "tag-v2" ‖ expiry BE32)[0..16]; key = HKDF(group tag key, "env-v2").
- Channel key = HKDF-SHA256(ikm = NFC(lowercase(name)) UTF-8, info "chan-v1"); channel hash = SHA-256("chan-id-v1" ‖ channel key)[0..16].
- Handshake key = HKDF-SHA256(ikm = reply/return tag, info "hs-v2"). (The reply tag is public to channel readers; handshake confidentiality comes from Noise.)

### A.5 Re-sealing (R3)

Any change to an authenticated header field (expiry extension under jamming, retry)
re-seals the envelope under a **new message ID**. Reusing a message ID with a
different header reuses the XChaCha20-Poly1305 nonce and breaks authentication.
The sender tracks the superseded ID until expiry so either Ack completes delivery.

### A.6 Link frames (v1.01, restated)

`frame type (1) ‖ stream ID (2) ‖ index (1) ‖ count (1) ‖ length (2) ‖ data` inside one
Noise transport message. Types: 0x01 envelope data, 0x02 padding, 0x03 summary
vector, 0x04 envelope request, 0x05 beacon refresh. Standard mode: frame up to
MTU − 3 − 16. High-Risk: exactly 180 bytes on air, 6 frames per neighbour per 2 s.

Summary vector data = count (2) ‖ count × (message ID 16 ‖ tag 16 ‖ expiry 4).
Envelope request data = count (2) ‖ count × message ID (16).

### A.7 Broadcast associated data

For Broadcast envelopes the associated data is the header with TTL **and the proof
field** zeroed, because the PoW nonce is searched after sealing (PoW covers header
with TTL 0 and proof zeroed ‖ nonce ‖ payload). Tampering with the nonce fails the
PoW check; tampering with anything else fails the AEAD.

### A.8 In-person pairing over envelopes

In-person (IK) handshake messages travel as Handshake envelopes with TTL 1
(kind 4 = IK message 1, kind 5 = IK message 2) addressed to
tag = HMAC-SHA256(token, "pair-tag-v1")[0..16], key = HKDF(tag, "hs-v2"). This
reuses the envelope path inside the Noise_NN link instead of a new frame type.

### A.9 Group members who are not contacts

Direct envelopes between group members who are not contacts use pairwise tag key
= HKDF-SHA256(X25519(own Noise static key, member's Noise static key),
"member-tag-v1"). Membership messages therefore carry each member's Noise static
key and binding signature (256 bytes per member, 15 members per message; larger
groups send several Add messages).

### A.10 Remote pairing binding

Bob's XX message 2 payload = card length (2) ‖ card ‖ pseudonym pk (32) ‖
signature over "xx-bind-v1" ‖ h1, where h1 is the handshake hash after message 1.
Message 3 goes to the reply tag.

## B. Section fixes (R4)

- §16 constants: protocol version 0x02; LoRa bucket 96, LoRa text ≤ 61 bytes;
  cover traffic 6 × 180-byte frames per 2 s; rate limit 20 frames/s and 50 KB/s;
  wire margin 192.
- §14 `relay.db.envelopes`: drop `priority`; index `envelopes_evict` on
  `(mine, recipient_tag, received_at)`; add `superseded_by BLOB` (A.5).
- §15.2 step 4: Bob's XX message 2 payload = his contact card ‖ pseudonym pk (32) ‖
  Ed25519 signature by the post's pseudonym key over "xx-bind-v1" ‖ handshake hash.
  Alice verifies the pseudonym pk equals the post's before continuing.
- §15.3: groups have no Delivered state: Queued → Sent → Read (first read receipt).
- §12.2 handoff bundle order: own queued envelopes, then newest.

## C. Keystore (R5)

`trait Keystore` gains `fn hmac(&self, slot: KeySlot, data: Vec<u8>) -> Result<Vec<u8>, KeystoreError>`
(hardware-held HMAC-SHA256 key; Android StrongBox/TEE; iOS Keychain item). Used only
for the duress verifier. Slots: `RelayDb`, `MessageDb`, `Duress`.

## D. Licences (R6)

- QR decoding happens in the Rust core with `rqrr` (MIT/Apache-2.0) from luma frames
  supplied by the camera; ML Kit and `expo-camera` barcode scanning are banned.
- The Meshtastic bridge hand-encodes the handful of protobuf messages it needs
  (`ToRadio.packet`, `MeshPacket.decoded`, `Data{portnum=256, payload}`); no
  GPL-3.0 `.proto` files enter the repository.

## E. Architecture changes for the Expo app

- **Core engine is sans-IO** (D29): a deterministic state machine receiving
  `(now, event)` and emitting actions to `TransportHost` / `EventSink`. The async
  actor of §13 wraps it on native; the WASM build wraps it synchronously; the
  simulator drives it with virtual time.
- **Storage trait** (D30): SQLCipher (`message.db`, `relay.db`) on device; an
  in-memory backend for the browser harness and simulator.
- **App** (D27): Expo / React Native (TypeScript), Android first, iOS deferred. Radio,
  keystore, FLAG_SECURE and RGBA image view are local Expo Kotlin modules.
- **Bindings** (D28): UniFFI Kotlin bindings called from the Expo Kotlin module
  (no JSI/C++); `wasm-bindgen` for the browser harness.
- **Images** (D31): received images are drawn from raw RGBA (Android `Bitmap.copyPixelsFromBuffer`
  view; `<canvas>` putImageData on web). React Native `<Image>` is never given
  received bytes.
- **Release builds** (D32): a config plugin removes `android.permission.INTERNET`
  from the merged manifest; CI checks the release manifest. `expo-updates`,
  `expo-dev-client` in release, and telemetry packages are banned.
- **Browser harness**: `app/` on web renders N phone frames, each with its own
  WASM core; a simulated radio ("ether") connects them by distance (drag to move),
  with jamming and per-phone power toggles.

## F. Decision log additions

| # | Decision | Chosen | Rejected alternative and reason |
| --- | --- | --- | --- |
| D17 | Wire format | Revision 2 (A.1), version byte 0x02 | Revision 1: fragment fields duplicated link framing |
| D18 | Tag time base | Expiry hour | 15-min epoch: stored messages stop matching after ±15 min |
| D19 | Envelope confidentiality | Outer XChaCha20-Poly1305, header as AD | Bare Olm/Megolm: ratchet keys, session IDs and identity keys link messages for relays |
| D20 | Relay ack | Hash commitment in proof field | Unauthenticated acks: any relay could delete others' messages |
| D21 | Relay priority | None (local only) | Priority flag: unverifiable by relays, abusable |
| D22 | Cover traffic | 6 × 180-byte frames per 2 s; text only | 1 × 256-byte packet: cannot carry any real envelope |
| D23 | Android key wrapping | StrongBox/TEE AES-256-GCM | P-256: Keystore cannot encrypt with EC; ECDH needs API 31 |
| D24 | Duress verifier | Hardware HMAC of Argon2id output | Stored hash: 6-digit PIN brute-forceable offline |
| D25 | QR decoding | `rqrr` in Rust | ML Kit: closed source, pulls telemetry and INTERNET permission |
| D26 | LoRa protobuf | Hand-written minimal encoding | Meshtastic `.proto` files: GPL-3.0 conflicts with D6 |
| D27 | App UI | Expo / React Native, Android first | Kotlin + Compose: one platform only, no browser harness |
| D28 | Bindings | UniFFI Kotlin via Expo module; wasm-bindgen for web | uniffi-bindgen-react-native JSI: extra C++ layer |
| D29 | Core structure | Sans-IO engine + thin async actor | Actor-only: untestable time, no deterministic simulator |
| D30 | Storage | Trait; SQLCipher on device, memory in harness | SQLCipher everywhere: unavailable in WASM |
| D31 | Image display | Raw RGBA views | RN `<Image>`: routes received bytes to OS codecs |
| D32 | INTERNET permission | Stripped from release manifest | Kept: violates D7 |
| D33 | Wire margin | 192 bytes | 160: Olm PreKey messages overflow |
| D34 | Database layout | Typed records in a key-value table per SQLCipher database | §14 relational schema: duplicated for the in-memory (WASM) backend |
| D35 | Binding surface | JSON method calls + JSON events (one `EventSink::emit`) | Typed UniFFI records: two binding layers (Kotlin, WASM) to keep in sync |
| D36 | Core runtime | Host-driven `tick()` (100 ms) on one mutex-guarded engine | Tokio actor: no tokio on wasm32, harder to simulate |
| D37 | App navigation | In-memory stack per phone (no Expo Router) | Expo Router: URL routing cannot host several independent phones in the browser harness |
| D38 | Link pacing | Senders pace to 18 frames/s; receivers rate-limit after Noise decryption | Dropping before decryption desynchronises Noise nonces and kills the link |
| D39 | GATT data service | Fixed data-service UUID visible only after connecting; advertising carries the mode's UUIDs | Rotating GATT UUID: Android cannot re-register services every 2 s without dropping links |
| D40 | Attestation crypto | Pure-Rust X.509 walk + RustCrypto ECDSA/RSA (verify only), compiled only for Android | x509-parser + ring: C code, does not build for wasm32 |
| D41 | Simultaneous connections | `on_connected(peer, mtu, outgoing)`; duplicate links resolved by lower ephemeral ID | Inferring the Noise role: both phones can believe they initiated |
