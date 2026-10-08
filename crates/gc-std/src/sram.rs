//! System SRAM (64 bytes, lives on the EXI RTC/UART chip) — Rust port of
//! libogc `system.c`'s SRAM read/write machinery.
//!
//! SRAM holds the console's persistent settings: language, video mode,
//! audio mode, RTC counter bias, wireless-pad pairing IDs, DVD error code,
//! and the flashID used to unlock memory cards.
//!
//! Read path: select EXI ch0/dev1 at 8 MHz, immediate-write the 4-byte
//! command `0x20000100`, DMA-read 64 bytes. Write path mirrors libogc's
//! `__sram_write` (command `0xa0000100 + loc*64`, immediate write of the
//! tail), with checksum recalculation (`__buildchecksum`) when the main
//! block changes.

use crate::{exi, hw};

/// libogc `syssram` (the checksummed main block at the head of SRAM).
#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct SysSram {
    pub checksum: u16,
    pub checksum_inv: u16,
    pub ead0: u32,
    pub ead1: u32,
    pub counter_bias: u32,
    pub display_offset_h: i8,
    pub ntd: u8,
    pub lang: u8,
    pub flags: u8,
}

/// libogc `syssramex` (extended block at offset 0x14? laid out at
/// `srambuf[20..]` packed; here over the full 64-byte window for simplicity —
/// parse from the raw buffer via [`read_sram_ex`]).
#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct SysSramEx {
    pub flash_id: [[u8; 12]; 2],
    pub wireless_kbd_id: u32,
    pub wireless_pad_id: [u16; 4],
    pub dvderr_code: u8,
    pub padding0: u8,
    pub flashid_chksum: [u8; 2],
    pub gbs: u16,
    pub padding1: u16,
}

const SRAM_SIZE: usize = 64;

// SRAM flags byte bits (libogc system.c)
pub const SRAM_VIDEO_MODE_BITS: u8 = 0x03;
pub const SRAM_SOUND_MODE_BIT: u8 = 0x04;
pub const SRAM_PROGSCAN_BIT: u8 = 0x80;

pub const SYS_LANG_ENGLISH: u8 = 0;
pub const SYS_LANG_GERMAN: u8 = 1;
pub const SYS_LANG_FRENCH: u8 = 2;
pub const SYS_LANG_SPANISH: u8 = 3;
pub const SYS_LANG_ITALIAN: u8 = 4;
pub const SYS_LANG_DUTCH: u8 = 5;

pub const SYS_VIDEO_NTSC: u8 = 0;
pub const SYS_VIDEO_PAL: u8 = 1;
pub const SYS_VIDEO_MPAL: u8 = 2;

pub const SYS_SOUND_MONO: u8 = 0;
pub const SYS_SOUND_STEREO: u8 = 1;

#[repr(align(32))]
struct SramBuf([u8; SRAM_SIZE]);

/// `__sram_read` — DMA the whole 64-byte SRAM into MEM1.
fn sram_read(buf: &mut [u8; SRAM_SIZE]) -> bool {
    // libogc DCInvalidateRange before DMA: drop cached lines so the buffer
    // reads back what the device sent.
    let mut staging = SramBuf([0u8; SRAM_SIZE]);
    unsafe {
        hw::dc_invalidate_range(staging.0.as_ptr(), SRAM_SIZE);
    }

    if !exi::lock(exi::EXI_CHANNEL_0, exi::EXI_DEVICE_1) {
        return false;
    }
    let mut ok = false;
    'tx: {
        if !exi::select(exi::EXI_CHANNEL_0, exi::EXI_DEVICE_1, exi::EXI_SPEED8MHZ) {
            break 'tx;
        }
        let mut cmd = 0x2000_0100u32.to_be_bytes();
        if !exi::imm(exi::EXI_CHANNEL_0, &mut cmd, exi::EXI_WRITE) {
            exi::deselect(exi::EXI_CHANNEL_0);
            break 'tx;
        }
        if !exi::dma(exi::EXI_CHANNEL_0, staging.0.as_mut_ptr(), SRAM_SIZE, exi::EXI_READ) {
            exi::deselect(exi::EXI_CHANNEL_0);
            break 'tx;
        }
        exi::deselect(exi::EXI_CHANNEL_0);
        ok = true;
    }
    exi::unlock(exi::EXI_CHANNEL_0);

    if ok {
        unsafe {
            hw::dc_invalidate_range(staging.0.as_ptr(), SRAM_SIZE);
        }
        buf.copy_from_slice(&staging.0);
    }
    ok
}

