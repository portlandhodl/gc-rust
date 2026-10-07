/* gc-rust linker script: GameCube (Gekko) memory layout.
 *
 * Layout mirrors devkitPPC's ogc.ld conventions:
 *   entry/code at 0x80003100  (just below the reload stub at 0x80001800
 *   is untouched; 0x80000100-0x800030FF is the loader/exception area)
 *   24 MB MEM1: 0x80000000-0x81800000
 *
 * Stack goes at the top of MEM1, growing down (main thread stack is at
 * __stack). Heap (our bump-less free list) sits between __Arena1Lo and
 * __Arena1Hi.
 */

OUTPUT_FORMAT("elf32-powerpc");
OUTPUT_ARCH(powerpc:common);
ENTRY(_start);

MEMORY {
    MEM1 (rwx) : ORIGIN = 0x80003100, LENGTH = 0x17CFF00
}

SECTIONS
{
    . = 0x80003100;

    .crt0 : {
        KEEP(*(.crt0))
        KEEP(*(.crt0.*))
    } > MEM1

    .text : {
        *(.text)
        *(.text.*)
    } > MEM1

    .rodata : {
        *(.rodata)
        *(.rodata.*)
        *(.srodata .srodata.*)
    } > MEM1

    .data : {
        *(.data)
        *(.data.*)
        *(.sdata .sdata.*)
        *(.sdata2 .sdata2.*)
    } > MEM1

    .bss (NOLOAD) : ALIGN(32) {
        __bss_start = .;
        *(.bss)
        *(.bss.*)
        *(.sbss .sbss.*)
        *(.sbss2 .sbss2.*)
        *(COMMON)
        __bss_end = .;
    } > MEM1

    . = ALIGN(32);
    __Arena1Lo = .;
    __Arena1Hi = 0x817FEFF0;
    __stack = 0x817FEFF0;

    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}
