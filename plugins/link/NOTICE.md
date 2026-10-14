# MeshGRID: third-party software

MeshGRID (this add-on, Apache-2.0) bundles:

| Component | License | Source |
|---|---|---|
| Nym SDK and client crates | Apache-2.0 | https://github.com/nymtech/nym at the commit pinned in `build-addon.sh` (`NYM_COMMIT`), plus `native/nym-socks5.patch` |
| libXray | MIT | https://github.com/XTLS/libXray at the commit pinned in `build-addon.sh` (`LIBXRAY_COMMIT`) |
| Xray-core (inside libXray) | MPL-2.0 | https://github.com/XTLS/Xray-core at the version pinned in libXray's `go.mod`; unmodified |
| Rust crates (635, resolved by `native/Cargo.lock`) | MIT, Apache-2.0, BSD-3-Clause, ISC, Unicode-3.0, Unlicense, MPL-2.0 or compatible; none GPL | crates.io |

Each component's license text ships in its source repository.
