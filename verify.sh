#!/bin/sh
# Is this Meshtrike / MeshGRID APK the real one? One command:
#
#   sh verify.sh meshtrike.apk meshgrid.apk
#
# Prints REAL or NOT REAL for each file. It checks, with whatever tools you have:
#   1. apksigner (Android SDK): the APK is signed by the Meshtrike release key.
#      This works on any copy, however you got it, even with no other files.
#   2. minisign: the APK's SHA-256 is listed in SHA256SUMS, and SHA256SUMS
#      carries a valid Meshtrike signature (SHA256SUMS + SHA256SUMS.minisig
#      must sit next to the APK, as downloaded from the release page).
# Any failed check means NOT REAL. The pinned values below are also published
# on https://meshtrike0.github.io/verify.html, in the whitepaper and in every
# release; a fake would have to change all of them at once.
set -u

APK_CERT_SHA256="AB:09:7B:F4:3C:9B:63:0F:06:67:02:E0:61:9A:60:43:05:13:6A:D2:D6:4B:AA:38:E7:62:2F:AF:17:09:AD:3E"   # APK signing certificate, SHA-256
MINISIGN_PUBKEY="RWSC18AkPYb7l9TWSVy8CzBHwL/ZqkM7aXeAqeLlLHhsKDeoAaiZ463t"   # minisign public key (base64 line)

if [ $# -lt 1 ]; then
  echo "Usage: sh verify.sh <apk> [more apks]"
  exit 2
fi
case "$APK_CERT_SHA256$MINISIGN_PUBKEY" in
  *NOT_YET_PUBLISHED*)
    echo "This copy of verify.sh has no release keys yet: they are published with the first signed release."
    exit 2 ;;
esac

norm() { tr -d ' :\n' | tr 'A-F' 'a-f'; }
WANT_CERT=$(printf '%s' "$APK_CERT_SHA256" | norm)

sha256() {
  if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else sha256sum "$1" | cut -d' ' -f1; fi
}

APKSIGNER=$(command -v apksigner 2>/dev/null || true)
if [ -z "$APKSIGNER" ]; then
  for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Library/Android/sdk" "$HOME/Android/Sdk" \
             /opt/homebrew/share/android-commandlinetools /usr/local/share/android-commandlinetools; do
    [ -n "$sdk" ] || continue
    found=$(ls "$sdk"/build-tools/*/apksigner 2>/dev/null | tail -1)
    [ -n "$found" ] && { APKSIGNER=$found; break; }
  done
fi
MINISIGN=$(command -v minisign 2>/dev/null || true)

status=0
for apk in "$@"; do
  name=$(basename "$apk")
  if [ ! -f "$apk" ]; then echo "NOT FOUND  $apk"; status=1; continue; fi
  passed=0; failed=0; notes=""

  # 1. Who signed the APK (the check Android itself relies on for updates).
  if [ -n "$APKSIGNER" ]; then
    certs=$("$APKSIGNER" verify --print-certs "$apk" 2>/dev/null)
    if [ $? -eq 0 ]; then
      got=$(printf '%s\n' "$certs" | sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | head -1 | norm)
      signers=$(printf '%s\n' "$certs" | grep -c '^Signer #[0-9]* certificate SHA-256 digest')
      if [ "$got" = "$WANT_CERT" ] && [ "$signers" -eq 1 ]; then
        passed=$((passed + 1)); notes="$notes signing-key:ok"
      else
        failed=$((failed + 1)); notes="$notes signing-key:WRONG"
      fi
    else
      failed=$((failed + 1)); notes="$notes signature:BROKEN"
    fi
  fi

  # 2. The file is one of the files the release key signed.
  dir=$(dirname "$apk")
  if [ -n "$MINISIGN" ] && [ -f "$dir/SHA256SUMS" ] && [ -f "$dir/SHA256SUMS.minisig" ]; then
    if "$MINISIGN" -Vq -P "$MINISIGN_PUBKEY" -m "$dir/SHA256SUMS" -x "$dir/SHA256SUMS.minisig" >/dev/null 2>&1; then
      if cut -d' ' -f1 "$dir/SHA256SUMS" | grep -qx "$(sha256 "$apk")"; then
        passed=$((passed + 1)); notes="$notes checksum:ok"
      else
        failed=$((failed + 1)); notes="$notes checksum:NOT-LISTED"
      fi
    else
      failed=$((failed + 1)); notes="$notes SHA256SUMS-signature:BAD"
    fi
  fi

  if [ $failed -gt 0 ]; then
    echo "NOT REAL   $name  ($notes ) - do not install it, delete it, do not pass it on"
    status=1
  elif [ $passed -gt 0 ]; then
    echo "REAL       $name  ($notes )"
  else
    echo "CAN'T TELL $name  - install apksigner (Android SDK build-tools) or minisign, or keep SHA256SUMS and SHA256SUMS.minisig next to the APK"
    [ $status -eq 0 ] && status=3
  fi
done
exit $status
