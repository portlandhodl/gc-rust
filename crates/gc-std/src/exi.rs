//! EXI (External Interface) bus driver — synchronous, pure Rust port of the
//! core of libogc's `exi.c`.
//!
//! The EXI bus carries the memory cards (slots A/B = channels 0/1), the
//! SDL "Boot IPL" / SRAM+RTC chip, SD Gecko adapters, USB Gecko, and the
//! BBA (channel 2, always in reset-map).
//!
//! Register frame: `0xCC006800 + 0x14*chn` (five u32s):
//! CSR(0), MAR(4), LENGTH(8), CR(0x0C), DATA(0x10).
//!
//! This port is deliberately *synchronous* — `imm*`/`dma` fuse libogc's
//! `EXI_Imm`/`EXI_Dma` with `EXI_Sync` (immediate SPI-style transactions
//! complete in µs; polling avoids the EXI interrupt cascade entirely).
//! Channel locking mirrors `EXI_Lock`/`EXI_Unlock` semantics (returns false
//! when already locked) but without the unlock-callback queue.

use crate::hw;

pub const EXI_CHANNEL_0: u32 = 0; // memory card slot A
pub const EXI_CHANNEL_1: u32 = 1; // memory card slot B
pub const EXI_CHANNEL_2: u32 = 2; // BBA / others

pub const EXI_DEVICE_0: u32 = 0;
pub const EXI_DEVICE_1: u32 = 1;
pub const EXI_DEVICE_2: u32 = 2;

pub const EXI_SPEED1MHZ: u32 = 0;
pub const EXI_SPEED2MHZ: u32 = 1;
pub const EXI_SPEED4MHZ: u32 = 2;
pub const EXI_SPEED8MHZ: u32 = 3;
pub const EXI_SPEED16MHZ: u32 = 4;
pub const EXI_SPEED32MHZ: u32 = 5;

pub const EXI_READ: u32 = 0;
pub const EXI_WRITE: u32 = 1;

// CSR bits (u32 view)
const EXI_DEVICE0: u32 = 0x0080;
#[allow(dead_code)]
const EXI_DEVICE1: u32 = 0x0100;
#[allow(dead_code)]
const EXI_DEVICE2: u32 = 0x0200;
#[allow(dead_code)]
const EXI_EXT_IRQ: u32 = 0x0800;
const EXI_EXT_BIT: u32 = 0x1000;

const MAX_CHANNELS: usize = 3;

const EXI_BASE: u32 = 0xCC00_6800;

#[inline(always)]
unsafe fn reg(chn: u32, idx: u32) -> u32 {
    hw::read32(EXI_BASE + chn * 0x14 + idx * 4)
}

#[inline(always)]
unsafe fn set_reg(chn: u32, idx: u32, v: u32) {
    hw::write32(EXI_BASE + chn * 0x14 + idx * 4, v);
}

/// Per-channel driver state.
struct Channel {
    flags: u32,
    locked_dev: u32,
}

const FLAG_DMA: u32 = 0x0001;
const FLAG_IMM: u32 = 0x0002;
const FLAG_SELECT: u32 = 0x0004;
const FLAG_LOCKED: u32 = 0x0010;

static mut CHANNELS: [Channel; MAX_CHANNELS] = [
    Channel { flags: 0, locked_dev: 0 },
    Channel { flags: 0, locked_dev: 0 },
    Channel { flags: 0, locked_dev: 0 },
];

#[inline(always)]
unsafe fn ch(chn: u32) -> &'static mut Channel {
    &mut (*(&raw mut CHANNELS))[chn as usize]
}

/// `EXI_Lock` — take exclusive access to `chn` for `dev`. Returns false when
/// the channel is already locked.
pub fn lock(chn: u32, dev: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 {
        return false;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & FLAG_LOCKED != 0 {
            return false;
        }
        c.locked_dev = dev;
        c.flags |= FLAG_LOCKED;
    }
    true
}

/// `EXI_Unlock` — release a channel.
pub fn unlock(chn: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 {
        return false;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & FLAG_LOCKED == 0 {
            return false;
        }
        c.flags &= !FLAG_LOCKED;
    }
    true
}

/// `EXI_Probe` — is there a device physically present behind this channel's
/// `EXT` line? (memory-card detect, SD Gecko detect, ...)
pub fn probe(chn: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 {
        return false;
    }
    unsafe { reg(chn, 0) & EXI_EXT_BIT != 0 }
}

/// `EXI_Select` — select `dev` on `chn` at `freq` (one of `EXI_SPEED*`).
/// Mirrors libogc: requires the channel locked for `dev` (except ch2), and
/// only device 0 requests leave the mask bits alone (interrupt masks kept).
pub fn select(chn: u32, dev: u32, freq: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 || dev > 2 {
        return false;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & FLAG_SELECT != 0 {
            return false;
        }
        if chn != EXI_CHANNEL_2 {
            if c.flags & FLAG_LOCKED == 0 || c.locked_dev != dev {
                return false;
            }
        }
        c.flags |= FLAG_SELECT;
        let csr = reg(chn, 0);
        // keep irq-mask bits (0x405), set CS line for dev + clock divider
        set_reg(chn, 0, (csr & 0x405) | (EXI_DEVICE0 << dev) | (freq << 4));
    }
    true
}

