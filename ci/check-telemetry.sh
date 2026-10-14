#!/usr/bin/env bash
# Section 19 / D15: fail if a known analytics, crash-reporting or OTA package appears.
set -euo pipefail
cd "$(dirname "$0")/.."
pattern='sentry|firebase|crashlytics|bugsnag|amplitude|segment-analytics|@segment|mixpanel|datadog|appcenter|expo-updates|expo-insights|posthog|newrelic|instabug|onesignal'
hits=$( (grep -Eio "\"node_modules/($pattern)[^\"]*\"" app/package-lock.json || true; grep -Eio "^name = \"($pattern)[^\"]*\"" Cargo.lock || true) | sort -u)
if [ -n "$hits" ]; then echo "Telemetry/OTA packages found:"; echo "$hits"; exit 1; fi
echo "OK: no telemetry packages"
