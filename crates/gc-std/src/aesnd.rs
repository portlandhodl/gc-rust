//! AESND-style polyphonic audio mixer — host-side port of libogc's
//! `libaesnd/aesndlib.c` onto the DSP mixer microcode in [`crate::dspcode`].
//!
//! Layout of the system:
//!
//! * The AI DMA engine streams 2 ms stereo s16 blocks (384 bytes) from one of
//!   two ping-pong buffers out to the DAC at 48 kHz. Every block-complete
//!   interrupt re-arms the DMA and kicks the DSP mixer for the next block.
//! * The DSP mixer microcode walks one voice at a time: we hand it a 64-byte
//!   *parameter block* (PB) in MEM1 and a mailbox command, it mixes that
//!   voice's next chunk into the armed output buffer, writes the updated PB
//!   back and interrupts us (`DSP` PI line) for the next voice.
//! * Voice sample double-buffers (2 × 1152 bytes) live in ARAM; the host
//!   refills them from MEM1 sample data as the DSP consumes them.
//!
//! The mailbox protocol and PB layout are byte-compatible with libaesnd's:
//! `0xface0080` = set command-PB address, `0xface0010` = mix voice into the
//! fresh output buffer, `0xface0020` = mix next voice, `0xface0100` = output
//! block complete.
//!
//! Note on sample data: `play`/`set_buffer` take `&[u8]` slices in MEM1; the
//! bytes must stay resident (and unchanged in location) until the voice stops
//! — the host refills the ARAM staging buffers from that memory every ~2 ms.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::{aram, dsp, dspcode, hw, irq};

// ---------------------------------------------------------------------------
// constants (from libogc gc/aesndlib.h)
// ---------------------------------------------------------------------------

/// Hardware voice count.
pub const MAX_VOICES: usize = 32;
/// AI DMA block: 2 ms of stereo 16-bit at 48 kHz.
pub const SND_BUFFER_BYTES: usize = 384;
/// Per-voice ARAM staging: 2 ms of input at max 144 kHz.
const DSP_STREAMBUFFER_SIZE: usize = 1152;
/// GameCube AI clock (libogc `DSP_DEFAULT_FREQ` for HW_DOL).
const DSP_DEFAULT_FREQ: f32 = 54_000_000.0 / 1124.0;
/// DSP-visible parameter-block prefix (bytes), cache-block-aligned.
const PB_DSP_SIZE: usize = 64;



// flags
const F_PAUSE: u32 = 0x0000_0008;
const F_LOOP: u32 = 0x0000_0010;
const F_ONCE: u32 = 0x0000_0020;
const F_STREAM: u32 = 0x0000_0040;
const F_FINISHED: u32 = 0x0010_0000;
const F_STOPPED: u32 = 0x0020_0000;
const F_RUNNING: u32 = 0x4000_0000;
const F_USED: u32 = 0x8000_0000;

/// Voice callback states (`voice` cb second argument).
pub const VOICE_STATE_STOPPED: u32 = 0;
pub const VOICE_STATE_RUNNING: u32 = 1;
pub const VOICE_STATE_STREAM: u32 = 2;

/// Voice event callback (`voice`, `VOICE_STATE_*`). Runs in interrupt context.
pub type VoiceCallback = extern "C" fn(usize, u32);
/// Mixed-audio callback: may post-process the about-to-be-armed DMA block
/// (192 stereo s16 frames = 2 ms). Runs in interrupt context.
pub type AudioCallback = extern "C" fn(&mut [i16]);

/// Sample format of voice data.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum VoiceFormat {
    Mono8 = 0,
    Stereo8 = 1,
    Mono16 = 2,
    Stereo16 = 3,
    Mono8Unsigned = 4,
    Stereo8Unsigned = 5,
    Mono16Unsigned = 6,
    Stereo16Unsigned = 7,
}

impl VoiceFormat {
    fn shift(self) -> u32 {
        match self {
            VoiceFormat::Mono8
            | VoiceFormat::Stereo8
            | VoiceFormat::Mono8Unsigned
            | VoiceFormat::Stereo8Unsigned => 0,
            _ => 1,
        }
    }
}

/// Handle to an allocated voice.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Voice(usize);

/// AESND subsystem handle (singleton).
pub struct Aesnd {
    _private: (),
}