/// `EXI_SelectSD` — same as [`select`] but without raising the CS line
/// (used for SD-card dummy clocks / some SPI-mode quirks).
pub fn select_sd(chn: u32, dev: u32, freq: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 || dev > 2 {
        return false;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & FLAG_SELECT != 0 {
            return false;
        }
        if chn != EXI_CHANNEL_2 {
            if c.flags & FLAG_LOCKED == 0 || c.locked_dev != dev {
                return false;
            }
        }
        c.flags |= FLAG_SELECT;
        let csr = reg(chn, 0);
        set_reg(chn, 0, (csr & 0x405) | (freq << 4));
    }
    true
}

/// `EXI_Deselect`.
pub fn deselect(chn: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 {
        return false;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & FLAG_SELECT == 0 {
            return false;
        }
        c.flags &= !FLAG_SELECT;
        let csr = reg(chn, 0);
        set_reg(chn, 0, csr & 0x405);
    }
    true
}

/// `EXI_Imm` + `EXI_Sync` fused: one immediate transfer of ≤4 bytes
/// (1–4), big-endian packed in the DATA register.
pub fn imm(chn: u32, data: &mut [u8], mode: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 {
        return false;
    }
    let len = data.len();
    if len == 0 || len > 4 {
        return false;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & (FLAG_DMA | FLAG_IMM) != 0 || c.flags & FLAG_SELECT == 0 {
            return false;
        }
        c.flags |= FLAG_IMM;
        if mode != EXI_READ {
            let mut value: u32 = 0;
            for (i, b) in data.iter().enumerate() {
                value |= (*b as u32) << ((3 - i) * 8);
            }
            set_reg(chn, 4, value);
        }
        set_reg(chn, 3, (((len as u32 - 1) & 0x03) << 4) | ((mode & 0x03) << 2) | 0x01);

        // EXI_Sync: poll TSTART
        let mut n = 0u32;
        while reg(chn, 3) & 0x1 != 0 {
            n += 1;
            if n > 10_000_000 {
                c.flags &= !(FLAG_DMA | FLAG_IMM);
                return false;
            }
            core::hint::spin_loop();
        }

        if mode != EXI_WRITE {
            let value = reg(chn, 4);
            for i in 0..len {
                data[i] = ((value >> ((3 - i) * 8)) & 0xff) as u8;
            }
        }
        c.flags &= !(FLAG_DMA | FLAG_IMM);
    }
    true
}

/// `EXI_ImmEx` — extended immediate transfer (chunks of 4 bytes).
pub fn imm_ex(chn: u32, data: &mut [u8], mode: u32) -> bool {
    let mut off = 0usize;
    while off < data.len() {
        let n = (data.len() - off).min(4);
        if !imm(chn, &mut data[off..off + n], mode) {
            return false;
        }
        off += n;
    }
    true
}

/// `EXI_Dma` + `EXI_Sync` fused: DMA transfer between MEM1 and the EXI
/// device. `buf` must be 32-byte aligned; cache maintenance around the DMA
/// is the caller's job (flush for writes, invalidate for reads), exactly
/// like libogc.
pub fn dma(chn: u32, buf: *mut u8, len: usize, mode: u32) -> bool {
    if chn >= MAX_CHANNELS as u32 || len == 0 {
        return false;
    }
    if (buf as usize) & 0x1f != 0 {
        return false; // DMA requires 32 B alignment
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = ch(chn);
        if c.flags & (FLAG_DMA | FLAG_IMM) != 0 || c.flags & FLAG_SELECT == 0 {
            return false;
        }
        c.flags |= FLAG_DMA;
        set_reg(chn, 1, hw::virt_to_phys(buf) & 0x03ff_ffe0);
        set_reg(chn, 2, len as u32);
        set_reg(chn, 3, ((mode & 0x03) << 2) | 0x03);

        let mut n = 0u32;
        while reg(chn, 3) & 0x1 != 0 {
            n += 1;
            if n > 50_000_000 {
                c.flags &= !(FLAG_DMA | FLAG_IMM);
                return false;
            }
            core::hint::spin_loop();
        }
        c.flags &= !(FLAG_DMA | FLAG_IMM);
    }
    true
}

/// `EXI_GetID` (simplified): issue the device-ID query (write 2 zero bytes,
/// read 4-byte ID) at 1 MHz. Unlike libogc this does not cache IDs or do the
/// insert-detect timing dance for slot devices on channels 0/1 — use
/// [`probe`] beforehand when you need presence detection.
pub fn get_id(chn: u32, dev: u32) -> Option<u32> {
    if !lock(chn, dev) {
        return None;
    }
    let mut out = None;
    if select(chn, dev, EXI_SPEED1MHZ) {
        let mut cmd = [0u8; 2];
        let mut id = [0u8; 4];
        if imm(chn, &mut cmd, EXI_WRITE) && imm(chn, &mut id, EXI_READ) {
            out = Some(u32::from_be_bytes(id));
        }
        deselect(chn);
    }
    unlock(chn);
    out
}
