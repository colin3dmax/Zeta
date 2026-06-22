# Minimal RISC-V boot stub for the QEMU `virt` machine.
#
# With `-bios none`, QEMU's reset ROM jumps to the start of DRAM (0x8000_0000),
# which is where the linker script places this code with ENTRY(_start). The jobs
# here: give Zeta's `main` a valid stack pointer (every Zeta function uses stack
# allocas), install the machine trap vector, then call main. `main` never
# returns, but if it ever did we wait-for-interrupt forever.
# Example assembly routine called FROM Zeta via `extern fn asm_add3(...)`,
# demonstrating the C ABI: integer args arrive in a0, a1, a2; the result is
# returned in a0. This is the same mechanism a scheduler's `switch_context`
# would use.
.global asm_add3
asm_add3:
    add  a0, a0, a1
    add  a0, a0, a2
    ret

.section .text.boot
.global _start
_start:
    la   sp, _stack_top      # point sp at the top of the reserved stack
    la   t0, trap_entry      # install the machine trap vector (direct mode)
    csrw mtvec, t0
    call main                # enter the Zeta kernel
1:  wfi                      # halt
    j    1b

# Machine trap entry. Hardware jumps here on any M-mode trap (here: the CLINT
# timer interrupt) with mepc = interrupted PC. We save the caller-saved registers
# the Zeta handler may clobber (it preserves s0–s11 per the C ABI), call the
# handler, restore, and `mret` back to exactly where we were interrupted.
.align 4
.global trap_entry
trap_entry:
    addi sp, sp, -128
    sd   ra,   0(sp)
    sd   t0,   8(sp)
    sd   t1,  16(sp)
    sd   t2,  24(sp)
    sd   a0,  32(sp)
    sd   a1,  40(sp)
    sd   a2,  48(sp)
    sd   a3,  56(sp)
    sd   a4,  64(sp)
    sd   a5,  72(sp)
    sd   a6,  80(sp)
    sd   a7,  88(sp)
    sd   t3,  96(sp)
    sd   t4, 104(sp)
    sd   t5, 112(sp)
    sd   t6, 120(sp)
    call trap_handler        # Zeta `fn trap_handler()` (return value ignored)
    ld   ra,   0(sp)
    ld   t0,   8(sp)
    ld   t1,  16(sp)
    ld   t2,  24(sp)
    ld   a0,  32(sp)
    ld   a1,  40(sp)
    ld   a2,  48(sp)
    ld   a3,  56(sp)
    ld   a4,  64(sp)
    ld   a5,  72(sp)
    ld   a6,  80(sp)
    ld   a7,  88(sp)
    ld   t3,  96(sp)
    ld   t4, 104(sp)
    ld   t5, 112(sp)
    ld   t6, 120(sp)
    addi sp, sp, 128
    mret
