//! AESND polyphonic audio — Rust port of libogc's `libaesnd/aesndlib.c`
//! host protocol onto the DSP mixer microcode in [`crate::dspcode`].
//!
//! The public surface deliberately mirrors libogc's `AESND_*` API 1:1
//! (free functions, same argument order, same constants) so that existing
//! GameCube audio documentation and code samples transfer and diffs against
//! the C original stay reviewable. Names are snake_cased per Rust idiom.
//!
//! Layout of the system:
//!
//! * The AI DMA engine streams 2 ms stereo s16 blocks ([`SND_BUFFERSIZE`]
//!   bytes) from one of two ping-pong buffers out to the DAC at 48 kHz.
//!   Every block-complete interrupt re-arms the DMA and kicks the DSP mixer
//!   for the next block.
//! * The DSP mixer microcode walks one voice at a time: we hand it a 64-byte
//!   *parameter block* (PB) in MEM1 and a mailbox command, it mixes that
//!   voice's next chunk into the armed output buffer, DMAs the updated PB
//!   back and interrupts us (`DSP` PI line) for the next voice.
//! * Voice sample double-buffers (2 × [`DSP_STREAMBUFFER_SIZE`]) live in
//!   ARAM; the host refills them from MEM1 sample data as the DSP consumes
//!   them.
//!
//! The mailbox protocol and PB layout are byte-compatible with libaesnd's:
//! `0xface0080` = set command-PB address, `0xface0010` = mix first voice,
//! `0xface0020` = mix next voice, `0xface0100` = output block complete.
//!
//! Note on sample lifetimes: `play_voice`/`set_voice_buffer` take raw MEM1
//! pointers; the bytes must stay resident (and at the same address) until
//! the voice finishes — the host refills the ARAM staging buffers from that
//! memory every ~2 ms.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::{aram, dsp, dspcode, hw, irq};

// ---------------------------------------------------------------------------
// constants (from libogc gc/aesndlib.h)
// ---------------------------------------------------------------------------

pub const MAX_VOICES: usize = 32;
/// Bytes of one AI DMA block: 2 ms of stereo 16-bit at 48 kHz.
pub const SND_BUFFERSIZE: usize = 384;
/// Per-voice ARAM staging half: 2 ms of input at max 144 kHz.
pub const DSP_STREAMBUFFER_SIZE: usize = 1152;
/// GameCube AI clock (libogc `DSP_DEFAULT_FREQ` for HW_DOL).
const DSP_DEFAULT_FREQ: f32 = 54_000_000.0 / 1124.0;
/// DSP-visible parameter-block prefix (bytes), cache-block-aligned.
const PB_STRUCT_SIZE: usize = 64;

// voice formats (`AESND_PlayVoice`'s `format` argument)
pub const VOICE_MONO8: u32 = 0;
pub const VOICE_STEREO8: u32 = 1;
pub const VOICE_MONO16: u32 = 2;
pub const VOICE_STEREO16: u32 = 3;
pub const VOICE_MONO8_UNSIGNED: u32 = 4;
pub const VOICE_STEREO8_UNSIGNED: u32 = 5;
pub const VOICE_MONO16_UNSIGNED: u32 = 6;
pub const VOICE_STEREO16_UNSIGNED: u32 = 7;

// PB flag bits
const VOICE_PAUSE: u32 = 0x0000_0008;
const VOICE_LOOP: u32 = 0x0000_0010;
const VOICE_ONCE: u32 = 0x0000_0020;
const VOICE_STREAM: u32 = 0x0000_0040;
const VOICE_FINISHED: u32 = 0x0010_0000;
const VOICE_STOPPED: u32 = 0x0020_0000;
const VOICE_RUNNING: u32 = 0x4000_0000;
const VOICE_USED: u32 = 0x8000_0000;

// voice callback states
pub const VOICE_STATE_STOPPED: u32 = 0;
pub const VOICE_STATE_RUNNING: u32 = 1;
pub const VOICE_STATE_STREAM: u32 = 2;

/// Voice event callback. Runs in interrupt context; the `pb` argument is the
/// live parameter block (the *command* block while the DSP owns the voice).
pub type AesndVoiceCallback = extern "C" fn(*mut AesndPb, u32);
/// Mixed-audio callback: the DMA is about to be (re)armed with `buf[..len]`;
/// may post-process in place. Runs in interrupt context.
pub type AesndAudioCallback = extern "C" fn(*mut u8, usize);

