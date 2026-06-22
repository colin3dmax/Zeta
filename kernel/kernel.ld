/* Linker script for the QEMU virt riscv64 kernel.
 *
 * DRAM begins at 0x8000_0000; with `-bios none` QEMU jumps there at reset, so
 * the boot stub (.text.boot) must come first. A small stack is reserved at the
 * end of the image and its top exported as `_stack_top` for boot.s. */
ENTRY(_start)

SECTIONS
{
  . = 0x80000000;

  .text : {
    KEEP(*(.text.boot))   /* _start must be at the very base */
    *(.text .text.*)
  }

  .rodata : { *(.rodata .rodata.*) }   /* string literals live here */
  .data   : { *(.data .data.*) }
  .bss    : { *(.bss .bss.*) *(COMMON) }

  . = ALIGN(16);
  . += 0x10000;           /* 64 KiB stack */
  _stack_top = .;
}
