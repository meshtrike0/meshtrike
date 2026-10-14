#!/usr/bin/env bash
# Build the Android release APK: Rust core (.so) → UniFFI Kotlin bindings → Expo prebuild → Gradle.
set -euo pipefail
cd "$(dirname "$0")/.."
source "$HOME/.cargo/env"
REPO="$(pwd)"
# Keep build-machine paths (user name, disk name) out of the published binaries.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME=/home --remap-path-prefix=$REPO=/src"
export ANDROID_HOME="${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | tail -1)}"
# MESHTRIKE_RELEASE=1: sign both APKs with the private release key (created by
# ci/make-release-keys.sh). Its passphrase comes from the macOS Keychain.
if [ "${MESHTRIKE_RELEASE:-0}" = 1 ]; then
  KEYS="${MESHTRIKE_KEYS:-$HOME/MeshtrikeKeys}"
  export MESHTRIKE_KEYSTORE="$KEYS/meshtrike-release.p12"
  [ -f "$MESHTRIKE_KEYSTORE" ] || { echo "No release key at $MESHTRIKE_KEYSTORE (run ci/make-release-keys.sh)"; exit 1; }
  MESHTRIKE_STORE_PASSWORD="$(security find-generic-password -a meshtrike -s meshtrike-release-keystore -w)"
  export MESHTRIKE_STORE_PASSWORD
fi
# Built outside the project folder: bundled C libraries (OpenSSL in SQLCipher) record their build path.
CARGO_TARGET_DIR=/tmp/meshtrike-build/core cargo ndk -t arm64-v8a -t x86_64 -P 29 -o app/modules/mesh-core/android/src/main/jniLibs build -p mesh-ffi --release
cargo build -q -p mesh-ffi
cargo run -q -p mesh-ffi --bin uniffi-bindgen -- generate --library target/debug/libmesh_ffi.dylib \
  --language kotlin --out-dir app/modules/mesh-core/android/src/main/java --no-format
cd app
CI=1 npx expo prebuild --platform android --no-install
echo "sdk.dir=$ANDROID_HOME" > android/local.properties
(cd android && ./gradlew assembleRelease -q)
ls -la android/app/build/outputs/apk/release/app-release.apk
# Meshtrike Link add-on (Wi-Fi Direct + internet route), signed with the same
# key: its signature permissions depend on it.
../plugins/link/build-addon.sh
../ci/check-release-manifest.sh