/// Init failure reasons.
#[derive(Copy, Clone, Debug)]
pub enum InitError {
    /// `aesnd::init()` was already called.
    InUse,
    /// ARAM exhausted while reserving voice staging buffers.
    AramFull,
    /// The DSP never answered the boot handshake.
    DspBoot,
}

// ---------------------------------------------------------------------------
// parameter block — first PB_DSP_SIZE bytes are shared with the DSP verbatim
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone)]
struct Pb {
    // !! DSP-visible prefix: offsets frozen by the microcode !!
    out_buf: u32,    //  0
    buf_start: u32,  //  4
    buf_end: u32,    //  8
    buf_curr: u32,   // 12
    yn1: u16,        // 16
    yn2: u16,        // 18
    pds: u16,        // 20
    freq_h: u16,     // 22
    freq_l: u16,     // 24
    counter: u16,    // 26
    left: i16,       // 28
    right: i16,      // 30
    volume_l: u16,   // 32
    volume_r: u16,   // 34
    delay: u32,      // 36
    flags: u32,      // 40
    _pad: [u8; 20],  // 44 .. 64
    // ---- host-side tail (never flushed to the DSP) ----
    mram_start: u32,
    mram_curr: u32,
    mram_end: u32,
    stream_last: u32,
    voiceno: u32,
    shift: u32,
    cb: Option<VoiceCallback>,
}

const _: () = assert!(core::mem::offset_of!(Pb, mram_start) == PB_DSP_SIZE);

impl Pb {
    const fn zeroed() -> Pb {
        Pb {
            out_buf: 0,
            buf_start: 0,
            buf_end: 0,
            buf_curr: 0,
            yn1: 0,
            yn2: 0,
            pds: 0,
            freq_h: 0,
            freq_l: 0,
            counter: 0,
            left: 0,
            right: 0,
            volume_l: 0,
            volume_r: 0,
            delay: 0,
            flags: 0,
            _pad: [0; 20],
            mram_start: 0,
            mram_curr: 0,
            mram_end: 0,
            stream_last: 0,
            voiceno: 0,
            shift: 0,
            cb: None,
        }
    }
}

/// `__aesndcopycommand` — copy everything except `out_buf` (owned by the
/// AI-side arming code).
fn copy_pb(dst: &mut Pb, src: &Pb) {
    let out_buf = dst.out_buf;
    *dst = src.clone();
    dst.out_buf = out_buf;
}

// ---------------------------------------------------------------------------
// statics
// ---------------------------------------------------------------------------

#[repr(align(32))]
struct A32<T>(T);

static mut VOICES: [Pb; MAX_VOICES] = [const { Pb::zeroed() }; MAX_VOICES];
/// Command block — the one PB the DSP ever touches directly.
static mut CMD: A32<Pb> = A32(Pb::zeroed());

static mut ABUF: [A32<[u8; SND_BUFFER_BYTES]>; 2] =
    [A32([0; SND_BUFFER_BYTES]), A32([0; SND_BUFFER_BYTES])];
static mut MUTE: A32<[u8; SND_BUFFER_BYTES]> = A32([0; SND_BUFFER_BYTES]);
static mut STREAM_BUF: A32<[u8; DSP_STREAMBUFFER_SIZE * 2]> =
    A32([0; DSP_STREAMBUFFER_SIZE * 2]);

static mut ARAM_BLOCK: [u32; MAX_VOICES] = [0; MAX_VOICES];

// cross-context cells (ISR <-> thread); volatile u32 access only.
static mut CUR_AB: u32 = 0;
static mut CUR_VOICE: u32 = 0;
static mut DSP_INITED: u32 = 0;
static mut DSP_COMPLETE: u32 = 0;
static mut AB_REQUESTED: u32 = 0;
static mut GLOBAL_PAUSE: u32 = 0;
static mut VOICES_STOPPED: u32 = 1;
/// Voice index whose PB currently lives in CMD, if any (ISR-stream refills
/// target the command block). `u32::MAX` when CMD is not checked out.
static mut CMD_VOICE: u32 = u32::MAX;

static mut AUDIO_CB: Option<AudioCallback> = None;

static INITED: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
unsafe fn rd(p: *mut u32) -> u32 {
    core::ptr::read_volatile(p)
}

