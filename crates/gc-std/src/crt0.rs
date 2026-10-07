//! crt0: the `_start` entry point (pure Rust port of devkitPro's
//! `PPCEarlyInit` from tuxedo/early_init.S, zlib license), plus BSS clear,
//! stack setup, call `main`.
//!
//! LLVM's PowerPC inline assembler uses bare register numbers (`3`, not
//! `r3`) and `@ha`/`@l` relocation operators.

use core::arch::global_asm;

global_asm!(
    r#"
    .section .crt0,"ax"
    .balign 32
    .global _start
_start:
    // --- Part 1: machine bring-up -----------------------------------------

    // Ensure the return address is virtual (some loaders leave it physical).
    mflr 3
    oris 3, 3, 0x8000
    mtlr 3

    // Drop to real mode at ".Lreal"
    lis 3, .Lreal@ha
    ori 3, 3, .Lreal@l
    mtsrr0 3

    li 3, 0x0032        // MSR_FP | MSR_ME | MSR_RI
    mtsrr1 3
    rfi

.Lreal:
    li 0, 0

    // HID0 = DPM|NHR | ICFI|DCFI|SPD|DCFA|BTIC|BHT (caches off for now)
    lis 3, 0x0011
    ori 3, 3, 0x0c64
    mtspr 1008, 3
    isync
    sync
    mtspr 1017, 0       // L2CR off
    sync

    // HID2 = LSQE | WPE | PSE (paired singles + gather pipe)
    lis 3, 0xE000
    mtspr 920, 3
    isync

    // WPAR = physical pipe address
    lis 3, 0x0C00
    ori 3, 3, 0x8000
    mtspr 921, 3

    // start L2 invalidate
    lis 3, 0x0020
    mtspr 1017, 3

    // clear BATs
    li 3, 0
    mtspr 528, 3
    mtspr 530, 3
    mtspr 532, 3
    mtspr 534, 3
    mtspr 536, 3
    mtspr 538, 3
    mtspr 540, 3
    mtspr 542, 3

    // SRs: direct-store pattern
    lis 3, 0x8000
    mtsr 0, 3
    mtsr 1, 3
    mtsr 2, 3
    mtsr 3, 3
    mtsr 4, 3
    mtsr 5, 3
    mtsr 6, 3
    mtsr 7, 3
    mtsr 8, 3
    mtsr 9, 3
    mtsr 10, 3
    mtsr 11, 3
    mtsr 12, 3
    mtsr 13, 3
    mtsr 14, 3
    mtsr 15, 3

    // IBAT0/DBAT0: 256MB VA 80000000 -> PA 0, cached
    li 3, 2
    lis 4, 0x8000
    ori 4, 4, 0x1fff
    mtspr 529, 3
    mtspr 528, 4
    mtspr 537, 3
    mtspr 536, 4

    // DBAT1: 256MB VA C0000000 -> PA 0, uncached, RW
    li 3, 0x2a
    lis 4, 0xc000
    ori 4, 4, 0x1fff
    mtspr 539, 3
    mtspr 538, 4

    // GQRs off
    li 3, 0
    mtspr 912, 3
    mtspr 913, 3
    mtspr 914, 3
    mtspr 915, 3
    mtspr 916, 3
    mtspr 917, 3
    mtspr 918, 3
    mtspr 919, 3

    // perf counters off
    mtspr 952, 3
    mtspr 956, 3
    mtspr 953, 3
    mtspr 954, 3
    mtspr 957, 3
    mtspr 958, 3

    // wait for L2 invalidate, then enable L2
1:  mfspr 3, 1017
    andi. 3, 3, 1
    bne 1b
    lis 3, 0x8000
    mtspr 1017, 3

    // L1 caches on
    mfspr 3, 1008
    ori 3, 3, 0xC000
    mtspr 1008, 3
    isync

    // FPSCR = 0, then set NI bit
    li 5, 0
    stw 5, -16(1)
    stw 5, -12(1)
    lfd 0, -16(1)
    mtfsf 0xff, 0
    mtfsb1 29

    // re-enable MMU
    mfmsr 3
    ori 3, 3, 0x0030
    mtmsr 3
    isync

    // --- Part 2: C environment --------------------------------------------

    // stack at top of MEM1 (IBA needs 8-byte align; keep 16 spare)
    lis 1, __stack@ha
    ori 1, 1, __stack@l
    li 0, 0
    stw 0, -16(1)
    stw 0, -12(1)
    addi 1, 1, -16

    li 13, 0            // SDA2 not used
    li 2, 0             // neither is r2

    bl __gc_rust_start
.Lhang:
    b .Lhang
"#,
);

global_asm!(
    r#"
    .section .bss.crt0,"aw",@nobits
    .balign 8
    .global __fpscr_zero
__fpscr_zero:
    .skip 8
"#
);

#[no_mangle]
pub extern "C" fn __gc_rust_start() -> ! {
    extern "C" {
        static mut __bss_start: u32;
        static mut __bss_end: u32;
        static __Arena1Lo: u32;
        static __Arena1Hi: u32;
    }

    unsafe {
        let mut p = &raw mut __bss_start as *mut u32;
        let end = &raw mut __bss_end as *mut u32;
        while p < end {
            core::ptr::write_volatile(p, 0);
            p = p.add(1);
        }
        crate::hw::isync();

        let lo = &raw const __Arena1Lo as *mut u8;
        let hi = &raw const __Arena1Hi as *mut u8;
        crate::heap::init(lo, hi);
    }

    extern "C" {
        fn main() -> i32;
    }
    let _ = unsafe { main() };
    crate::system::exit_to_loader();
    loop {
        unsafe { crate::hw::isync() };
    }
}
