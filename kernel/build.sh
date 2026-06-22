#!/usr/bin/env bash
# Build the minimal Zeta riscv64 kernel:
#   Zeta source --(zeta emit-ir)--> LLVM IR --(clang --target=riscv64)--> object
#   + boot stub --(ld.lld + linker script)--> freestanding ELF for QEMU virt.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
OUT="$HERE/build"
LLVM="/opt/homebrew/opt/llvm/bin"
CLANG="$LLVM/clang"
LLD="/usr/local/bin/ld.lld"
TARGET="riscv64"
MARCH="rv64imac"
MABI="lp64"
# medany code model: PC-relative (auipc) addressing so .rodata/.text at the
# 0x8000_0000 DRAM base are reachable — the default medlow can't reach that high.
CFLAGS="--target=$TARGET -march=$MARCH -mabi=$MABI -mcmodel=medany -mno-relax -ffreestanding -fno-pic -nostdlib"

mkdir -p "$OUT"

echo "[1/5] build zeta (llvm feature)"
( cd "$ROOT" && LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
    cargo build --release --features llvm --bin zeta >/dev/null )

echo "[2/5] emit LLVM IR from kmain.zeta"
"$ROOT/target/release/zeta" emit-ir "$HERE/kmain.zeta" > "$OUT/kmain.raw.ll"
# Drop the host (arm64) datalayout/triple so clang re-targets cleanly to riscv64.
grep -v -E '^target (datalayout|triple)' "$OUT/kmain.raw.ll" > "$OUT/kmain.ll"

echo "[3/5] compile IR -> riscv64 object"
"$CLANG" $CFLAGS -c "$OUT/kmain.ll" -o "$OUT/kmain.o"

echo "[4/6] assemble boot stub"
"$CLANG" $CFLAGS -c "$HERE/boot.s" -o "$OUT/boot.o"

echo "[5/6] compile freestanding runtime stub (malloc/memcpy/...)"
# -O0: keep the byte-copy loops as loops; loop-idiom at -O>0 would rewrite them
# into memcpy/memset calls — i.e. into themselves — and recurse.
"$CLANG" $CFLAGS -O0 -c "$HERE/runtime.c" -o "$OUT/runtime.o"

echo "[6/6] link freestanding ELF"
"$LLD" -T "$HERE/kernel.ld" "$OUT/boot.o" "$OUT/kmain.o" "$OUT/runtime.o" -o "$OUT/kernel.elf"

echo "ok -> $OUT/kernel.elf"
