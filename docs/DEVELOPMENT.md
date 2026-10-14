# Developing Meshtrike

Specification: `spec/blueprint-v1.0.md` (base), `spec/blueprint-v1.01-revisions.txt`,
and `spec/blueprint-v1.02-amendments.md` (wire format revision 2, decision log
D17–D36). Where they conflict, v1.02 wins.

## Layout

| Path | What |
| --- | --- |
| `core/constants` | Every protocol constant (§16) |
| `core/crypto` | Tags, envelope AEAD (XChaCha20-Poly1305), Noise NN/IK/XX (`snow`), PoW, safety numbers; Olm/Megolm via `vodozemac` |
| `core/wire` | Envelope, inner plaintext, link frames, summary vectors — fixed binary layouts |
| `core/identity` | Account, contact cards, pairing QR, community QR |
| `core/discovery` | Epochs, beacons, presence tags, Public / Community / Listen-only advertising plans |
| `core/routing` | Relay store: fair-share eviction, per-tag quota, ack commitments, seen filter |
| `core/abuse`, `core/countermeasures` | Rate limits, PoW difficulty; TX power, interference detector, Listen-only policy |
| `core/handoff`, `core/media` | RaptorQ + QR handoff, QR decoding (`rqrr`); Rust-only JPEG encode/decode, sanitisers |
| `core/storage`, `core/lock` | SQLCipher / in-memory key-value stores; Argon2id PIN, duress, wipe policy |
| `core/api` | The engine (sans-IO, host-driven), JSON method surface, Android attestation verifier, LoRa encoding |
| `core/ffi` | UniFFI binding for Android (SQLCipher on disk, hardware keystore via `NativeHost`) |
| `core/wasm` | The same engine compiled to WebAssembly for the browser harness |
| `simulator` | Headless multi-node simulator and `mesh-sim` delivery benchmark |
| `fuzz` | cargo-fuzz targets for every parser plus a live-engine target |
| `app` | Expo / React Native app (Android) and the multi-phone browser harness (web) |
| `app/modules/mesh-core` | Kotlin: BLE GATT transport, AndroidKeyStore (StrongBox/TEE), relay service, CameraX QR, Meshtastic bridge, RGBA view |
| `ci` | Pipeline and policy checks (no INTERNET, no telemetry, `forbid(unsafe_code)`), release keys and signed releases |
| `plugins/link` | MeshGRID add-on (Android): Wi-Fi Direct link and the private internet route over the Nym mixnet |
| `website` | The website (GitHub Pages), whitepaper and the release verification page |
| `verify.sh` | One-command check that an APK is a real Meshtrike / MeshGRID release |

## Running

Prerequisites: Rust (pinned in `rust-toolchain.toml`), Node 20+, Android SDK + NDK 27 for device builds.

```bash
# Core tests (162 tests incl. 27 end-to-end mesh scenarios and 11 internet-route tests)
cargo test --workspace --release --all-features

# Delivery benchmark: 50 nodes, 1,000 messages, walking-speed mobility
cargo run --release -p mesh-simulator --bin mesh-sim -- 50 1000 150 42

# Browser harness: several Android phone frames, each running the real core in WASM,
# connected by a simulated Bluetooth radio (drag phones, place jammers, fast-forward time)
cd app && npm install && npm run build:core && npx expo start --web --port 8088
node e2e/flows.mjs  http://localhost:8088/ /tmp   # 10 UI flows
node e2e/flows2.mjs http://localhost:8088/ /tmp   # 9 more (groups, media, handoff, …)

# Android release APK (no INTERNET permission)
cd .. && cargo ndk -t arm64-v8a -t x86_64 -P 29 -o app/modules/mesh-core/android/src/main/jniLibs build -p mesh-ffi --release
cargo build -p mesh-ffi && cargo run -p mesh-ffi --bin uniffi-bindgen -- generate --library target/debug/libmesh_ffi.dylib \
  --language kotlin --out-dir app/modules/mesh-core/android/src/main/java --no-format
cd app && npx expo prebuild --platform android && cd android && ./gradlew assembleRelease
../../ci/check-release-manifest.sh
python3 ../e2e/android_drive.py      # drives the installed APK via the accessibility tree

# Everything at once
ci/pipeline.sh
```

## Verified results

| Check | Result |
| --- | --- |
| Rust unit + integration tests | 162 passed (37 suites) |
| Mesh scenarios (simulator) | 27/27, including voice notes, nearby calls, replies, Wi-Fi Direct hand-off and: pairing, delivery + read receipts, 3-hop relay with ack return, data mule, locked-phone acks, duress & wrong-PIN wipe, groups with non-contact members and removal, channels + remote pairing, delayed send, image + file, High-Risk cover traffic, Listen-only, Community mode, jamming + QR handoff, LoRa fallback |
| `mesh-sim` 50 nodes / 1,000 msgs | 100 % delivered within 10 min (4 seeds), median 12–15 s |
| `mesh-sim` 200 nodes / 1,000 msgs | 100 % within 10 min, median 54 s (release criterion ≥ 99 %) |
| Browser harness flows | 19/19 through the real UI and WASM core |
| Fuzzing | 10 targets, no crashes (CI budget: 5 min each) |
| cargo-deny | licences, bans, sources OK |
| Android release APK | builds; no INTERNET permission (`ci/check-release-manifest.sh`) |