// ---------------------------------------------------------------------------
// parameter block — first PB_STRUCT_SIZE bytes are shared with the DSP
// verbatim (layout frozen by the microcode); the tail is host-only
// ---------------------------------------------------------------------------

/// Voice parameter block. Opaque to applications, exactly like libogc's
/// `typedef struct aesndpb_t AESNDPB`.
#[repr(C)]
#[derive(Clone)]
pub struct AesndPb {
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
    cb: Option<AesndVoiceCallback>,
}

const _: () = assert!(core::mem::offset_of!(AesndPb, mram_start) == PB_STRUCT_SIZE);

impl AesndPb {
    const fn zeroed() -> AesndPb {
        AesndPb {
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

/// [`__aesndcopycommand`] — copy everything except `out_buf` (owned by the
/// AI-side arming code).
fn copy_pb(dst: &mut AesndPb, src: &AesndPb) {
    let out_buf = dst.out_buf;
    *dst = src.clone();
    dst.out_buf = out_buf;
}

// ---------------------------------------------------------------------------
// statics
// ---------------------------------------------------------------------------

#[repr(align(32))]
struct A32<T>(T);

static mut VOICE_PB: [AesndPb; MAX_VOICES] = [const { AesndPb::zeroed() }; MAX_VOICES];
/// Command block — the one PB the DSP ever touches directly.
static mut COMMAND: A32<AesndPb> = A32(AesndPb::zeroed());

static mut ABUF: [A32<[u8; SND_BUFFERSIZE]>; 2] =
    [A32([0; SND_BUFFERSIZE]), A32([0; SND_BUFFERSIZE])];
static mut MUTE: A32<[u8; SND_BUFFERSIZE]> = A32([0; SND_BUFFERSIZE]);
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
static mut DSP_PROCESS_START: u64 = 0;
static mut DSP_PROCESS_TIME: u64 = 0;

static mut AUDIO_CB: Option<AesndAudioCallback> = None;

static INITED: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
unsafe fn rd(p: *mut u32) -> u32 {
    core::ptr::read_volatile(p)
}

#[inline(always)]
unsafe fn wr(p: *mut u32, v: u32) {
    core::ptr::write_volatile(p, v)
}

#[inline(always)]
unsafe fn command() -> &'static mut AesndPb {
    &mut (*(&raw mut COMMAND)).0
}

#[inline(always)]
unsafe fn voicepbs() -> &'static mut [AesndPb; MAX_VOICES] {
    &mut *(&raw mut VOICE_PB)
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

/// [`AUDIO_Init`] + [`AUDIO_SetDSPSampleRate(48k)`].
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

/// [`AUDIO_StopDMA`].
unsafe fn ai_stop_dma() {
    let mut c = hw::read16(AI_CTRL);
    c &= !AI_AIINTMSK;
    c |= AI_AIINT;
    hw::write16(AI_CTRL, c);
}

unsafe fn ai_ack() {
    let mut c = hw::read16(AI_CTRL);
    c |= AI_AIINT;
    hw::write16(AI_CTRL, c);
}

/// [`AUDIO_InitDMA`] — arm the AI DMA engine on a 32-byte-aligned MEM1 block.
unsafe fn ai_start_dma(ptr_phys: u32, len: u32) {
    hw::write16(DSP_BASE + 2 * 24, ((ptr_phys >> 16) & 0x1fff) as u16);
    hw::write16(DSP_BASE + 2 * 25, (ptr_phys & 0xffe0) as u16);
    hw::write16(
        DSP_BASE + 2 * 27,
        (hw::read16(DSP_BASE + 2 * 27) & !0x7fff) | (((len >> 5) as u16) & 0x7fff),
    );
    hw::write16(DSP_BASE + 2 * 27, hw::read16(DSP_BASE + 2 * 27) | 0x8000);
}

// ---------------------------------------------------------------------------
// mailbox helpers (libogc spins on DSP_CheckMailTo() after every send; the
// spins are bounded so a dead DSP — e.g. Dolphin's HLE fallback — degrades
// to silence instead of a lockup)
// ---------------------------------------------------------------------------

use dsp::{MAIL_ACK_DONE as ACK_TASK_DONE2, MAIL_ACK_KILL as ACK_TASK_DONE, MAIL_TASK_DONE2 as TASK_DONE2, MAIL_TASK_DONE as TASK_DONE, MAIL_TASK_REQ as TASK_REQ, MAIL_TASK_RUN as TASK_RUN};

const M_AESND_SET_CMD_ADDR: u32 = 0xface_0080;
const M_AESND_MIX_FIRST: u32 = 0xface_0010;
const M_AESND_MIX_NEXT: u32 = 0xface_0020;
const M_AESND_BUF_DONE: u32 = 0xface_0100;
const M_AESND_KILL: u32 = 0xface_dead;

#[inline]
fn send_mail(mail: u32) {
    dsp::send_mail_to(mail);
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
// DSP PI interrupt handler — the AESND mailbox protocol (`__dsp_def_taskcb`
// specialized to our single resident task)
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
            send_mail(M_AESND_SET_CMD_ADDR);
            send_mail(hw::virt_to_phys(core::ptr::addr_of!(COMMAND) as *const u8));
            unsafe {
                wr(&raw mut DSP_INITED, 1);
                wr(&raw mut DSP_COMPLETE, 1);
            }
        }
        TASK_REQ => unsafe { dsp_request_cb() },
        TASK_DONE => {
            // single resident task: libogc answers 0xCDD10003 (keep running).
            send_mail(ACK_TASK_DONE);
        }
        TASK_DONE2 => unsafe {
            // mixer sent the kill response to M_AESND_KILL.
            wr(&raw mut DSP_INITED, 0);
            wr(&raw mut DSP_COMPLETE, 0);
            wr(&raw mut AB_REQUESTED, 0);
            send_mail(ACK_TASK_DONE2);
        },
        _ => {}
    }
}

