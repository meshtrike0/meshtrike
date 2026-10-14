#!/usr/bin/env bash
# Build and sign a Meshtrike release. Run YOURSELF in Terminal (gpg and
# minisign ask for their passphrases). Output: dist/<version>/ with
#   meshtrike.apk, meshgrid.apk                  signed with the release key
#                                                (stable names, so the website and app can
#                                                link to .../releases/latest/download/<name>)
#   SHA256SUMS                                   hash of each APK
#   SHA256SUMS.asc                               OpenPGP signature (like Tor)
#   SHA256SUMS.minisig                           minisign signature
#   PUBLIC-FINGERPRINTS.txt                      what users compare against
#   verify.sh                                    the one-command check for users
# Nothing is uploaded; publishing is a separate, deliberate step.
# SKIP_BUILD=1 ci/release.sh re-signs the APKs already built (e.g. after a
# passphrase prompt timed out) without rebuilding.
set -euo pipefail
cd "$(dirname "$0")/.."
KEYS="${MESHTRIKE_KEYS:-$HOME/MeshtrikeKeys}"
for f in meshtrike-release.p12 meshtrike-release.minisign.key meshtrike-release.minisign.pub PUBLIC-FINGERPRINTS.txt; do
  [ -f "$KEYS/$f" ] || { echo "Missing $KEYS/$f - run ci/make-release-keys.sh first"; exit 1; }
done

APP_VER=$(python3 -c 'import json;print(json.load(open("app/app.json"))["expo"]["version"])')
APP_CODE=$(python3 -c 'import json;print(json.load(open("app/app.json"))["expo"]["android"]["versionCode"])')
GRID_VER=$(sed -n "s/.*versionName '\(.*\)'.*/\1/p" plugins/link/build.gradle)
OUT="dist/meshtrike-$APP_VER-$APP_CODE"
mkdir -p "$OUT"

if [ "${SKIP_BUILD:-0}" = 1 ]; then echo "Skipping build: using the APKs already built."; else MESHTRIKE_RELEASE=1 ci/build-android.sh; fi

cp app/android/app/build/outputs/apk/release/app-release.apk "$OUT/meshtrike.apk"
cp plugins/link/build/outputs/apk/release/link-arm64-v8a-release.apk "$OUT/meshgrid.apk"

# Every APK must carry exactly the published release certificate.
APKSIGNER="$(ls -d "${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"/build-tools/* | tail -1)/apksigner"
WANT=$(awk '/Android APK signing certificate/{getline; gsub(/[ :]/,""); print tolower($0)}' "$KEYS/PUBLIC-FINGERPRINTS.txt")
for apk in "$OUT"/*.apk; do
  GOT=$("$APKSIGNER" verify --print-certs "$apk" | awk -F': ' '/Signer #1 certificate SHA-256 digest/{print tolower($2)}')
  [ "$GOT" = "$WANT" ] || { echo "WRONG SIGNING KEY on $apk ($GOT)"; exit 1; }
  echo "OK  release certificate on $(basename "$apk")"
done

(cd "$OUT" && shasum -a 256 *.apk > SHA256SUMS)
# gpg asks for its passphrase in this Terminal (pinentry-curses needs to know which one).
export GPG_TTY="$(tty)"
gpg-connect-agent updatestartuptty /bye >/dev/null 2>&1 || true
echo "Next: your OpenPGP passphrase (gpg), then your minisign password."
gpg --yes --local-user "Meshtrike Releases" --armor --detach-sign --output "$OUT/SHA256SUMS.asc" "$OUT/SHA256SUMS"
minisign -S -s "$KEYS/meshtrike-release.minisign.key" -m "$OUT/SHA256SUMS" -x "$OUT/SHA256SUMS.minisig" \
  -t "Meshtrike $APP_VER ($APP_CODE), MeshGRID $GRID_VER"
cp "$KEYS/PUBLIC-FINGERPRINTS.txt" "$KEYS/meshtrike-release-openpgp.asc" "$KEYS/meshtrike-release.minisign.pub" verify.sh "$OUT/"

# Check our own work the way a user would.
(cd "$OUT" && shasum -a 256 -c SHA256SUMS)
gpg --verify "$OUT/SHA256SUMS.asc" "$OUT/SHA256SUMS"
minisign -V -p "$KEYS/meshtrike-release.minisign.pub" -m "$OUT/SHA256SUMS" -x "$OUT/SHA256SUMS.minisig"
# verify.sh must carry this release's keys (fill them in before releasing).
sh verify.sh "$OUT"/*.apk
echo
echo "Release ready in $OUT"
