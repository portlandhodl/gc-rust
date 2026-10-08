//! GameCube memory map + MMIO register accessors and PPC bit helpers.
//!
//! Reference addresses come from the GameCube Hardware Reference (YAGCD)
//! and libogc: VI @ 0xCC002000, PE @ 0xCC001000, CP @ 0xCC000000,
//! SI @ 0xCC006400, write-gather pipe ("wgPipe") @ 0xCC008000.

use core::ptr::{read_volatile, write_volatile};

/// Cached view of main memory.
pub const MEM_BASE_CACHED: u32 = 0x8000_0000;
/// Uncached view of main memory (writes there skip the CPU cache; MMIO
/// lives here).
pub const MEM_BASE_UNCACHED: u32 = 0xC000_0000;
/// Base of the on-chip I/O block.
pub const IO_BASE: u32 = 0xCC00_0000;

/// 24 MiB MEM1.
pub const MEM1_TOP: u32 = 0x8180_0000;

/// The GX "wgPipe" (write-gather pipe) the CPU pushes commands into.
pub const WG_PIPE: u32 = 0xCC00_8000;

/// Video Interface registers (u16-indexed).
pub const VI_BASE: u32 = 0xCC00_2000;
/// Command Processor registers (u16-indexed).
pub const CP_BASE: u32 = 0xCC00_0000;
/// Pixel Engine registers (u16-indexed).
pub const PE_BASE: u32 = 0xCC00_1000;
/// Processor Interface registers (u32-indexed).
pub const PI_BASE: u32 = 0xCC00_3000;
/// Serial Interface registers (u32-indexed).
pub const SI_BASE: u32 = 0xCC00_6400;

#[inline(always)]
pub const fn phys(addr: u32) -> u32 {
    addr & 0x1fff_ffff
}

/// K0 (cached) -> K1 (uncached) address conversion.
#[inline(always)]
pub fn cached_to_uncached<T>(p: *mut T) -> *mut T {
    (p as usize).wrapping_add((MEM_BASE_UNCACHED - MEM_BASE_CACHED) as usize) as *mut T
}

/// Virtual (0x8...) -> physical (0x0...) address conversion.
#[inline(always)]
pub fn virt_to_phys<T>(p: *const T) -> u32 {
    (p as u32) & 0x3fff_ffff
}

#[inline(always)]
pub unsafe fn read16(addr: u32) -> u16 {
    read_volatile(addr as *const u16)
}

#[inline(always)]
pub unsafe fn write16(addr: u32, v: u16) {
    write_volatile(addr as *mut u16, v);
}

#[inline(always)]
pub unsafe fn read32(addr: u32) -> u32 {
    read_volatile(addr as *const u32)
}

#[inline(always)]
pub unsafe fn write32(addr: u32, v: u32) {
    write_volatile(addr as *mut u32, v);
}

/// VI register read (index in u16 units from VI_BASE, as in libogc sources).
#[inline(always)]
pub unsafe fn vi_read(idx: u16) -> u16 {
    read16(VI_BASE + 2 * u32::from(idx))
}

#[inline(always)]
pub unsafe fn vi_write(idx: u16, v: u16) {
    write16(VI_BASE + 2 * u32::from(idx), v);
}

/// CP register read/write (u16 units).
#[inline(always)]
pub unsafe fn cp_read(idx: u16) -> u16 {
    read16(CP_BASE + 2 * u32::from(idx))
}

#[inline(always)]
pub unsafe fn cp_write(idx: u16, v: u16) {
    write16(CP_BASE + 2 * u32::from(idx), v);
}

/// PE register read/write (u16 units).
#[inline(always)]
pub unsafe fn pe_read(idx: u16) -> u16 {
    read16(PE_BASE + 2 * u32::from(idx))
}

#[inline(always)]
pub unsafe fn pe_write(idx: u16, v: u16) {
    write16(PE_BASE + 2 * u32::from(idx), v);
}

/// SI register read/write (u32 units).
#[inline(always)]
pub unsafe fn si_read(idx: u32) -> u32 {
    read32(SI_BASE + 4 * idx)
}

#[inline(always)]
pub unsafe fn si_write(idx: u32, v: u32) {
    write32(SI_BASE + 4 * idx, v);
}

/// `_SHIFTL(v, s, w)` exactly as libogc's gcutil.h: take the low `w` bits
/// of `v` and place them at bit position `s` from the right (zero-based).
#[inline(always)]
pub const fn shiftl(v: u32, s: u32, w: u32) -> u32 {
    (v & ((1u32 << w) - 1)) << s
}

/// `_SHIFTR(v, s, w)` exactly as libogc's gcutil.h: extract the field of
/// `w` bits starting at bit `s` from the right.
#[inline(always)]
pub const fn shiftr(v: u32, s: u32, width: u32) -> u32 {
    (v >> s) & ((1u32 << width) - 1)
}

/// Flush (write back) a data-cache range so hardware DMA/GPU sees it.
#[inline(always)]
pub unsafe fn dc_flush_range<T>(ptr: *const T, len: usize) {
    core::arch::asm!(
    "1:",
    "dcbf 0, {ptr}",
    "addi {ptr}, {ptr}, 32",
    "addi {len}, {len}, -32",
    "cmpwi {len}, 0",
    "bgt 1b",
    ptr = inout(reg) ptr => _,
    len = inout(reg) len as i32 => _,
    );
    sync();
}

/// Invalidate a data-cache range (so a following read goes to memory).
#[inline(always)]
pub unsafe fn dc_invalidate_range<T>(ptr: *const T, len: usize) {
    core::arch::asm!(
    "1:",
    "dcbi 0, {ptr}",
    "addi {ptr}, {ptr}, 32",
    "addi {len}, {len}, -32",
    "cmpwi {len}, 0",
    "bgt 1b",
    ptr = inout(reg) ptr => _,
    len = inout(reg) len as i32 => _,
    );
    sync();
}

/// `sync` instruction (memory barrier).
#[inline(always)]
pub unsafe fn sync() {
    core::arch::asm!("sync");
}

/// `isync` instruction (barrier against subsequent instruction fetches).
#[inline(always)]
pub unsafe fn isync() {
    core::arch::asm!("isync");
}

/// `dcbf` - data-cache block flush (write-back) for `addr`.
#[inline(always)]
pub unsafe fn dcbf(addr: *const u8) {
    core::arch::asm!("dcbf 0, {addr}", addr = in(reg) addr);
}

/// `dcbi` - data-cache block invalidate for `addr`.
#[inline(always)]
pub unsafe fn dcbi(addr: *const u8) {
    core::arch::asm!("dcbi 0, {addr}", addr = in(reg) addr);
}

/// Busy-wait for `us` microseconds using the ~40.5 MHz timebase
/// (40 ticks/µs close enough for our settle waits).
pub fn gcdelay(us: u32) {
    let ticks = (us as u64).saturating_mul(40);
    let start = mftb();
    while mftb().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

/// Read the 64-bit timebase (`mftb`, runs at bus clock / 4 ≈ 40.5 MHz).
#[inline(always)]
pub fn mftb() -> u64 {
    loop {
        let hi: u32;
        let lo: u32;
        let hi2: u32;
        unsafe {
            core::arch::asm!(
                "mftbu {hi}",
                "mftb {lo}",
                "mftbu {hi2}",
                hi = out(reg) hi,
                lo = out(reg) lo,
                hi2 = out(reg) hi2,
            );
        }
        if hi == hi2 {
            return ((hi as u64) << 32) | lo as u64;
        }
    }
}
