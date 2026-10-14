#!/usr/bin/env bash
# Section 20: every core crate forbids unsafe code (FFI glue excepted).
set -euo pipefail
cd "$(dirname "$0")/.."
bad=0
for c in core/*/; do
  name=$(basename "$c")
  [ "$name" = ffi ] && continue
  grep -q '#!\[forbid(unsafe_code)\]' "$c/src/lib.rs" || { echo "missing forbid(unsafe_code): $c"; bad=1; }
done
[ $bad -eq 0 ] && echo "OK: forbid(unsafe_code) in all core crates"
exit $bad