/// [`__dsp_requestcallback`]: the DSP just wrote back the command PB after
/// finishing a voice chunk.
unsafe fn dsp_request_cb() {
    if rd(&raw mut AB_REQUESTED) == 1 {
        // this REQ is the DSP's "output block complete" answer.
        wr(&raw mut AB_REQUESTED, 0);
        wr(&raw mut DSP_COMPLETE, 1);
        let t = hw::mftb().wrapping_sub(core::ptr::read_volatile(&raw const DSP_PROCESS_START));
        core::ptr::write_volatile(&raw mut DSP_PROCESS_TIME, t);
        return;
    }

    let cmd = command();
    hw::dc_invalidate_range(cmd as *mut AesndPb as *const u8, PB_STRUCT_SIZE);

    if cmd.flags & VOICE_FINISHED != 0 {
        cmd.flags &= !VOICE_FINISHED;
        handle_request(cmd);

        let cur = rd(&raw mut CUR_VOICE) as usize;
        if cmd.flags & VOICE_STOPPED != 0 {
            if let Some(cb) = cmd.cb {
                cb(cmd, VOICE_STATE_STOPPED);
            }
        }
        copy_pb(&mut voicepbs()[cur], cmd);

        let mut next = cur + 1;
        while next < MAX_VOICES {
            let f = voicepbs()[next].flags;
            if f & VOICE_USED != 0 && f & VOICE_STOPPED == 0 {
                break;
            }
            next += 1;
        }
        if next < MAX_VOICES {
            wr(&raw mut CUR_VOICE, next as u32);
            copy_pb(cmd, &voicepbs()[next]);
            if let Some(cb) = cmd.cb {
                cb(cmd, VOICE_STATE_RUNNING);
            }
            hw::dc_flush_range(cmd as *mut AesndPb as *const u8, PB_STRUCT_SIZE);
            send_mail(M_AESND_MIX_NEXT);
            return;
        }
        wr(&raw mut CUR_VOICE, next as u32);
    }

    if rd(&raw mut CUR_VOICE) as usize >= MAX_VOICES {
        send_mail(M_AESND_BUF_DONE);
        wr(&raw mut AB_REQUESTED, 1);
    }
}

// ---------------------------------------------------------------------------
// voice buffer refill (`__aesndhandlerequest` / `__aesndfillbuffer`, HW_DOL
// variant: samples staged into ARAM double-buffers per voice)
// ---------------------------------------------------------------------------

