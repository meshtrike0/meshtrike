#!/usr/bin/env bash
# Section 21 step 7: fail if any merged release manifest declares INTERNET.
# Also guards the MeshGRID add-on, the only APK allowed to hold INTERNET.
set -euo pipefail
cd "$(dirname "$0")/../app/android"
found=0
while IFS= read -r m; do
  if grep -q 'android.permission.INTERNET"' "$m" && ! grep -q 'android.permission.INTERNET" tools:node="remove"' "$m"; then
    echo "INTERNET permission found in $m"; found=1
  fi
done < <(find app/build/intermediates -path '*release*' -name AndroidManifest.xml 2>/dev/null)
apk=app/build/outputs/apk/release/app-release.apk
if [ -f "$apk" ]; then
  aapt=$(ls "${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"/build-tools/*/aapt2 | tail -1)
  if "$aapt" dump permissions "$apk" | grep -q 'android.permission.INTERNET'; then
    echo "INTERNET permission present in $apk"; found=1
  fi
else
  echo "no release APK built"; exit 1
fi
# The chat app holds no Wi-Fi permission either: Wi-Fi Direct lives in the add-on.
if "$aapt" dump permissions "$apk" | grep -qE "WIFI_STATE|NEARBY_WIFI_DEVICES|NETWORK_STATE"; then
  echo "Wi-Fi or network-state permission present in $apk"; found=1
fi
# The chat app never opens a network socket; only the MeshGRID add-on may.
if grep -rnE 'java\.net\.(Server|Datagram)?Socket\b|java\.net\.URL\b|HttpURLConnection|okhttp3' \
    ../modules/mesh-core/android/src/main/java/expo ../src 2>/dev/null; then
  echo "socket or HTTP code found in the chat app"; found=1
fi
[ $found -eq 0 ] && echo "OK: release build has no INTERNET permission"

# MeshGRID add-on: exactly INTERNET plus Wi-Fi Direct setup permissions,
# both signature permissions defined, and no library but libXray.
plugin=../../plugins/link
papk=$plugin/build/outputs/apk/release/link-universal-release.apk
if [ -f "$papk" ]; then
  perms=$("$aapt" dump permissions "$papk" | sed -n "s/^uses-permission: name='\([^']*\)'.*/\1/p" | sort | tr '\n' ' ')
  want="android.permission.ACCESS_FINE_LOCATION android.permission.ACCESS_WIFI_STATE android.permission.CHANGE_WIFI_STATE android.permission.INTERNET android.permission.NEARBY_WIFI_DEVICES "
  if [ "$perms" != "$want" ]; then echo "MeshGRID permissions changed: $perms"; found=1; fi
  for p in WIFI_LINK NET_LINK; do
    if ! "$aapt" dump permissions "$papk" | grep -qx "permission: org.meshtrike.permission.$p"; then
      echo "MeshGRID must define org.meshtrike.permission.$p"; found=1
    fi
  done
  deps=$(sed -n '/^dependencies {/,/^}/p' "$plugin/build.gradle" | grep -vE '^dependencies \{|^\}|^\s*//|^\s*$' | tr -d ' ')
  if [ "$deps" != "implementationfiles('libs/libXray.aar')" ]; then echo "MeshGRID dependencies changed: $deps"; found=1; fi
  [ $found -eq 0 ] && echo "OK: MeshGRID holds only INTERNET and Wi-Fi Direct permissions; only libXray"
else
  echo "no MeshGRID APK built"; found=1
fi
exit $found
