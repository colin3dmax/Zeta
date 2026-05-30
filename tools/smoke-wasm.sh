#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
WASM_TARGET="${ZETA_WASM_TARGET:-wasm32-unknown-unknown}"
WASI_TARGET="${ZETA_WASI_TARGET:-wasm32-wasip1}"

cd "$ROOT"

cargo build --release --target "$WASM_TARGET" --lib
node tools/smoke-wasm.cjs "target/$WASM_TARGET/release/zeta.wasm"

if rustup target list --installed | grep -qx "$WASI_TARGET"; then
  cargo build --release --target "$WASI_TARGET" --lib
  if [ -f "target/$WASI_TARGET/release/zeta.wasm" ]; then
    node tools/smoke-wasm.cjs "target/$WASI_TARGET/release/zeta.wasm"
  fi
else
  echo "skip $WASI_TARGET smoke: Rust target is not installed"
fi
