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

# --- Cooperative scheduler primitives (called from Zeta via extern FFI) ---
#
# A task context is 14 doublewords: ra, sp, s0..s11 (the callee-saved set the
# C ABI guarantees across a call). switch_context saves the current set into
# *a0 and loads *a1, then `ret` resumes whoever owns the new context — either
# mid-yield or, for a fresh task, at task_trampoline.
.global switch_context
switch_context:                 # a0 = &old_ctx, a1 = &new_ctx
    sd   ra,    0(a0)
    sd   sp,    8(a0)
    sd   s0,   16(a0)
    sd   s1,   24(a0)
    sd   s2,   32(a0)
    sd   s3,   40(a0)
    sd   s4,   48(a0)
    sd   s5,   56(a0)
    sd   s6,   64(a0)
    sd   s7,   72(a0)
    sd   s8,   80(a0)
    sd   s9,   88(a0)
    sd   s10,  96(a0)
    sd   s11, 104(a0)
    ld   ra,    0(a1)
    ld   sp,    8(a1)
    ld   s0,   16(a1)
    ld   s1,   24(a1)
    ld   s2,   32(a1)
    ld   s3,   40(a1)
    ld   s4,   48(a1)
    ld   s5,   56(a1)
    ld   s6,   64(a1)
    ld   s7,   72(a1)
    ld   s8,   80(a1)
    ld   s9,   88(a1)
    ld   s10,  96(a1)
    ld   s11, 104(a1)
    ret

# Entry for a freshly-created task: its context is set up with ra=task_trampoline,
# a fresh sp, and s0 = task id. The trampoline hands the id to the Zeta dispatcher
# `run_task(id)`. If a task ever returns it parks here forever.
.global task_trampoline
task_trampoline:
    mv   a0, s0
    call run_task
1:  wfi
    j    1b

# extern fn trampoline_addr() -> Int — so Zeta can seed a new context's ra slot.
.global trampoline_addr
trampoline_addr:
    la   a0, task_trampoline
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
