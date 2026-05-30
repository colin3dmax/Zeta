#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
VERSION="$(grep '^version = ' "$ROOT/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
PACKAGE_DIR="$ROOT/dist/packages"
STAGE="$ROOT/dist/zeta-$VERSION-$OS-$ARCH"

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="aarch64" ;;
esac

STAGE="$ROOT/dist/zeta-$VERSION-$OS-$ARCH"

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/docs" "$PACKAGE_DIR"

cd "$ROOT"
cargo build --release
cp "$ROOT/target/release/zeta" "$STAGE/bin/"
cp README.md "$STAGE/"
cp -R docs/. "$STAGE/docs/"

if command -v shasum >/dev/null 2>&1; then
  (cd "$STAGE" && shasum -a 256 bin/zeta > SHA256SUMS)
fi

tar -C "$ROOT/dist" -czf "$PACKAGE_DIR/zeta-$VERSION-$OS-$ARCH.tar.gz" "$(basename "$STAGE")"
echo "$PACKAGE_DIR/zeta-$VERSION-$OS-$ARCH.tar.gz"
