# Zeta bare-metal kernel (minimal loop)

The smallest possible proof that Zeta can target bare metal: a `kmain` written in
Zeta is compiled to a freestanding riscv64 ELF, boots in QEMU's `virt` machine,
and prints a line to the UART — then halts.

```
Zeta source ──(zeta emit-ir)──► LLVM IR ──(clang --target=riscv64)──► object
        + boot.s (set sp, call main) ──(ld.lld + kernel.ld)──► kernel.elf ──► QEMU
```

## Run it

```sh
bash kernel/build.sh     # build kernel/build/kernel.elf
bash kernel/run.sh       # boot in QEMU (Ctrl-A then X to quit; it spins forever)
```

Expected serial output:

```
Zeta OS: booting on bare-metal riscv64
the answer is 42
sum of 5 elems = 150
delta = -273
kernel: done, halting.
```

The kernel uses **heap Strings** (`string_concat`, `int_to_string`), an **array**,
and a String-by-value helper — the full value-semantics type system, on bare metal.

## How it works / what was needed

* **Output = a volatile store.** QEMU virt maps the NS16550 UART transmit
  register at `0x1000_0000`. The only new language primitive is the `mmio_write_byte`
  / `mmio_read_byte` builtin, lowered to a volatile `i8` store/load at an
  `inttoptr` address. No syscalls, no inline assembly for I/O.
* **A freestanding runtime (`runtime.c`).** The native backend references a few
  C-library symbols when Strings/arrays/structs allocate and copy:
  `malloc`/`free`/`memcpy`/`memcmp`/`memset`. `runtime.c` supplies them with a
  bump allocator over a static arena (`free` is a no-op) plus byte-loop copies.
  Compiled at `-O0` so the loop-idiom pass doesn't rewrite those loops into calls
  to themselves. `int_to_string` needs **no** runtime help — the backend emits a
  self-contained decimal conversion (no libc `snprintf`).
* **`boot.s`** is the only assembly: ~5 lines to point `sp` at a reserved stack
  (every Zeta function uses stack allocas) and `call main`. If `main` ever
  returned it `wfi`-loops instead of running garbage.
* **`kernel.ld`** places `.text.boot` first at the `0x8000_0000` DRAM base
  (where QEMU jumps at reset with `-bios none`) and reserves a 64 KiB stack.
* **`-mcmodel=medany`** is required: the default `medlow` uses absolute
  `lui`/`addi` addressing that cannot reach `0x8000_0000`; `medany` is PC-relative.

## Files

| file | role |
|---|---|
| `kmain.zeta` | the kernel, in Zeta |
| `runtime.c` | freestanding malloc/free/memcpy/memcmp/memset |
| `boot.s` | stack setup + entry (the only assembly) |
| `kernel.ld` | linker script: load at 0x8000_0000, reserve stack |
| `build.sh` | emit-ir → clang riscv64 → ld.lld → ELF |
| `run.sh` | boot the ELF in `qemu-system-riscv64` |

## Toolchain

`qemu-system-riscv64`, Homebrew LLVM (`/opt/homebrew/opt/llvm/bin/clang`),
`ld.lld` (`/usr/local/bin`). Adjust paths in `build.sh` if yours differ.

## Next steps (see `docs/compiler/handoff.md` §6)

1. ✅ ~~Freestanding runtime stubs → String/array/struct in the kernel.~~ Done.
2. Raw pointer type `*T` + a real UART driver with status polling (`mmio_read_byte`
   of the line-status register) instead of blind writes; structured MMIO.
3. A reclaiming allocator (the bump arena never frees) behind the same symbols.
4. Traps/interrupts, then a timer + the first scheduler (needs the concurrency
   primitives on the roadmap).
