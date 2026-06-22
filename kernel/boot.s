# Minimal RISC-V boot stub for the QEMU `virt` machine.
#
# With `-bios none`, QEMU's reset ROM jumps to the start of DRAM (0x8000_0000),
# which is where the linker script places this code with ENTRY(_start). The only
# job here is to give Zeta's `main` a valid stack pointer (every Zeta function
# uses stack allocas) and then call it. `main` never returns, but if it ever did
# we wait-for-interrupt forever rather than execute garbage.
.section .text.boot
.global _start
_start:
    la   sp, _stack_top      # point sp at the top of the reserved stack
    call main                # enter the Zeta kernel
1:  wfi                      # halt: low-power wait, looped in case of spurious wake
    j    1b
