#!/usr/bin/env bash
# Create Meshtrike's release keys. Run this YOURSELF in Terminal: every
# passphrase is typed by you and never written to a file or shown to anyone.
#
#   1. Android release key  - signs the Meshtrike and MeshGRID APKs (both must
#      share it: MeshGRID only serves an app signed with the same key).
#   2. OpenPGP release key   - signs SHA256SUMS (.asc), like Tor's releases.
#   3. minisign release key  - a second, independent signature (.minisig).
#
# Keys live in $MESHTRIKE_KEYS (default ~/MeshtrikeKeys), outside the project,
# so they can never be committed. Afterwards: copy that folder AND your
# passphrases to an encrypted offline backup (e.g. an encrypted USB stick).
# If the Android key is lost, no update can ever be published to installed
# copies; if it leaks, anyone can publish "updates" - guard it.
set -euo pipefail

KEYS="${MESHTRIKE_KEYS:-$HOME/MeshtrikeKeys}"
mkdir -p "$KEYS"
chmod 700 "$KEYS"
cd "$KEYS"
echo "Keys folder: $KEYS"
echo

# ----- 1. Android release key -----
if [ -f meshtrike-release.p12 ]; then
  echo "Android key exists, keeping it."
else
  echo "== Android release key (RSA 4096, valid 30 years) =="
  echo "Choose a strong passphrase (20+ characters). You will type it twice."
  keytool -genkeypair -v \
    -keystore meshtrike-release.p12 -storetype PKCS12 \
    -alias meshtrike -keyalg RSA -keysize 4096 -validity 10950 \
    -dname "CN=Meshtrike Release, O=Meshtrike"
  chmod 600 meshtrike-release.p12
  echo
  echo "Store the same passphrase in your macOS Keychain, so release builds can"
  echo "sign without it ever being typed into a file or a chat (type it again):"
  security add-generic-password -U -a meshtrike -s meshtrike-release-keystore -w
fi
echo

# ----- 2. OpenPGP release key -----
UID_NAME="Meshtrike Releases"
if gpg --list-secret-keys "$UID_NAME" >/dev/null 2>&1; then
  echo "OpenPGP key exists, keeping it."
else
  echo "== OpenPGP release key (Ed25519, signing only, expires in 3 years) =="
  echo "gpg will ask for a passphrase in a pop-up."
  gpg --quick-generate-key "$UID_NAME" ed25519 sign 3y
fi
gpg --armor --export "$UID_NAME" > meshtrike-release-openpgp.asc
echo

# ----- 3. minisign release key -----
if [ -f meshtrike-release.minisign.key ]; then
  echo "minisign key exists, keeping it."
else
  echo "== minisign release key (Ed25519) =="
  minisign -G -p meshtrike-release.minisign.pub -s meshtrike-release.minisign.key
  chmod 600 meshtrike-release.minisign.key
fi
echo

# ----- Public fingerprints (safe to publish) -----
{
  echo "Meshtrike release keys - public fingerprints"
  echo
  echo "Android APK signing certificate (SHA-256):"
  keytool -list -v -keystore meshtrike-release.p12 -storetype PKCS12 -alias meshtrike \
    -storepass "$(security find-generic-password -a meshtrike -s meshtrike-release-keystore -w)" \
    | awk -F': ' '/SHA256:/{print "  " $2}'
  echo
  echo "OpenPGP key fingerprint:"
  gpg --with-colons --fingerprint "$UID_NAME" | awk -F: '/^fpr/{print "  " $10; exit}'
  echo
  echo "minisign public key:"
  tail -1 meshtrike-release.minisign.pub | sed 's/^/  /'
} | tee PUBLIC-FINGERPRINTS.txt
echo
echo "Done. Public parts (safe to share): PUBLIC-FINGERPRINTS.txt,"
echo "meshtrike-release-openpgp.asc, meshtrike-release.minisign.pub."
echo "Now back up $KEYS and your passphrases offline."
