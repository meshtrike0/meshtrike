#!/usr/bin/env bash
# Build the MeshGRID add-on (org.meshtrike.link) (Wi-Fi Direct + internet route): Nym (pinned commit + nym-socks5.patch),
# the Rust native library, libXray (pinned commit), then the APK.
set -euo pipefail
cd "$(dirname "$0")"
source "$HOME/.cargo/env"
REPO="$(cd ../.. && pwd)"
# Keep build-machine paths (user name, disk name) out of the published binaries.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME=/home --remap-path-prefix=$REPO=/src"
export ANDROID_HOME="${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | tail -1)}"
export PATH="$HOME/go/bin:$PATH"

NYM_COMMIT=e2307144726c44db51f60e68d59f2f74823dcf11
LIBXRAY_COMMIT=c0cc8d4a86d32e71e2b4e9e6377723597a069a3d

# Nym at a pinned commit, with the SOCKS5 gateway patch applied exactly once.
if [ ! -d native/.nym/.git ]; then
  git init -q native/.nym
  git -C native/.nym remote add origin https://github.com/nymtech/nym
fi
if [ "$(git -C native/.nym rev-parse HEAD 2>/dev/null || true)" != "$NYM_COMMIT" ]; then
  git -C native/.nym fetch -q --depth 1 origin "$NYM_COMMIT"
  git -C native/.nym checkout -q --force "$NYM_COMMIT"
fi
git -C native/.nym reset -q --hard && git -C native/.nym clean -qfd
git -C native/.nym apply ../nym-socks5.patch

# Rust native library (Nym client behind the SOCKS5 endpoint).
# Built outside the project folder: bundled C code records its build path.
(cd native && CARGO_TARGET_DIR=/tmp/meshtrike-build/grid cargo ndk -t arm64-v8a -t x86_64 -P 29 -o ../src/main/jniLibs build --release --lib)

# Xray-core (VLESS + REALITY), pinned.
if [ ! -f libs/libXray.aar ]; then
  # Built in a neutral folder: gomobile records the source folder in the library.
  XB=/tmp/meshgrid-libxray
  if [ ! -d "$XB/.git" ]; then git clone -q https://github.com/XTLS/libXray "$XB"; fi
  git -C "$XB" fetch -q --depth 1 origin "$LIBXRAY_COMMIT" && git -C "$XB" checkout -q --force "$LIBXRAY_COMMIT"
  (cd "$XB" && python3 build/main.py android)
  mkdir -p libs && cp "$(find "$XB" -name 'libXray.aar' | head -1)" libs/libXray.aar
fi

echo "sdk.dir=$ANDROID_HOME" > local.properties
./gradlew assembleRelease -q
ls -la build/outputs/apk/release/link-*-release.apk