#[inline(always)]
unsafe fn wr(p: *mut u32, v: u32) {
    core::ptr::write_volatile(p, v);
}

#[inline(always)]
unsafe fn cmd() -> &'static mut Pb {
    &mut (*(&raw mut CMD)).0
}

#[inline(always)]
unsafe fn voices() -> &'static mut [Pb; MAX_VOICES] {
    &mut *(&raw mut VOICES)
}

// ---------------------------------------------------------------------------
// AI DMA low level (same register program as audio.rs; kept separate so
// either module owns the AI exclusively)
// ---------------------------------------------------------------------------

const DSP_BASE: u32 = 0xCC00_5000;
const AI_CTRL: u32 = 0xCC00_6C00;
const AI_VOL: u32 = 0xCC00_6C04;
const AI_CTR_SAMP: u32 = 0xCC00_6C08;

const AI_PSTAT: u16 = 0x01;
const AI_AISFR: u16 = 0x02;
const AI_AIINTMSK: u16 = 0x04;
const AI_AIINT: u16 = 0x08;
const AI_SCRESET: u16 = 0x20;
const AI_DMAFR: u16 = 0x40;

unsafe fn ai_hw_init() {
    let _ = hw::read16(AI_CTRL);
    hw::write16(AI_CTRL, AI_SCRESET);
    let mut c = hw::read16(AI_CTRL);
    c &= !(AI_AISFR | AI_DMAFR); // 48 kHz stream rate
    c |= AI_PSTAT | AI_AIINTMSK; // play + DMA-complete interrupts
    hw::write16(AI_CTRL, c);
    hw::write16(AI_VOL, 0xFFFF);
    hw::write16(AI_CTR_SAMP, AI_SCRESET);
}

unsafe fn ai_ack() {
    let mut c = hw::read16(AI_CTRL);
    c |= AI_AIINT;
    hw::write16(AI_CTRL, c);
}

unsafe fn ai_arm_dma(ptr_phys: u32, len: u32) {
    hw::write16(DSP_BASE + 2 * 24, ((ptr_phys >> 16) & 0x1fff) as u16);
    hw::write16(DSP_BASE + 2 * 25, (ptr_phys & 0xffe0) as u16);
    hw::write16(
        DSP_BASE + 2 * 27,
        (hw::read16(DSP_BASE + 2 * 27) & !0x7fff) | (((len >> 5) as u16) & 0x7fff),
    );
    hw::write16(DSP_BASE + 2 * 27, hw::read16(DSP_BASE + 2 * 27) | 0x8000);
}

// ---------------------------------------------------------------------------
// mailbox helpers
// ---------------------------------------------------------------------------

// libogc's `__dsp_def_taskcb` ack scheme (single-task list): on TASK_DONE
// the DSP expects 0xCDD10003 (resume current), on TASK_DONE2 0xCDD10002.
use dsp::{MAIL_ACK_DONE as ACK_DONE2, MAIL_ACK_KILL as ACK_RESUME, MAIL_TASK_DONE2 as TASK_DONE2, MAIL_TASK_DONE as TASK_DONE, MAIL_TASK_REQ as TASK_REQ, MAIL_TASK_RUN as TASK_RUN};

const M_SET_CMD_ADDR: u32 = 0xface_0080;
const M_MIX_FIRST: u32 = 0xface_0010;
const M_MIX_NEXT: u32 = 0xface_0020;
const M_BUF_DONE: u32 = 0xface_0100;

