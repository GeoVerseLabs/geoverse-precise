#!/usr/bin/env bash
# Build the WASM module and generate JS glue into packages/geoverse-precise/wasm.
#
# Requirements:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <same as crates/wasm Cargo.toml>
#   (optional) wasm-opt from binaryen
#
# Environment overrides (used in restricted build environments):
#   CARGO            cargo binary (default: cargo)
#   CARGO_EXTRA      extra cargo args, e.g. "-Zbuild-std=std,panic_abort"
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/packages/geoverse-precise/wasm"
CARGO="${CARGO:-cargo}"

cd "$ROOT"
# shellcheck disable=SC2086
"$CARGO" build -p geoverse-precise-wasm --release --target wasm32-unknown-unknown ${CARGO_EXTRA:-}

WASM="$ROOT/target/wasm32-unknown-unknown/release/geoverse_precise_wasm.wasm"
rm -rf "$OUT" && mkdir -p "$OUT"
wasm-bindgen "$WASM" --out-dir "$OUT" --target web --out-name geoverse_precise_wasm

# wasm-opt is optional. Binaryen before 116 renumbers tables without fixing the
# export that points at them, which silently hands the JS glue the funcref table
# (fixed size) in place of the growable externref table: the module then fails to
# initialise with "failed to grow table by 4". Skip those versions rather than
# ship a broken module.
WASM_OPT_OK=0
if command -v wasm-opt >/dev/null 2>&1; then
  WASM_OPT_VER="$(wasm-opt --version 2>/dev/null | grep -oE '[0-9]+' | head -1 || echo 0)"
  if [ "${WASM_OPT_VER:-0}" -ge 116 ]; then
    WASM_OPT_OK=1
  else
    echo "wasm-opt $WASM_OPT_VER is older than 116 (mis-handles externref tables); skipping"
  fi
fi

if [ "$WASM_OPT_OK" = "1" ] && [ "${SKIP_WASM_OPT:-0}" != "1" ]; then
  wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    --enable-mutable-globals --enable-multivalue --enable-reference-types \
    "$OUT/geoverse_precise_wasm_bg.wasm" -o "$OUT/geoverse_precise_wasm_bg.wasm" \
    || echo "wasm-opt failed; keeping unoptimised module"
fi
ls -la "$OUT"