unsafe fn fill_buffer(pb: &mut AesndPb, which: u32) {
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

unsafe fn handle_request(pb: &mut AesndPb) {
    if pb.mram_curr >= pb.mram_end {
        if pb.flags & VOICE_STREAM != 0 {
            if let Some(cb) = pb.cb {
                cb(pb, VOICE_STATE_STREAM);
            }
        }
        if pb.flags & VOICE_ONCE != 0 {
            pb.flags |= VOICE_STOPPED;
            return;
        } else if pb.flags & VOICE_LOOP != 0 {
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
    pb.flags |= VOICE_RUNNING;
}

// ---------------------------------------------------------------------------
// AI DMA-complete interrupt — re-arm, then kick the DSP for the next block
// (`__audio_dma_callback`)
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
            cb(buf, SND_BUFFERSIZE);
        }
        ai_start_dma(hw::virt_to_phys(buf), SND_BUFFERSIZE as u32);

        if paused {
            return;
        }
        if rd(&raw mut DSP_INITED) == 0 || rd(&raw mut DSP_COMPLETE) == 0 {
            return;
        }

        // first active voice?
        let pbs = voicepbs();
        let mut i = 0usize;
        while i < MAX_VOICES {
            let f = pbs[i].flags;
            if f & VOICE_USED != 0 && f & VOICE_STOPPED == 0 {
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

        let cmd = command();
        copy_pb(cmd, &pbs[i]);
        if let Some(cb) = cmd.cb {
            cb(cmd, VOICE_STATE_RUNNING);
        }
        cmd.out_buf = hw::virt_to_phys((*(&raw mut ABUF))[cur as usize].0.as_ptr());
        hw::dc_flush_range(cmd as *mut AesndPb as *const u8, PB_STRUCT_SIZE);

        core::ptr::write_volatile(&raw mut DSP_PROCESS_START, hw::mftb());
        // tell the DSP to mix this voice's next chunk into `out_buf`.
        send_mail(M_AESND_MIX_FIRST);
    }
}

// ---------------------------------------------------------------------------
// public API — mirrors libogc's aesndlib.h 1:1
// (`AESND_InvokeX` -> `aesnd::invoke_x`)
// ---------------------------------------------------------------------------

/// [`AESND_Init`].
pub fn init() -> Result<(), InitError> {
    if INITED.swap(1, Ordering::AcqRel) != 0 {
        return Err(InitError::InUse);
    }

    unsafe {
        // per-voice ARAM staging (2 * 1152 B, 32-byte aligned) — the
        // HW_DOL path of AESND_Init (AR_Init/AR_Alloc equivalent).
        for b in &mut *(&raw mut ARAM_BLOCK) {
            *b = match aram::alloc(DSP_STREAMBUFFER_SIZE * 2) {
                Some(a) => a,
                None => return Err(InitError::AramFull),
            };
        }

        for i in 0..2 {
            hw::dc_flush_range((*(&raw mut ABUF))[i].0.as_ptr(), SND_BUFFERSIZE);
        }
        hw::dc_flush_range((*(&raw mut MUTE)).0.as_ptr(), SND_BUFFERSIZE);

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
        ai_start_dma(
            hw::virt_to_phys((*(&raw mut ABUF))[0].0.as_ptr()),
            SND_BUFFERSIZE as u32,
        );

        wr(&raw mut CUR_AB, 0);
        wr(&raw mut VOICES_STOPPED, 1);

        irq::enable_source(irq::Source::Ai);
        irq::enable_source(irq::Source::DspDma);
        irq::enable();
    }

    Ok(())
}

/// [`AESND_Reset`]: kill the DSP task, stop the AI DMA.
pub fn reset() {
    unsafe {
        let _lock = irq::IrqLock::take();
        if INITED.load(Ordering::Acquire) == 1 {
            ai_stop_dma();
            send_mail(M_AESND_KILL);
            // wait for the DSP's kill handshake (bounded)
            let mut n = 0u32;
            while rd(&raw mut DSP_INITED) != 0 {
                n += 1;
                if n > 1_000_000 {
                    break;
                }
                core::hint::spin_loop();
            }
            INITED.store(0, Ordering::Release);
        }
        hw::sync();
    }
}

/// [`AESND_Pause`] global output gate (DMA keeps running, fed with silence).
pub fn pause(paused: bool) {
    unsafe {
        let _lock = irq::IrqLock::take();
        wr(&raw mut GLOBAL_PAUSE, paused as u32);
    }
}

/// [`AESND_GetDSPProcessTime`] — µs the DSP spent mixing the last block.
pub fn get_dsp_process_time() -> u32 {
    unsafe {
        let ticks = core::ptr::read_volatile(&raw const DSP_PROCESS_TIME);
        // timebase: 40.5 MHz (405 ticks per 10 µs)
        (ticks / 40) as u32
    }
}

/// [`AESND_GetDSPProcessUsage`] — % of one 2 ms block the mixer consumed.
pub fn get_dsp_process_usage() -> f32 {
    (get_dsp_process_time() as f32 * 100.0) / 2000.0
}

/// [`AESND_RegisterAudioCallback`]: returns the previously registered one.
pub fn register_audio_callback(cb: Option<AesndAudioCallback>) -> Option<AesndAudioCallback> {
    unsafe {
        let _lock = irq::IrqLock::take();
        core::mem::replace(&mut *(&raw mut AUDIO_CB), cb)
    }
}

#[inline(always)]
unsafe fn pb<'a>(pb: *mut AesndPb) -> &'a mut AesndPb {
    &mut *pb
}

/// [`AESND_AllocateVoice`]. Returns null when all [`MAX_VOICES`] are used.
pub fn allocate_voice(cb: Option<AesndVoiceCallback>) -> *mut AesndPb {
    let _lock = irq::IrqLock::take();
    unsafe {
        let v = voicepbs();
        for i in 0..MAX_VOICES {
            if v[i].flags & VOICE_USED == 0 {
                let p = &mut v[i];
                *p = AesndPb::zeroed();
                p.voiceno = i as u32;
                p.flags = VOICE_USED | VOICE_STOPPED;
                p.volume_l = 0x100;
                p.volume_r = 0x100;
                p.freq_h = 0x0001;
                p.freq_l = 0x0000;
                p.cb = cb;
                return p as *mut AesndPb;
            }
        }
    }
    core::ptr::null_mut()
}

/// [`AESND_FreeVoice`].
///
/// # Safety
/// `pb` must be a pointer handed out by [`allocate_voice`] (or null).
pub unsafe fn free_voice(pb: *mut AesndPb) {
    if pb.is_null() {
        return;
    }
    let _lock = irq::IrqLock::take();
    unsafe {
        *self::pb(pb) = AesndPb::zeroed();
    }
}

#[inline(always)]
fn set_voice_format(p: &mut AesndPb, format: u32) {
    p.flags = (p.flags & !0x07) | (format & 0x07);
    p.shift = match format & 0x07 {
        VOICE_MONO8 | VOICE_STEREO8 | VOICE_MONO8_UNSIGNED | VOICE_STEREO8_UNSIGNED => 0,
        _ => 1,
    };
}

#[inline(always)]
fn set_voice_buffer(p: &mut AesndPb, buffer: *const u8, len: usize) {
    p.mram_start = buffer as u32;
    p.mram_curr = buffer as u32;
    p.mram_end = buffer as u32 + len as u32;
}

#[inline(always)]
fn set_voice_freq(p: &mut AesndPb, freq: f32) {
    let ratio = (0x10000 as f32 * (freq / DSP_DEFAULT_FREQ) + 0.5) as u32;
    p.freq_h = (ratio >> 16) as u16;
    p.freq_l = (ratio & 0xffff) as u16;
}

/// [`AESND_PlayVoice`]. `buffer`/`len` point at raw sample data in MEM1 in
/// one of the `VOICE_*` formats; `freq` is the absolute playback rate in Hz
/// (pass the sample's recorded rate for neutral pitch); `delay` is in ms;
/// `looped` wraps at the buffer end.
///
/// # Safety
/// `pb` must come from [`allocate_voice`]; `buffer` must stay resident
/// until the voice stops.
pub unsafe fn play_voice(
    pb: *mut AesndPb,
    format: u32,
    buffer: *const u8,
    len: usize,
    freq: f32,
    delay: u32,
    looped: bool,
) {
    let _lock = irq::IrqLock::take();
    unsafe {
        let p = self::pb(pb);
        set_voice_format(p, format);
        set_voice_freq(p, freq);
        set_voice_buffer(p, buffer, len);

        p.flags &= !(VOICE_RUNNING | VOICE_STOPPED | VOICE_LOOP | VOICE_ONCE);
        p.flags |= if looped { VOICE_LOOP } else { VOICE_ONCE };

        p.buf_start = 0;
        p.buf_curr = 0;
        p.buf_end = 0;
        p.stream_last = 0;
        p.delay = delay * 48;
        p.pds = 0;
        p.yn1 = 0;
        p.yn2 = 0;
        p.counter = 0;
    }
}

/// [`AESND_SetVoiceBuffer`].
///
/// # Safety
/// Same residency rules as [`play_voice`].
pub unsafe fn set_buffer(pb: *mut AesndPb, buffer: *const u8, len: usize) {
    let _lock = irq::IrqLock::take();
    unsafe { set_voice_buffer(self::pb(pb), buffer, len) }
}

/// [`AESND_SetVoiceFormat`].
pub unsafe fn set_format(pb: *mut AesndPb, format: u32) {
    let _lock = irq::IrqLock::take();
    unsafe { set_voice_format(self::pb(pb), format) }
}

/// [`AESND_SetVoiceVolume`] (per-channel, `0x0000`..=`0x0100`+; `0x100` =
/// unity). Left/right pans the voice.
pub unsafe fn set_volume(pb: *mut AesndPb, volume_l: u16, volume_r: u16) {
    let _lock = irq::IrqLock::take();
    unsafe {
        let p = self::pb(pb);
        p.volume_l = volume_l;
        p.volume_r = volume_r;
    }
}

/// [`AESND_SetVoiceFrequency`] (Hz).
pub unsafe fn set_frequency(pb: *mut AesndPb, freq: f32) {
    let _lock = irq::IrqLock::take();
    unsafe { set_voice_freq(self::pb(pb), freq) }
}

/// [`AESND_SetVoiceStream`].
pub unsafe fn set_stream(pb: *mut AesndPb, stream: bool) {
    let _lock = irq::IrqLock::take();
    unsafe {
        let p = self::pb(pb);
        if stream {
            p.flags |= VOICE_STREAM;
        } else {
            p.flags &= !VOICE_STREAM;
        }
    }
}

/// [`AESND_SetVoiceLoop`].
pub unsafe fn set_loop(pb: *mut AesndPb, looped: bool) {
    let _lock = irq::IrqLock::take();
    unsafe {
        let p = self::pb(pb);
        if looped {
            p.flags |= VOICE_LOOP;
        } else {
            p.flags &= !VOICE_LOOP;
        }
    }
}

/// [`AESND_SetVoiceMute`].
pub unsafe fn set_mute(pb: *mut AesndPb, mute: bool) {
    let _lock = irq::IrqLock::take();
    unsafe {
        let p = self::pb(pb);
        if mute {
            p.flags |= VOICE_PAUSE;
        } else {
            p.flags &= !VOICE_PAUSE;
        }
    }
}

/// [`AESND_SetVoiceStop`].
pub unsafe fn set_stop(pb: *mut AesndPb, stop: bool) {
    let _lock = irq::IrqLock::take();
    unsafe {
        let p = self::pb(pb);
        if stop {
            p.flags |= VOICE_STOPPED;
        } else {
            p.flags &= !VOICE_STOPPED;
        }
    }
}

/// [`AESND_SetVoiceDelay`] (ms before first sample is emitted).
pub unsafe fn set_delay(pb: *mut AesndPb, delay: u32) {
    let _lock = irq::IrqLock::take();
    unsafe { self::pb(pb).delay = delay * 48 }
}

/// [`AESND_RegisterVoiceCallback`]: returns the previously registered one.
pub fn register_voice_callback(
    pb: *mut AesndPb,
    cb: Option<AesndVoiceCallback>,
) -> Option<AesndVoiceCallback> {
    let _lock = irq::IrqLock::take();
    unsafe { core::mem::replace(&mut self::pb(pb).cb, cb) }
}

/// Init failure reasons.
#[derive(Copy, Clone, Debug)]
pub enum InitError {
    /// [`init`] was already called.
    InUse,
    /// ARAM exhausted while reserving voice staging buffers.
    AramFull,
    /// The DSP never answered the boot handshake.
    DspBoot,
}
