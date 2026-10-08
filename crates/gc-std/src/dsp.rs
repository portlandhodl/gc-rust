//! DSP host driver for the GameCube/Wii DSP (mailbox & task loader).
//!
//! The DSP is a separate little CPU. ROM boot code in the DSP accepts the
//! microcode we upload via mailbox messages; the running microcode then
//! handles audio synthesis.
//!
//! Register block: 0xCC005000 (u16-indexed).

#[allow(dead_code)]
use crate::hw;

// DSP CSR bits (port of libogc's dsp.h / dolphin's HW/DSPRegisters.h)
pub mod csr {
    pub const DSPRESET: u16 = 0x0800;
    pub const DSPDMA: u16 = 0x0200;
    pub const DSPINTMSK: u16 = 0x0100;
    pub const DSPINT: u16 = 0x0080;
    pub const ARINTMSK: u16 = 0x0040;
    pub const ARINT: u16 = 0x0020;
    pub const AIINTMSK: u16 = 0x0010;
    pub const AIINT: u16 = 0x0008;
    pub const HALT: u16 = 0x0004;
    pub const PIINT: u16 = 0x0002;
    pub const RES: u16 = 0x0001;
}

// DSP proof codes — these are the DSP's ROM-driven handshakes.
pub(crate) const MAIL_DSP_READY: u32 = 0x8071_FEED;
pub(crate) const MAIL_TASK_INIT: u32 = 0x80F3_A001;
pub(crate) const MAIL_TASK_IRAM_ADDR: u32 = 0x80F3_C002;
pub(crate) const MAIL_TASK_IRAM_LEN: u32 = 0x80F3_A002;
pub(crate) const MAIL_TASK_DRAM_LOAD: u32 = 0x80F3_B002;
pub(crate) const MAIL_TASK_INIT_VEC: u32 = 0x80F3_D001;
// runtime messages (microcode -> host)
pub(crate) const MAIL_TASK_RUN: u32 = 0xDCD1_0000;
#[allow(dead_code)]
pub(crate) const MAIL_TASK_YIELD: u32 = 0xDCD1_0001;
pub(crate) const MAIL_TASK_DONE: u32 = 0xDCD1_0002;
pub(crate) const MAIL_TASK_DONE2: u32 = 0xDCD1_0003;
pub(crate) const MAIL_TASK_REQ: u32 = 0xDCD1_0004;
// runtime acks (host -> microcode)
pub(crate) const MAIL_ACK_DONE: u32 = 0xCDD1_0002;
#[allow(dead_code)]
pub(crate) const MAIL_ACK_YIELD: u32 = 0xCDD1_0001;
pub(crate) const MAIL_ACK_KILL: u32 = 0xCDD1_0003;

const DSP_REG_BASE: u32 = 0xCC00_5000;

#[inline(always)]
unsafe fn dsp_read(idx: u32) -> u16 {
    hw::read16(DSP_REG_BASE + idx * 2)
}

#[inline(always)]
unsafe fn dsp_write(idx: u32, v: u16) {
    hw::write16(DSP_REG_BASE + idx * 2, v);
}

/// Reset the DSP into a clean state (mirrors libogc's `DSP_Init`
/// register sequence: pulse DSPRESET, drop it again, RES stays clear).
pub fn reset() {
    unsafe {
        let old = dsp_read(5);
        dsp_write(5, (old & !(csr::AIINT | csr::ARINT | csr::DSPINT)) | csr::DSPRESET);
        dsp_write(
            5,
            old & !(csr::HALT | csr::AIINT | csr::ARINT | csr::DSPINT | csr::DSPRESET),
        );
    }
}

/// Acknowledge the DSP's mailbox interrupt (`DSPCR_DSPINT` is W1C).
/// Called from the DSP PI interrupt handler, mirroring libogc's
/// `__dsp_inthandler` ack.
pub fn ack_interrupt() {
    unsafe {
        let old = dsp_read(5);
        dsp_write(5, (old & !(csr::AIINT | csr::ARINT)) | csr::DSPINT);
    }
}

