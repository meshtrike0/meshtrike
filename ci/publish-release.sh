#!/usr/bin/env bash
# Upload a release built by ci/release.sh to GitHub as a DRAFT.
# Nothing is public until the draft is reviewed and published:
#   gh release edit v<version> --repo meshtrike0/meshtrike --draft=false
#
#   ci/publish-release.sh dist/meshtrike-<version>-<code>
set -euo pipefail
cd "$(dirname "$0")/.."
REPO=meshtrike0/meshtrike
DIR="${1:?usage: ci/publish-release.sh dist/meshtrike-<version>-<code>}"

APP_VER=$(python3 -c 'import json;print(json.load(open("app/app.json"))["expo"]["version"])')
APP_CODE=$(python3 -c 'import json;print(json.load(open("app/app.json"))["expo"]["android"]["versionCode"])')
GRID_VER=$(sed -n "s/.*versionName '\(.*\)'.*/\1/p" plugins/link/build.gradle)
TAG="v$APP_VER"
FILES=(meshtrike.apk meshgrid.apk SHA256SUMS SHA256SUMS.asc SHA256SUMS.minisig PUBLIC-FINGERPRINTS.txt
       meshtrike-release-openpgp.asc meshtrike-release.minisign.pub verify.sh)
for f in "${FILES[@]}"; do [ -f "$DIR/$f" ] || { echo "Missing $DIR/$f (run ci/release.sh first)"; exit 1; }; done

# The published fingerprints must already be in the repository (ci/publish-fingerprints.py),
# and the commit being tagged must be the one on GitHub.
if grep -q NOT_YET_PUBLISHED verify.sh website/js/verify.js; then
  echo "Fingerprints are not filled in yet: run ci/publish-fingerprints.py, commit and push first."; exit 1
fi
HEAD=$(git rev-parse HEAD)
REMOTE=$(gh api "repos/$REPO/commits/main" --jq .sha)
[ "$HEAD" = "$REMOTE" ] || { echo "Local HEAD $HEAD is not GitHub main $REMOTE: push first."; exit 1; }

# Check the files exactly as a user would, with the verify.sh that ships with them.
(cd "$DIR" && sh ./verify.sh meshtrike.apk meshgrid.apk)

fp() { awk -v k="$1" 'index($0,k){getline; gsub(/^ +/,""); print; exit}' "$DIR/PUBLIC-FINGERPRINTS.txt"; }
CERT=$(fp "Android APK signing certificate")
PGP=$(fp "OpenPGP key fingerprint")
MINI=$(fp "minisign public key")
size() { du -m "$DIR/$1" | cut -f1; }

NOTES=$(mktemp)
cat > "$NOTES" <<EOF
**Meshtrike $APP_VER** (build $APP_CODE) and **MeshGRID $GRID_VER**. Android 10 or newer.

| File | What it is |
|---|---|
| [\`meshtrike.apk\`](https://github.com/$REPO/releases/download/$TAG/meshtrike.apk) (~$(size meshtrike.apk) MB) | The messenger. No internet permission. |
| [\`meshgrid.apk\`](https://github.com/$REPO/releases/download/$TAG/meshgrid.apk) (~$(size meshgrid.apk) MB) | Optional add-on: Wi-Fi Direct and the private internet route over the Nym mixnet. |

One APK works on any number of phones: share it phone to phone. Every copy can be checked.

### Check that your APK is real
- **In the browser (phone or computer):** https://meshtrike0.github.io/verify.html, pick the APK; it never leaves your device.
- **One command:** download \`verify.sh\` from this release, then \`sh verify.sh meshtrike.apk meshgrid.apk\`, which prints REAL or NOT REAL.
- **By hand, like Tor:** \`SHA256SUMS\` is signed with OpenPGP (\`SHA256SUMS.asc\`) and minisign (\`SHA256SUMS.minisig\`).

| Release key | Fingerprint |
|---|---|
| APK signing certificate (SHA-256) | \`$CERT\` |
| OpenPGP release key | \`$PGP\` |
| minisign public key | \`$MINI\` |

The same values are on the verify page, in the README, in the whitepaper and inside the app (Settings → About & limits → Verify this app). If anything differs, do not install it.

Signed by the Meshtrike team.
EOF

ASSETS=(); for f in "${FILES[@]}"; do ASSETS+=("$DIR/$f"); done
gh release create "$TAG" --repo "$REPO" --target "$HEAD" --draft \
  --title "Meshtrike $APP_VER" --notes-file "$NOTES" "${ASSETS[@]}"
rm -f "$NOTES"
echo
echo "Draft release $TAG uploaded. Review it on https://github.com/$REPO/releases, then publish:"
echo "  gh release edit $TAG --repo $REPO --draft=false"
