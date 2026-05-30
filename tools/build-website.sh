#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

cd "$ROOT"
cargo build --release --target wasm32-unknown-unknown --lib
WASM_HASH="$(shasum -a 256 target/wasm32-unknown-unknown/release/zeta.wasm | cut -c 1-16)"
WASM_FILE="zeta-$WASM_HASH.wasm"
rm -f website/public/zeta-*.wasm
cp target/wasm32-unknown-unknown/release/zeta.wasm "website/public/$WASM_FILE"
cp target/wasm32-unknown-unknown/release/zeta.wasm website/public/zeta.wasm
printf 'export const ZETA_WASM_URL = "/%s";\n' "$WASM_FILE" > website/src/wasm-url.js

cd "$ROOT/website"
npm run build

rm -rf "$ROOT/website/dist/docs"
mkdir -p "$ROOT/website/dist/docs"
cp -R "$ROOT/docs/." "$ROOT/website/dist/docs/"