#[inline]
fn send(mail: u32) {
    dsp::send_mail_to(mail);
    // libogc spins on `DSP_CheckMailTo()` after every send; the DSP consumes
    // mailbox words immediately. Bounded spin: if the DSP is dead (e.g. under
    // Dolphin's HLE mixer fallback) drop the rest of the transaction instead
    // of locking the machine.
    let mut n = 0u32;
    while !dsp::check_mail_to_free() {
        n += 1;
        if n > 1_000_000 {
            return;
        }
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// DSP PI interrupt handler — the AESND mailbox protocol
// ---------------------------------------------------------------------------

extern "C" fn dsp_isr(_: irq::Source) {
    dsp::ack_interrupt();
    let mut n = 0u32;
    while !dsp::check_mail_from() {
        n += 1;
        if n > 1_000_000 {
            return;
        }
        core::hint::spin_loop();
    }
    let mail = dsp::read_mail_from();
    match mail {
        TASK_RUN => {
            // __dsp_initcallback: hand the mixer its command block address.
            send(M_SET_CMD_ADDR);
            send(hw::virt_to_phys(core::ptr::addr_of!(CMD) as *const u8));
            unsafe {
                wr(&raw mut DSP_INITED, 1);
                wr(&raw mut DSP_COMPLETE, 1);
            }
        }
        TASK_REQ => unsafe { request_cb() },
        TASK_DONE => {
            // single-task target: nothing to switch to — resume ack.
            send(ACK_RESUME);
        }
        TASK_DONE2 => unsafe {
            wr(&raw mut DSP_INITED, 0);
            wr(&raw mut DSP_COMPLETE, 0);
            wr(&raw mut AB_REQUESTED, 0);
            send(ACK_DONE2);
        },
        _ => {}
    }
}

/// `__dsp_requestcallback`: the DSP just wrote back the command PB after
/// finishing a voice chunk.
unsafe fn request_cb() {
    if rd(&raw mut AB_REQUESTED) == 1 {
        // this REQ is the DSP's "output block complete" answer.
        wr(&raw mut AB_REQUESTED, 0);
        wr(&raw mut DSP_COMPLETE, 1);
        return;
    }

    let cmd = cmd();
    hw::dc_invalidate_range(cmd as *mut Pb as *const u8, PB_DSP_SIZE);

    if cmd.flags & F_FINISHED != 0 {
        cmd.flags &= !F_FINISHED;
        handle_request(cmd);

        let cur = rd(&raw mut CUR_VOICE) as usize;
        if cmd.flags & F_STOPPED != 0 {
            if let Some(cb) = cmd.cb {
                cb(cur, VOICE_STATE_STOPPED);
            }
        }
        copy_pb(&mut voices()[cur], cmd);
        wr(&raw mut CMD_VOICE, u32::MAX);

        let mut next = cur + 1;
        while next < MAX_VOICES {
            let f = voices()[next].flags;
            if f & F_USED != 0 && f & F_STOPPED == 0 {
                break;
            }
            next += 1;
        }
        if next < MAX_VOICES {
            wr(&raw mut CUR_VOICE, next as u32);
            copy_pb(cmd, &voices()[next]);
            wr(&raw mut CMD_VOICE, next as u32);
            if let Some(cb) = cmd.cb {
                cb(next, VOICE_STATE_RUNNING);
            }
            hw::dc_flush_range(cmd as *mut Pb as *const u8, PB_DSP_SIZE);
            send(M_MIX_NEXT);
            return;
        }
        wr(&raw mut CUR_VOICE, next as u32);
    }

    if rd(&raw mut CUR_VOICE) as usize >= MAX_VOICES {
        send(M_BUF_DONE);
        wr(&raw mut AB_REQUESTED, 1);
    }
}

// ---------------------------------------------------------------------------
// voice buffer refill (`__aesndhandlerequest` / `__aesndfillbuffer`, HW_DOL
// variant: samples staged into ARAM double-buffers per voice)
// ---------------------------------------------------------------------------

unsafe fn fill_buffer(pb: &mut Pb, which: u32) {
    let mut addr = ARAM_BLOCK[pb.voiceno as usize];
    if which != 0 {
        addr += DSP_STREAMBUFFER_SIZE as u32;
    }
    let avail = (pb.mram_end - pb.mram_curr) as usize;
    let copy = avail.min(DSP_STREAMBUFFER_SIZE);

    let stage = &mut (*(&raw mut STREAM_BUF)).0;
    core::ptr::copy_nonoverlapping(pb.mram_curr as *const u8, stage.as_mut_ptr(), copy);
    if copy < DSP_STREAMBUFFER_SIZE {
        core::ptr::write_bytes(stage.as_mut_ptr().add(copy), 0, DSP_STREAMBUFFER_SIZE - copy);
    }
    hw::dc_flush_range(stage.as_ptr(), DSP_STREAMBUFFER_SIZE);
    aram::dma_write_sync(hw::virt_to_phys(stage.as_ptr()), addr, DSP_STREAMBUFFER_SIZE as u32);

    pb.mram_curr += copy as u32;
}

unsafe fn handle_request(pb: &mut Pb) {
    if pb.mram_curr >= pb.mram_end {
        if pb.flags & F_STREAM != 0 {
            if let Some(cb) = pb.cb {
                cb(pb.voiceno as usize, VOICE_STATE_STREAM);
            }
        }
        if pb.flags & F_ONCE != 0 {
            pb.flags |= F_STOPPED;
            return;
        } else if pb.flags & F_LOOP != 0 {
            pb.mram_curr = pb.mram_start;
        }
    }

    if pb.buf_start != 0 {
        let curr_pos = pb.buf_curr;
        if curr_pos < pb.stream_last {
            fill_buffer(pb, 1);
        }
        let half = pb.buf_start + ((DSP_STREAMBUFFER_SIZE as u32) >> pb.shift);
        if curr_pos >= half && pb.stream_last < half {
            fill_buffer(pb, 0);
        }
        pb.stream_last = curr_pos;
        return;
    }

    // first fill: stage both halves (2 * 1152 bytes) in one DMA.
    let blk = ARAM_BLOCK[pb.voiceno as usize];
    pb.buf_start = blk >> pb.shift;
    pb.buf_end = (blk + (DSP_STREAMBUFFER_SIZE as u32) * 2 - (1 << pb.shift)) >> pb.shift;
    pb.buf_curr = pb.buf_start;

    let avail = (pb.mram_end - pb.mram_curr) as usize;
    let copy = avail.min(DSP_STREAMBUFFER_SIZE * 2);

    let stage = &mut (*(&raw mut STREAM_BUF)).0;
    core::ptr::copy_nonoverlapping(pb.mram_curr as *const u8, stage.as_mut_ptr(), copy);
    if copy < stage.len() {
        core::ptr::write_bytes(stage.as_mut_ptr().add(copy), 0, stage.len() - copy);
    }
    hw::dc_flush_range(stage.as_ptr(), stage.len());
    aram::dma_write_sync(hw::virt_to_phys(stage.as_ptr()), blk, stage.len() as u32);

    pb.mram_curr += copy as u32;
    pb.flags |= F_RUNNING;
}

// ---------------------------------------------------------------------------
// AI DMA-complete interrupt — re-arm, then kick the DSP for the next block
// ---------------------------------------------------------------------------

extern "C" fn ai_isr(_: irq::Source) {
    unsafe {
        ai_ack();

        let cur = rd(&raw mut CUR_AB) ^ 1;
        wr(&raw mut CUR_AB, cur);

        let paused = rd(&raw mut GLOBAL_PAUSE) != 0;
        let stopped = rd(&raw mut VOICES_STOPPED) != 0;
        let buf: *mut u8 = if paused || stopped {
            (*(&raw mut MUTE)).0.as_mut_ptr()
        } else {
            (*(&raw mut ABUF))[cur as usize].0.as_mut_ptr()
        };

        if let Some(cb) = *(&raw const AUDIO_CB) {
            cb(core::slice::from_raw_parts_mut(buf as *mut i16, SND_BUFFER_BYTES / 2));
        }
        ai_arm_dma(hw::virt_to_phys(buf), SND_BUFFER_BYTES as u32);

        if paused {
            return;
        }
        if rd(&raw mut DSP_INITED) == 0 || rd(&raw mut DSP_COMPLETE) == 0 {
            return;
        }

        // first active voice?
        let v = voices();
        let mut i = 0usize;
        while i < MAX_VOICES {
            let f = v[i].flags;
            if f & F_USED != 0 && f & F_STOPPED == 0 {
                break;
            }
            i += 1;
        }
        if i >= MAX_VOICES {
            wr(&raw mut VOICES_STOPPED, 1);
            return;
        }

        wr(&raw mut DSP_COMPLETE, 0);
        wr(&raw mut VOICES_STOPPED, 0);
        wr(&raw mut CUR_VOICE, i as u32);

        let cmd = cmd();
        copy_pb(cmd, &v[i]);
        wr(&raw mut CMD_VOICE, i as u32);
        if let Some(cb) = cmd.cb {
            cb(i, VOICE_STATE_RUNNING);
        }
        cmd.out_buf = hw::virt_to_phys((*(&raw mut ABUF))[cur as usize].0.as_ptr());
        hw::dc_flush_range(cmd as *mut Pb as *const u8, PB_DSP_SIZE);

        // tell the DSP to mix this voice's next chunk into `out_buf`.
        send(M_MIX_FIRST);
    }
}

// ---------------------------------------------------------------------------
// public API (thread context)
// ---------------------------------------------------------------------------

/// Initialize the AESND mixer: boot the DSP with the mixer microcode, wire up
/// the AI DMA ping-pong, reserve ARAM staging for every voice.
pub fn init() -> Result<Aesnd, InitError> {
    if INITED.swap(1, Ordering::AcqRel) != 0 {
        return Err(InitError::InUse);
    }

    unsafe {
        // per-voice ARAM staging (2 * 1152 B, 32-byte aligned)
        for b in &mut *(&raw mut ARAM_BLOCK) {
            *b = match aram::alloc(DSP_STREAMBUFFER_SIZE * 2) {
                Some(a) => a,
                None => return Err(InitError::AramFull),
            };
        }

        // silence buffers
        for i in 0..2 {
            hw::dc_flush_range((*(&raw mut ABUF))[i].0.as_ptr(), SND_BUFFER_BYTES);
        }
        hw::dc_flush_range((*(&raw mut MUTE)).0.as_ptr(), SND_BUFFER_BYTES);

        irq::register(irq::Source::DspDma, Some(dsp_isr));
        dsp::reset();
        if dsp::boot_microcode(
            hw::virt_to_phys(dspcode::AESND_DSP_MIXER.as_ptr()),
            0x0000,
            dspcode::AESND_DSP_MIXER_SIZE as u16,
            0x0010,
        )
        .is_err()
        {
            return Err(InitError::DspBoot);
        }

        irq::register(irq::Source::Ai, Some(ai_isr));
        ai_hw_init();
        // arm the DMA on the (zeroed) first audio buffer — 2 ms of silence.
        ai_arm_dma(
            hw::virt_to_phys((*(&raw mut ABUF))[0].0.as_ptr()),
            SND_BUFFER_BYTES as u32,
        );

        wr(&raw mut CUR_AB, 0);
        wr(&raw mut VOICES_STOPPED, 1);
        wr(&raw mut CMD_VOICE, u32::MAX);

        irq::enable_source(irq::Source::Ai);
        irq::enable_source(irq::Source::DspDma);
        irq::enable();
    }

    Ok(Aesnd { _private: () })
}

fn set_freq(pb: &mut Pb, hz: f32) {
    let ratio = (0x10000 as f32 * (hz / DSP_DEFAULT_FREQ) + 0.5) as u32;
    pb.freq_h = (ratio >> 16) as u16;
    pb.freq_l = (ratio & 0xffff) as u16;
}

impl Aesnd {
    /// Allocate a voice, or `None` when all 32 are used.
    pub fn alloc_voice(&self) -> Option<Voice> {
        let _lock = irq::IrqLock::take();
        unsafe {
            let v = voices();
            for i in 0..MAX_VOICES {
                if v[i].flags & F_USED == 0 {
                    let p = &mut v[i];
                    *p = Pb::zeroed();
                    p.voiceno = i as u32;
                    p.flags = F_USED | F_STOPPED;
                    p.volume_l = 0x100;
                    p.volume_r = 0x100;
                    p.freq_h = 0x0001;
                    p.freq_l = 0x0000;
                    return Some(Voice(i));
                }
            }
        }
        None
    }

    /// Release a voice back to the pool.
    pub fn free_voice(&self, v: Voice) {
        let _lock = irq::IrqLock::take();
        unsafe {
            voices()[v.0] = Pb::zeroed();
        }
    }

    /// Start (or re-start) a voice playing `data` (raw samples in MEM1).
    /// `freq_hz` is the voice's playback rate; play a sample at its recorded
    /// rate for neutral pitch. `looped` wraps the buffer at the end.
    pub fn play(&self, v: Voice, fmt: VoiceFormat, data: &[u8], freq_hz: f32, looped: bool) {
        let _lock = irq::IrqLock::take();
        self.set_buffer_raw(v, data);
        unsafe {
            let p = &mut voices()[v.0];
            p.flags = (p.flags & !0x07) | (fmt as u32);
            p.shift = fmt.shift();
            set_freq(p, freq_hz);
            p.flags &= !(F_RUNNING | F_STOPPED | F_LOOP | F_ONCE);
            p.flags |= if looped { F_LOOP } else { F_ONCE };
            p.buf_start = 0;
            p.buf_curr = 0;
            p.buf_end = 0;
            p.stream_last = 0;
            p.delay = 0;
            p.pds = 0;
            p.yn1 = 0;
            p.yn2 = 0;
            p.counter = 0;
        }
    }

    /// Replace a voice's sample buffer (streaming refill; valid from the
    /// `VOICE_STATE_STREAM` callback too).
    pub fn set_buffer(&self, v: Voice, data: &[u8]) {
        let _lock = irq::IrqLock::take();
        self.set_buffer_raw(v, data);
    }

    fn set_buffer_raw(&self, v: Voice, data: &[u8]) {
        unsafe {
            // When called from a stream callback, the PB in flight is the
            // command block — patch that one instead.
            let p = if rd(&raw mut CMD_VOICE) == v.0 as u32 {
                cmd()
            } else {
                &mut voices()[v.0]
            };
            p.mram_start = data.as_ptr() as u32;
            p.mram_curr = data.as_ptr() as u32;
            p.mram_end = data.as_ptr() as u32 + data.len() as u32;
        }
    }

    /// Per-voice volume, `0x0000`..=`0x0100`+ (`0x100` = unity).
    pub fn set_volume(&self, v: Voice, left: u16, right: u16) {
        let _lock = irq::IrqLock::take();
        unsafe {
            let p = &mut voices()[v.0];
            p.volume_l = left;
            p.volume_r = right;
        }
    }

    /// Change playback rate mid-flight (Hz).
    pub fn set_frequency(&self, v: Voice, freq_hz: f32) {
        let _lock = irq::IrqLock::take();
        unsafe {
            set_freq(&mut voices()[v.0], freq_hz);
        }
    }

    /// Stop (true) / un-stop (false) a voice.
    pub fn set_stop(&self, v: Voice, stop: bool) {
        let _lock = irq::IrqLock::take();
        unsafe {
            let p = &mut voices()[v.0];
            if stop {
                p.flags |= F_STOPPED;
            } else {
                p.flags &= !F_STOPPED;
            }
        }
    }

    /// Mute (pause) a voice without losing its position.
    pub fn set_mute(&self, v: Voice, mute: bool) {
        let _lock = irq::IrqLock::take();
        unsafe {
            let p = &mut voices()[v.0];
            if mute {
                p.flags |= F_PAUSE;
            } else {
                p.flags &= !F_PAUSE;
            }
        }
    }

    /// Toggle loop mode on a playing voice.
    pub fn set_loop(&self, v: Voice, looped: bool) {
        let _lock = irq::IrqLock::take();
        unsafe {
            let p = &mut voices()[v.0];
            if looped {
                p.flags |= F_LOOP;
            } else {
                p.flags &= !F_LOOP;
            }
        }
    }

    /// Mark a voice as streaming (its callback gets `VOICE_STATE_STREAM`
    /// whenever the sample window is exhausted).
    pub fn set_stream(&self, v: Voice, stream: bool) {
        let _lock = irq::IrqLock::take();
        unsafe {
            let p = &mut voices()[v.0];
            if stream {
                p.flags |= F_STREAM;
            } else {
                p.flags &= !F_STREAM;
            }
        }
    }

    /// Register/replace a voice's event callback.
    pub fn register_voice_callback(&self, v: Voice, cb: Option<VoiceCallback>) {
        let _lock = irq::IrqLock::take();
        unsafe {
            voices()[v.0].cb = cb;
        }
    }

    /// Pause the whole mixer (DMA keeps running, fed with silence).
    pub fn pause(&self, paused: bool) {
        let _lock = irq::IrqLock::take();
        unsafe {
            wr(&raw mut GLOBAL_PAUSE, paused as u32);
        }
    }
}

/// Post-process each mixed output block before it is armed on the AI DMA
/// (e.g. visualization or master FX). Runs in interrupt context.
pub fn on_audio_buffer(_: &Aesnd, cb: Option<AudioCallback>) {
    let _lock = irq::IrqLock::take();
    unsafe {
        *(&raw mut AUDIO_CB) = cb;
    }
}
