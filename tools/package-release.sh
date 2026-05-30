#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
VERSION="$(grep '^version = ' "$ROOT/Cargo.toml" | head -1 | sed 's/version = "\(.*\)"/\1/')"
TARGET=""

usage() {
  cat <<'USAGE'
usage: sh tools/package-release.sh [--target <rust-target>] [--version <version>]

Build and package the Zeta CLI for the current platform or a Rust target
already installed in the local toolchain.

Examples:
  sh tools/package-release.sh
  sh tools/package-release.sh --target x86_64-unknown-linux-gnu
  sh tools/package-release.sh --target aarch64-apple-darwin
  sh tools/package-release.sh --target x86_64-pc-windows-msvc
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --target)
      TARGET="${2:-}"
      [ -n "$TARGET" ] || { usage; exit 2; }
      shift 2
      ;;
    --version)
      VERSION="${2:-}"
      [ -n "$VERSION" ] || { usage; exit 2; }
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

host_os() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux) echo "linux" ;;
    MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
    *) uname -s | tr '[:upper:]' '[:lower:]' ;;
  esac
}

host_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64" ;;
    arm64|aarch64) echo "aarch64" ;;
    *) uname -m ;;
  esac
}

target_os() {
  case "$1" in
    *apple-darwin*) echo "macos" ;;
    *unknown-linux*) echo "linux" ;;
    *pc-windows*) echo "windows" ;;
    *) host_os ;;
  esac
}

target_arch() {
  case "$1" in
    x86_64-*) echo "x86_64" ;;
    aarch64-*|arm64-*) echo "aarch64" ;;
    *) host_arch ;;
  esac
}

cd "$ROOT"

if [ -n "$TARGET" ]; then
  OS="$(target_os "$TARGET")"
  ARCH="$(target_arch "$TARGET")"
  BUILD_DIR="$ROOT/target/$TARGET/release"
  cargo build --release --target "$TARGET"
else
  OS="$(host_os)"
  ARCH="$(host_arch)"
  TARGET="$ARCH-$OS"
  BUILD_DIR="$ROOT/target/release"
  cargo build --release
fi

EXE="zeta"
if [ "$OS" = "windows" ]; then
  EXE="zeta.exe"
fi

PACKAGE_DIR="$ROOT/dist/packages"
STAGE_NAME="zeta-$VERSION-$OS-$ARCH"
STAGE="$ROOT/dist/$STAGE_NAME"
ARCHIVE_BASE="$PACKAGE_DIR/$STAGE_NAME"

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/docs" "$PACKAGE_DIR"

cp "$BUILD_DIR/$EXE" "$STAGE/bin/$EXE"
cp README.md "$STAGE/"
cp -R docs/. "$STAGE/docs/"

cat > "$STAGE/INSTALL.txt" <<EOF
Zeta $VERSION

Install:
  1. Copy bin/$EXE to a directory on PATH.
  2. Run: zeta repl
  3. Verify: zeta run testdata/run_enum.zeta

Docs are included under docs/ and published at:
  https://zeta.jennieapp.com/docs/
EOF

if command -v shasum >/dev/null 2>&1; then
  (cd "$STAGE" && shasum -a 256 "bin/$EXE" > SHA256SUMS)
elif command -v sha256sum >/dev/null 2>&1; then
  (cd "$STAGE" && sha256sum "bin/$EXE" > SHA256SUMS)
fi

if [ "$OS" = "windows" ] && command -v zip >/dev/null 2>&1; then
  (cd "$ROOT/dist" && zip -qr "$ARCHIVE_BASE.zip" "$STAGE_NAME")
  ARCHIVE="$ARCHIVE_BASE.zip"
else
  tar -C "$ROOT/dist" -czf "$ARCHIVE_BASE.tar.gz" "$STAGE_NAME"
  ARCHIVE="$ARCHIVE_BASE.tar.gz"
fi

CHECKSUM=""
if command -v shasum >/dev/null 2>&1; then
  CHECKSUM="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  CHECKSUM="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
fi

cat > "$ARCHIVE_BASE.json" <<EOF
{
  "name": "zeta",
  "version": "$VERSION",
  "target": "$TARGET",
  "os": "$OS",
  "arch": "$ARCH",
  "archive": "$(basename "$ARCHIVE")",
  "sha256": "$CHECKSUM"
}
EOF

printf '%s\n' "$ARCHIVE"
