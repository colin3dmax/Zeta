#!/usr/bin/env bash
# Boot the kernel in QEMU and print its serial output. `-bios none` puts us in
# M-mode at 0x8000_0000 with no firmware; `-nographic` wires UART0 to stdout.
# The kernel spins forever after printing, so we cap the run with a timeout.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ELF="${1:-$HERE/build/kernel.elf}"

exec qemu-system-riscv64 \
  -machine virt \
  -bios none \
  -nographic \
  -no-reboot \
  -kernel "$ELF"