/// True if the mailbox-from-DSP is empty (host can send).
pub fn check_mail_to_free() -> bool {
    unsafe { dsp_read(0) & 0x8000 == 0 }
}

/// True if DSP has sent mail to us.
pub fn check_mail_from() -> bool {
    unsafe { dsp_read(2) & 0x8000 != 0 }
}

/// Read one 32-bit mailbox value from the DSP.
pub fn read_mail_from() -> u32 {
    unsafe {
        let hi = dsp_read(2);
        let lo = dsp_read(3);
        ((hi as u32) << 16) | (lo as u32)
    }
}

/// Send one 32-bit mailbox value to the DSP.
pub fn send_mail_to(mail: u32) {
    unsafe {
        dsp_write(0, (mail >> 16) as u16);
        dsp_write(1, (mail & 0xffff) as u16);
    }
}

/// Assert the DSP interrupt in the PI (for DSP-side interrupts).
pub fn assert_interrupt() {
    unsafe {
        let old = dsp_read(5);
        dsp_write(5, (old & !(csr::AIINT | csr::ARINT | csr::DSPINT)) | csr::PIINT);
    }
}

/// True if DSP-to-ARAM DMA is in flight.
pub fn dma_in_flight() -> bool {
    unsafe { dsp_read(5) & csr::DSPDMA != 0 }
}

/// Halt the DSP (stops microcode execution).
pub fn halt() {
    unsafe {
        let old = dsp_read(5);
        dsp_write(5, (old & !(csr::AIINT | csr::ARINT | csr::DSPINT)) | csr::HALT);
    }
}

/// Resume halted DSP.
pub fn unhalt() {
    unsafe {
        let old = dsp_read(5);
        dsp_write(5, old & !(csr::AIINT | csr::ARINT | csr::DSPINT | csr::HALT));
    }
}

// ---------------------------------------------------------------------------
// task loading (libogc's __dsp_boottask protocol)
// ---------------------------------------------------------------------------

/// Boot a DSP microcode image. `iram_addr`/`iram_len`/`init_vec` are in the
/// DSP address space. Blocks until the microcode is running.
pub fn boot_microcode(
    iram_image_phys: u32,   // physical address of the microcode in MEM1
    iram_addr: u16,         // DSP IRAM target address
    iram_len: u16,          // IRAM image length (bytes)
    init_vec: u16,          // entry vector (start PC in DSP space)
) -> Result<(), ()> {
    // The DSP bootrom waits for its mail/sync handshake.
    let mut tries = 0u32;
    while !check_mail_from() {
        tries += 1;
        if tries > 10_000_000 { return Err(()); }
        core::hint::spin_loop();
    }
    let ready = read_mail_from();
    if ready != MAIL_DSP_READY { return Err(()); }

    send_mail_to(MAIL_TASK_INIT); spin_wait_free();
    send_mail_to(iram_image_phys); spin_wait_free();
    send_mail_to(MAIL_TASK_IRAM_ADDR); spin_wait_free();
    send_mail_to(iram_addr as u32); spin_wait_free();
    send_mail_to(MAIL_TASK_IRAM_LEN); spin_wait_free();
    send_mail_to(iram_len as u32); spin_wait_free();
    send_mail_to(MAIL_TASK_DRAM_LOAD); spin_wait_free();
    send_mail_to(0); spin_wait_free();
    send_mail_to(MAIL_TASK_INIT_VEC); spin_wait_free();
    send_mail_to(init_vec as u32); spin_wait_free();

    Ok(())
}

#[inline]
fn spin_wait_free() {
    let mut n = 0u32;
    while !check_mail_to_free() {
        n += 1;
        if n > 10_000_000 {
            return;
        }
        core::hint::spin_loop();
    }
}
