#!/usr/bin/env bash
# Local equivalent of the merge pipeline (section 21).
set -euo pipefail
cd "$(dirname "$0")/.."
source "$HOME/.cargo/env"
step() { echo; echo "=== $* ==="; }
step "fmt";            cargo fmt --all --check
step "clippy";         cargo clippy --workspace --all-targets -- -D warnings
step "tests";          cargo test --workspace --release
step "sqlcipher";      cargo test -p mesh-storage --features sqlcipher
step "cargo deny";     cargo deny check licenses bans sources
step "unsafe";         ci/check-sources.sh
step "telemetry";      ci/check-telemetry.sh
step "simulator 50";   cargo run -q --release -p mesh-simulator --bin mesh-sim -- 50 1000 150 42
step "wasm";           (cd app && npm run -s build:core)
step "typecheck";      (cd app && npx tsc --noEmit)
if curl -s -o /dev/null http://localhost:8088/; then
  step "browser e2e";  (cd app && node e2e/flows.mjs http://localhost:8088/ /tmp)
else
  echo "(browser e2e skipped: start 'npx expo start --web --port 8088' in app/)"
fi
if [ -d app/android ]; then
  step "android release"; (cd app/android && ./gradlew -q assembleRelease)
  step "manifest";        ci/check-release-manifest.sh
fi
echo; echo "PIPELINE PASSED"
