//! Audio output — stereo 16-bit PCM at 48 kHz, AI-DMA driven.
//!
//! Path: AI DMA streams PCM from MEM1 forever, ping-ponging between two
//! 384-byte blocks. Each block = 2 ms. We refill the just-consumed block
//! from an [`on_refill`] callback each DMA-complete interrupt.
//!
//! Example output format used by Dolphin's AudioInterface HLE and by
//! libogc's `AUDIO_InitDMA`:
//!
//!   DSP-based AI DMA regs (u16-indexed from 0xCC005000):
//!     [24]  AUDIO_DMA_START_HI  = (physaddr >> 13) & 0x0fff
//!     [25]  AUDIO_DMA_START_LO  = physaddr & 0xffe0
//!     [27]  AUDIO_DMA_CTL_LEN   = 0x8000 | (len >> 5)
//!
//!   AI block (u16-indexed from 0xCC006C00):
//!     [0]   AI_CONTROL   (bits: PSTAT=0x01, AISFR=0x02, AIINTMSK=0x04,
//!                         AIINT=0x08 writes-1-to-clear, SCRESET=0x20, DMAFR=0x40)
//!     [1]   AI_VOLUME    (bits 0-7 left, 8-15 right, 0xff each = max)
//!     [2]   AI_SAMPLE_COUNT

#[allow(dead_code)]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::hw;

pub const AUDIO_RATE_HZ: u32 = 48_000;
pub const AI_BUF_BYTES: usize = 384; // 2 ms stereo 16-bit
pub const AI_BUF_FRAMES: usize = AI_BUF_BYTES / 4; // stereo frames per buffer

const AI_CTRL: u32 = 0xCC00_6C00;
const AI_VOL: u32 = 0xCC00_6C04;

const DSP_BASE: u32 = 0xCC00_5000;

const AI_PSTAT: u16 = 0x01;
const AI_AISFR: u16 = 0x02;
const AI_AIINTMSK: u16 = 0x04;
const AI_AIINT: u16 = 0x08;
const AI_SCRESET: u16 = 0x20;
const AI_DMAFR: u16 = 0x40;

#[repr(align(32))]
struct BufAlign([u8; AI_BUF_BYTES + 32]);

static mut BUF_A: BufAlign = BufAlign([0; AI_BUF_BYTES + 32]);
static mut BUF_B: BufAlign = BufAlign([0; AI_BUF_BYTES + 32]);

pub struct Audio {
    _private: (),
}

static INITED: AtomicBool = AtomicBool::new(false);
static RUN_BUF_B: AtomicBool = AtomicBool::new(false); // the buffer DMA is *playing*
#[allow(improper_ctypes_definitions)]
static mut REFILL_CB: Option<extern "C" fn(&mut [i16])> = None;

#[inline(always)]
unsafe fn buf_phys(b: bool) -> u32 {
    let p = if b { &raw const BUF_B } else { &raw const BUF_A };
    hw::virt_to_phys(p as *const u8)
}

#[inline(always)]
unsafe fn armed_buf(b: bool) {
    let addr = buf_phys(b) as u32;
    let len = AI_BUF_BYTES as u32;

    // AUDIO DMA registers (u16-indexed view of the DSP block)
    let base = DSP_BASE;
        hw::write16(base + 2 * 24, ((addr >> 16) & 0x1fff) as u16);
        hw::write16(base + 2 * 25, (addr & 0xffe0) as u16);
        hw::write16(base + 2 * 27, (hw::read16(base + 2 * 27) & !0x7fff) | (((len >> 5) as u16) & 0x7fff));
        hw::write16(base + 2 * 27, hw::read16(base + 2 * 27) | 0x8000);
    let _ = len;
}

/// AI DMA completion interrupt.
extern "C" fn ai_cb(_: crate::irq::Source) {
    unsafe {
        // Ack the AI interrupt bit, then re-arm the DMA with the *other* half
        let mut c = hw::read16(AI_CTRL);
        c |= AI_AIINT;
        hw::write16(AI_CTRL, c);

        // flip and re-arm
        let now_playing = RUN_BUF_B.load(Ordering::Acquire);
        let next = !now_playing;
        armed_buf(next);
        RUN_BUF_B.store(next, Ordering::Release);

        // refill what just finished (the other one)
        if let Some(cb) = REFILL_CB {
            let buf = if now_playing { &mut *core::ptr::addr_of_mut!(BUF_A) } else { &mut *core::ptr::addr_of_mut!(BUF_B) };
            let samples = core::slice::from_raw_parts_mut(&mut buf.0 as *mut u8 as *mut i16, AI_BUF_FRAMES);
            cb(samples);
            hw::dc_flush_range(samples.as_ptr() as *const u8, AI_BUF_BYTES);
        }
    }
}

/// Bring the audio interface up: 48 kHz stream mode, master volume
/// full-range, and AI DMA armed on BUF_A.
pub fn init() -> Audio {
    if INITED.swap(true, Ordering::AcqRel) {
        return Audio { _private: () };
    }
    unsafe {
        // libogc AUDIO_Init order:
        // 1. clear PI's AI int enable, reset stream control (SCRESET)
        let _ = hw::read16(AI_CTRL);
        hw::write16(AI_CTRL, AI_SCRESET);
        // 2. set stream sample rate to 48 kHz  (AISFR=0), turn PSTAT on
        let mut c = hw::read16(AI_CTRL);
        c &= !(AI_AISFR | AI_DMAFR);    // 48kHz stream rate; no AI DMA yet
        c |= AI_PSTAT;   // play
        c |= AI_AIINTMSK;
        hw::write16(AI_CTRL, c);

        // full volume
        hw::write16(AI_VOL, 0xFFFF);

        // clear sample counter (SCRESET toggle...) then drop reset
        hw::write16(AI_CTR_SAMP, AI_SCRESET);

        // zero the buffers to silence
        core::ptr::write_bytes((*(&raw mut BUF_A)).0.as_mut_ptr(), 0, AI_BUF_BYTES);
        core::ptr::write_bytes((*(&raw mut BUF_B)).0.as_mut_ptr(), 0, AI_BUF_BYTES);
        hw::dc_flush_range((*(&raw mut BUF_A)).0.as_ptr(), AI_BUF_BYTES);
        hw::dc_flush_range((*(&raw mut BUF_B)).0.as_ptr(), AI_BUF_BYTES);

        // arm AI DMA on channel A, the ISR will ping-pong
        armed_buf(false);
    }
    Audio { _private: () }
}

// AI sample counter register
const AI_CTR_SAMP: u32 = 0xCC00_6C08;

/// Install the refill callback. Called from interrupt context every 2 ms.
#[allow(improper_ctypes_definitions)]
pub fn on_refill(_: &Audio, f: extern "C" fn(buf: &mut [i16])) {
    unsafe {
        REFILL_CB = Some(f);
        crate::irq::register(crate::irq::Source::Ai, Some(ai_cb));
        crate::irq::enable_source(crate::irq::Source::Ai);
        crate::irq::enable();
    }
}

/// Current DMA position (frames), useful for latency math if needed.
pub fn frames_played() -> u32 {
    unsafe { hw::read16(DSP_BASE + 2 * 29) as u32 / 2 } // AUDIO_DMA_BLOCKS_LEFT
}

pub struct AudioHandle {
    _private: (),
}

/// Marker for the playing stream? Not needed; callers own the Audio.
pub fn is_playing() -> bool {
    INITED.load(Ordering::Acquire)
}