/// `__sram_write` (tail write from `loc` to the end of SRAM).
fn sram_write(buf: &[u8; SRAM_SIZE], loc: usize) -> bool {
    if loc >= SRAM_SIZE {
        return false;
    }
    if !exi::lock(exi::EXI_CHANNEL_0, exi::EXI_DEVICE_1) {
        return false;
    }
    let mut ok = false;
    'tx: {
        if !exi::select(exi::EXI_CHANNEL_0, exi::EXI_DEVICE_1, exi::EXI_SPEED8MHZ) {
            break 'tx;
        }
        let mut cmd = (0xa000_0100u32 + ((loc as u32) << 6)).to_be_bytes();
        if !exi::imm(exi::EXI_CHANNEL_0, &mut cmd, exi::EXI_WRITE) {
            break 'tx;
        }
        let mut tail = [0u8; SRAM_SIZE];
        tail[..SRAM_SIZE - loc].copy_from_slice(&buf[loc..]);
        if !exi::imm_ex(exi::EXI_CHANNEL_0, &mut tail[..SRAM_SIZE - loc], exi::EXI_WRITE) {
            break 'tx;
        }
        ok = true;
    }
    exi::deselect(exi::EXI_CHANNEL_0);
    exi::unlock(exi::EXI_CHANNEL_0);
    ok
}

/// `__buildchecksum` — the covered fields are the four u16 words at byte
/// offsets 12..20 (counter_bias, display_offsetH/ntd, lang, flags).
fn build_checksum(buf: &mut [u8; SRAM_SIZE]) {
    let mut c1: u16 = 0;
    let mut c2: u16 = 0;
    for i in 0..4 {
        let w = u16::from_be_bytes([buf[12 + i * 2], buf[13 + i * 2]]);
        c1 = c1.wrapping_add(w);
        c2 = c2.wrapping_add(w ^ 0xffff);
    }
    buf[0..2].copy_from_slice(&c1.to_be_bytes());
    buf[2..4].copy_from_slice(&c2.to_be_bytes());
}

fn checksum_ok(buf: &[u8; SRAM_SIZE]) -> bool {
    let mut c1: u16 = 0;
    let mut c2: u16 = 0;
    for i in 0..4 {
        let w = u16::from_be_bytes([buf[12 + i * 2], buf[13 + i * 2]]);
        c1 = c1.wrapping_add(w);
        c2 = c2.wrapping_add(w ^ 0xffff);
    }
    c1 == u16::from_be_bytes([buf[0], buf[1]]) && c2 == u16::from_be_bytes([buf[2], buf[3]])
}

/// Read and checksum-verify the main SRAM block.
pub fn read_sram() -> Option<SysSram> {
    let buf = read_raw()?;
    Some(SysSram {
        checksum: u16::from_be_bytes([buf[0], buf[1]]),
        checksum_inv: u16::from_be_bytes([buf[2], buf[3]]),
        ead0: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
        ead1: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
        counter_bias: u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]),
        display_offset_h: buf[16] as i8,
        ntd: buf[17],
        lang: buf[18],
        flags: buf[19],
    })
}

