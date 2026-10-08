/* gc-rust ISO apploader payload: Dolphin's RunApploader reads the strip at
 * disc offset 0x2440+0x20 into *physical* MEM1 at 0x01200000, then calls the
 * entry point found at hdr 0x2440+0x10 (a big-endian absolute address).
 * We link at 0x81200000 so addresses resolve through the cached mapping. */

OUTPUT_FORMAT("elf32-powerpc");
OUTPUT_ARCH(powerpc:common);
ENTRY(apploader_entry);

MEMORY {
    LOAD (rwx) : ORIGIN = 0x81200000, LENGTH = 64K
}

SECTIONS
{
    . = 0x81200000;
    .text : {
        *(.text)
        *(.text.*)
        *(.rodata)
        *(.rodata.*)
        *(.srodata .srodata.*)
    } > LOAD
    .data : {
        *(.data)
        *(.data.*)
        *(.sdata .sdata.*)
        *(.sdata2 .sdata2.*)
    } > LOAD
    .bss (NOLOAD) : ALIGN(32) {
        *(.bss)
        *(.bss.*)
        *(.sbss .sbss.*)
        *(.sbss2 .sbss2.*)
        *(COMMON)
    } > LOAD
    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
