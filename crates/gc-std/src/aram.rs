//! ARAM (auxiliary DRAM, 16 MB) driver — DMA to/from main memory.
//!
//! Registers: ARAM DMA lives inside the DSP register block at 0xCC005000.

#[allow(dead_code)]
#[allow(unused_imports)]
use core::alloc::Layout as _Layout;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::hw;

const DSP_BASE: u32 = 0xCC00_5000;

// DMA control bits (dspReg[5])
const CSR_DSPDMA: u16 = 0x0200;
const CSR_DSPINT: u16 = 0x0080;
const CSR_ARINT: u16 = 0x0020;
const CSR_AIINT: u16 = 0x0008;

#[inline(always)]
unsafe fn r(i: u32) -> u16 {
    hw::read16(DSP_BASE + i * 2)
}
#[inline(always)]
unsafe fn w(i: u32, v: u16) {
    hw::write16(DSP_BASE + i * 2, v);
}

/// Simple bump allocator over ARAM. ARAM is 16 MB at [0x4000, 0x01000000);
/// (the first 0x4000 bytes are left alone, matching libogc).
static ALLOC_PTR: AtomicU32 = AtomicU32::new(0x4000);

/// ARAM init: set the refresh-bits register and confirm for our model.
pub(crate) fn init() {
    unsafe {
        // From libogc AR_Init: dspReg[13] low byte is the SDRAM refresh —
        // Dolphin reports 156 or 176. Don't touch it if already set.
        let v = r(13);
        let _ = v;
        // nothing else needed for us
    }
}

/// Reserve `size` bytes of ARAM, 32-byte aligned.
pub fn alloc(size: usize) -> Option<u32> {
    let cur = ALLOC_PTR.fetch_add(align_up(size as u32, 32), Ordering::Relaxed);
    if cur + size as u32 > 0x0100_0000 {
        return None;
    }
    Some(cur)
}

const fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

/// Write `len` bytes from main-memory `src` into ARAM at `dst_aram`.
///
/// Synchronous (busy-wait); identical to libogc's `__ARWriteDMA`.
pub fn dma_write_sync(src_phys: u32, dst_aram: u32, len: u32) {
    unsafe {
        // main-memory source address
        w(16, (r(16) & !0x03ff) | ((src_phys >> 16) & 0x03ff) as u16);
        w(17, (r(17) & !0xffe0) | (src_phys & 0xffff) as u16);
        // ARAM dest
        w(18, (r(18) & !0x03ff) | ((dst_aram >> 16) & 0x03ff) as u16);
        w(19, (r(19) & !0xffe0) | (dst_aram & 0xffff) as u16);
        // control: bit15 of reg 20 = 0 → write to ARAM; len split across
        // reg 20/21 (libogc puts the length like that; DMA starts).
        let mut t20 = r(20) & !0x8000;
        t20 = (t20 & !0x03ff) | (((len >> 16) & 0x03ff) as u16);
        w(20, t20);
        w(21, (r(21) & !0xffe0) | ((len & 0xffff) as u16));
        // busy wait complete
        while r(5) & CSR_DSPDMA != 0 {}
        // clear ARAM interrupt (W1C)
        let c = r(5);
        w(5, ((c as u32) & !(CSR_DSPINT as u32 | CSR_AIINT as u32) | CSR_ARINT as u32) as u16);
    }
}

/// Read `len` bytes from ARAM into main memory (`__ARReadDMA`).
pub fn dma_read_sync(src_aram: u32, dst_phys: u32, len: u32) {
    unsafe {
        w(16, (r(16) & !0x03ff) | ((dst_phys >> 16) & 0x03ff) as u16);
        w(17, (r(17) & !0xffe0) | (dst_phys & 0xffff) as u16);
        w(18, (r(18) & !0x03ff) | ((src_aram >> 16) & 0x03ff) as u16);
        w(19, (r(19) & !0xffe0) | (src_aram & 0xffff) as u16);
        let mut t20 = r(20);
        t20 = (t20 | 0x8000) & !0x03ff;
        t20 |= ((len >> 16) & 0x03ff) as u16;
        w(20, t20);
        w(21, (r(21) & !0xffe0) | ((len & 0xffff) as u16));
        while r(5) & CSR_DSPDMA != 0 {}
        let c = r(5);
        w(5, ((c as u32) & !(CSR_DSPINT as u32 | CSR_AIINT as u32) | CSR_ARINT as u32) as u16);
    }
}