/// Read and checksum-verify, returning the *raw* 64 bytes for callers that
/// want to poke at fields not modeled in [`SysSram`].
pub fn read_raw() -> Option<[u8; SRAM_SIZE]> {
    let mut buf = [0u8; SRAM_SIZE];
    if !sram_read(&mut buf) || !checksum_ok(&buf) {
        return None;
    }
    Some(buf)
}

/// `SYS_GetCounterBias` — RTC offset programmed in SRAM.
pub fn counter_bias() -> Option<u32> {
    read_raw().map(|b| u32::from_be_bytes([b[12], b[13], b[14], b[15]]))
}

/// `SYS_SetCounterBias`.
pub fn set_counter_bias(bias: u32) -> bool {
    let Some(mut buf) = read_raw() else { return false };
    buf[12..16].copy_from_slice(&bias.to_be_bytes());
    build_checksum(&mut buf);
    sram_write(&buf, 12)
}

/// `SYS_GetVideoMode` from SRAM flags.
pub fn video_mode() -> Option<u8> {
    read_raw().map(|b| b[19] & SRAM_VIDEO_MODE_BITS)
}

/// `SYS_SetVideoMode`.
pub fn set_video_mode(mode: u8) -> bool {
    let Some(mut buf) = read_raw() else { return false };
    if (buf[19] & SRAM_VIDEO_MODE_BITS) > 0x02 {
        buf[19] &= !SRAM_VIDEO_MODE_BITS;
    }
    buf[19] = (buf[19] & !SRAM_VIDEO_MODE_BITS) | (mode & SRAM_VIDEO_MODE_BITS);
    build_checksum(&mut buf);
    sram_write(&buf, 19)
}

/// `SYS_GetSoundMode` / `SYS_SetSoundMode`.
pub fn sound_mode() -> Option<u8> {
    read_raw().map(|b| (b[19] & SRAM_SOUND_MODE_BIT) >> 2)
}

pub fn set_sound_mode(stereo: bool) -> bool {
    let Some(mut buf) = read_raw() else { return false };
    if stereo {
        buf[19] |= SRAM_SOUND_MODE_BIT;
    } else {
        buf[19] &= !SRAM_SOUND_MODE_BIT;
    }
    build_checksum(&mut buf);
    sram_write(&buf, 19)
}

/// `SYS_GetLanguage` / `SYS_SetLanguage`.
pub fn language() -> Option<u8> {
    read_raw().map(|b| b[18])
}

pub fn set_language(lang: u8) -> bool {
    let Some(mut buf) = read_raw() else { return false };
    buf[18] = lang;
    build_checksum(&mut buf);
    sram_write(&buf, 18)
}

/// Progressive-scan flag (bit 7 of the flags byte).
pub fn is_progressive_scan() -> Option<bool> {
    read_raw().map(|b| b[19] & SRAM_PROGSCAN_BIT != 0)
}

/// Read the extended block (`syssramex`, at `srambuf[20..48]` packed).
pub fn read_sram_ex() -> Option<SysSramEx> {
    read_raw().map(|b| {
        let mut ex = SysSramEx {
            flash_id: [[0; 12]; 2],
            wireless_kbd_id: 0,
            wireless_pad_id: [0; 4],
            dvderr_code: 0,
            padding0: 0,
            flashid_chksum: [0; 2],
            gbs: 0,
            padding1: 0,
        };
        ex.flash_id[0].copy_from_slice(&b[20..32]);
        ex.flash_id[1].copy_from_slice(&b[32..44]);
        ex.wireless_kbd_id = u32::from_be_bytes([b[44], b[45], b[46], b[47]]);
        for i in 0..4 {
            ex.wireless_pad_id[i] = u16::from_be_bytes([b[48 + i * 2], b[49 + i * 2]]);
        }
        ex.dvderr_code = b[56];
        ex.flashid_chksum = [b[58], b[59]];
        ex.gbs = u16::from_be_bytes([b[60], b[61]]);
        ex
    })
}
