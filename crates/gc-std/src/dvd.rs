//! DVD drive (DI) — synchronous read-only driver, register-level port of
//! libogc's `DVD_Low*` primitives (`libogc/dvd.c`).
//!
//! The DI block sits at 0xCC006000 (u32-indexed):
//!
//! ```text
//! reg 0  DI status (irq bits: DE_INT 4, TC_INT 16, BRK_INT 64 + masks)
//! reg 1  cover
//! reg 2  command
//! reg 3  offset (byte offset >> 2)
//! reg 4  length
//! reg 5  DMA target (physical)
//! reg 6  DMA length
//! reg 7  control (1 = start, 2 = do DMA)
//! reg 8  immediate result out (error code after REQUESTERROR)
//! ```
//!
//! Only the synchronous essentials: disc ID, inquiry, raw sector reads,
//! request-error, stop-motor. The firmware-patch / motor-dance state machine
//! of a full drive bring-up is deliberately omitted (badly-behaved early
//! drives are out of scope; Dolphin and healthy drives don't need it).
//!
//! Everything blocks (busy-polls) and is intentionally re-entrant-unsafe:
//! grab one read at a time. Drives seek slowly — first reads take up to
//! ~1.5 s.

use crate::hw;

const DI_BASE: u32 = 0xCC00_6000;

const DE_INT: u32 = 1 << 2; // device error interrupt
const TC_INT: u32 = 1 << 4; // transfer complete
const BRK_INT: u32 = 1 << 6; // break

// commands
const DVD_READSECTOR: u32 = 0xA800_0000;
const DVD_READDISKID: u32 = 0xA800_0040;
const DVD_DVDINQUIRY: u32 = 0x1200_0000;
const DVD_REQUESTERROR: u32 = 0xE000_0000;
const DVD_STOPMOTOR: u32 = 0xE300_0000;

pub const DISK_ID_SIZE: usize = 32;

pub const DVD_ERROR_OK: i32 = 0;
pub const DVD_ERROR_IDLE: i32 = -1; // no command in flight
pub const DVD_ERROR_IO: i32 = -2; // drive reported error
pub const DVD_ERROR_TIMEOUT: i32 = -3; // drive never answered
pub const DVD_ERROR_ALIGN: i32 = -4;

#[inline(always)]
unsafe fn r(i: u32) -> u32 {
    hw::read32(DI_BASE + i * 4)
}

#[inline(always)]
unsafe fn w(i: u32, v: u32) {
    hw::write32(DI_BASE + i * 4, v);
}

/// Wait for TC_INT / DE_INT / BRK_INT with a `tb`-based budget.
/// Returns Ok on clean completion.
unsafe fn wait_done(deadline_us: u64) -> Result<(), i32> {
    let start = hw::mftb();
    let budget = deadline_us.saturating_mul(41);
    loop {
        let st = r(0);
        if st & (TC_INT | DE_INT | BRK_INT) != 0 {
            // W1C ack (keep masks; write back the int bits we saw)
            w(0, (st & 0x2A) | (st & (TC_INT | DE_INT | BRK_INT)));
            if st & DE_INT != 0 {
                return Err(request_error());
            }
            if st & (TC_INT | BRK_INT) != 0 {
                return Ok(());
            }
        }
        if hw::mftb().wrapping_sub(start) > budget {
            // stop the drive if it never answers (mask further ints)
            w(0, r(0) & 0x2A); // deassert + ack
            return Err(DVD_ERROR_TIMEOUT);
        }
        core::hint::spin_loop();
    }
}

/// Completion-wait for a command that carries no DMA (`inquiry`/`stop` style).
unsafe fn exec_cmd(cmd: u32, deadline_us: u64) -> i32 {
    w(2, cmd);
    w(7, 0x01); // START (no DMA bit)
    match wait_done(deadline_us) {
        Ok(()) => DVD_ERROR_OK,
        Err(e) => e,
    }
}

/// DI DMA command engine.
unsafe fn exec_dma(cmd: u32, offset_bytes: u64, buf: *mut u8, len: usize, deadline_us: u64) -> i32 {
    if (buf as usize) & 0x1f != 0 || len & 0x1f != 0 {
        return DVD_ERROR_ALIGN;
    }
    hw::dc_invalidate_range(buf, len);
    w(2, cmd);
    w(3, (offset_bytes >> 2) as u32);
    w(4, len as u32);
    w(5, hw::virt_to_phys(buf));
    w(6, len as u32);
    // start with DMA
    w(7, 0x03);
    let rc = match wait_done(deadline_us) {
        Ok(()) => DVD_ERROR_OK,
        Err(e) => e,
    };
    hw::dc_invalidate_range(buf, len);
    rc
}

/// The drive's error code, fetched right after DE_INT via DVD_REQUESTERROR.
unsafe fn request_error() -> i32 {
    w(2, DVD_REQUESTERROR);
    w(7, 0x01);
    let start = hw::mftb();
    loop {
        let st = r(0);
        if st & (TC_INT | DE_INT) != 0 {
            w(0, (st & 0x2A) | (st & (TC_INT | DE_INT)));
            break;
        }
        if hw::mftb().wrapping_sub(start) > 41 * 1_000_000 {
            break;
        }
        core::hint::spin_loop();
    }
    let _code = r(8); // error code; callers get DVD_ERROR_IO
    DVD_ERROR_IO
}

/// ~1 s default budget; disc seeks can be slow but this is a *per command*
/// timeout, not per seek-motif.
const DEFAULT_TIMEOUT_US: u64 = 10_000_000;

/// Read the 32-byte disc ID (gamecode etc.).
pub fn read_disk_id(buf: &mut [u8; DISK_ID_SIZE]) -> i32 {
    unsafe {
        exec_dma(
            DVD_READDISKID,
            0,
            buf.as_mut_ptr(),
            DISK_ID_SIZE,
            DEFAULT_TIMEOUT_US,
        )
    }
}

/// `DVD_Inquiry` — drive vendor/revision string blob (32 bytes).
pub fn inquiry(buf: &mut [u8; 32]) -> i32 {
    unsafe {
        // libogc uses the DMA register dance for inquiry as well.
        exec_dma(DVD_DVDINQUIRY, 0, buf.as_mut_ptr(), 32, DEFAULT_TIMEOUT_US)
    }
}

/// `DVD_Read` — absolute byte offset on the disc, sector-aligned (2048 B),
/// `buf` 32-byte aligned in MEM1, `len` a 32-byte multiple. This is a raw
/// DMA into MEM1; cache is invalidated around it.
pub fn read_abs(buf: &mut [u8], offset: u64, len: usize) -> i32 {
    unsafe { exec_dma(DVD_READSECTOR, offset, buf.as_mut_ptr(), len, 20_000_000) }
}

/// `DVD_LowStopMotor`.
pub fn stop_motor() -> i32 {
    unsafe { exec_cmd(DVD_STOPMOTOR, DEFAULT_TIMEOUT_US) }
}
