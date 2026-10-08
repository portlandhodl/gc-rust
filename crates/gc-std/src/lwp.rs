//! LWP — preemptive multithreading for the Gekko (single core).
//!
//! The PowerPC decrementer exception (vector 0x0900) is the scheduler's
//! tick. Every TIMESLICE_US the handler re-arms DEC and may hand the CPU
//! to the next runnable thread. `yield_now()`/`sleep_ms()`/`join()` force
//! an immediate DEC trap to leave the core sooner.
//!
//! Thread state: full PPCContext (Tuxedo-shaped: PC/MSR/CR/LR/CTR/XER +
//! GPR0..31 + FPR0..31/FPSCR + GQR0..7; 0x2C0 bytes per thread).
//! Context switching happens exclusively on the DEC exception vector.

use alloc::boxed::Box;
use core::sync::atomic::Ordering;

use crate::hw;

extern "C" {
    /// The DEC trampoline (global_asm below; installed at vector 0x0900).
    pub fn __lwp_dec_entry();
}

/// Max simultaneous threads (main + spawned).
pub const MAX_THREADS: usize = 8;
/// Scheduler slice length (µs).
pub const TIMESLICE_US: u64 = 4000;
/// GC timebase: 162 MHz / 4 (bus/4 at 40.5 MHz).
pub const TB_HZ: u64 = 40_500_000;

const DEC_PERIOD_TICKS: u64 = TB_HZ / 1_000_000 * TIMESLICE_US;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum State {
    Runnable = 0,
    Sleeping = 1,
    Done = 2,
}

/// A fully-preemptive context frame.
#[repr(C, align(16))]
pub struct Frame {
    bytes: [u8; 0x2c0],
}

/// Task control block (owned by the scheduler after spawn).
#[repr(C)]
pub struct Tcb {
    frame: Frame,
    state: State,
    wake_tick: u64,
    done_ret: u32,
}

static mut TCBS: [Option<*mut Tcb>; MAX_THREADS] = [None; MAX_THREADS];
static mut CURRENT: usize = 0;
static mut COUNT: usize = 0;

// ---------------------------------------------------------------------------
// scheduler (DEC exception path; EE=0 here)
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn tcbs() -> &'static [Option<*mut Tcb>; MAX_THREADS] {
    &*core::ptr::addr_of!(TCBS)
}
#[inline(always)]
unsafe fn tcbs_mut() -> &'static mut [Option<*mut Tcb>; MAX_THREADS] {
    &mut *core::ptr::addr_of_mut!(TCBS)
}

/// Executes on the DEC exception trampoline's stack with interrupts off.
/// Returns the index of the thread to run next.
#[no_mangle]
extern "C" fn __lwp_dec_dispatch() -> usize {
    unsafe {
        let now = hw::mftb();
        for s in tcbs_mut().iter_mut() {
            if let Some(t) = *s {
                if (*t).state == State::Sleeping && (*t).wake_tick <= now {
                    (*t).state = State::Runnable;
                }
            }
        }

        // round-robin: probe the next runnable starting after CURRENT.
        let mut sel = CURRENT;
        for _ in 0..MAX_THREADS {
            sel = (sel + 1) % MAX_THREADS;
            if let Some(t) = tcbs()[sel] {
                if (*t).state == State::Runnable {
                    break;
                }
            }
        }
        if let Some(t) = tcbs()[sel] {
            if (*t).state == State::Runnable {
                (&raw mut CURRENT).write_volatile(sel);
                // repoint the save target (SPRG2) at the new current's frame
                core::arch::asm!("mtsprg2 {}", in(reg) (*t).frame.bytes.as_mut_ptr());
                return sel;
            }
        }
        // If nobody else is runnable (all sleeping), keep this thread so the
        // ticks keep coming; the sleep loop will re-trap.
        CURRENT
    }
}

/// asm bridge: save target ctx pointer for CURRENT → sprg2 side
#[no_mangle]
extern "C" fn __lwp_ctx_of(idx: usize) -> *mut u8 {
    unsafe {
        match tcbs()[idx] {
            Some(t) => (*t).frame.bytes.as_mut_ptr(),
            None => core::ptr::null_mut(),
        }
    }
}

// ---------------------------------------------------------------------------
// api
// ---------------------------------------------------------------------------

pub struct SpawnError;

impl core::fmt::Debug for SpawnError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str("SpawnError")
    }
}

#[derive(Copy, Clone)]
pub struct Thread {
    idx: usize,
}

/// True after the first spawn (the scheduler is armed afterwards).
pub fn scheduler_running() -> bool {
    unsafe { COUNT > 0 }
}

/// Prime the DEC and make the main thread schedulable.
pub fn init() {
    unsafe {
        if !INITED.swap(true, Ordering::Relaxed) {
            // main occupies slot 0 — its state at first switch is captured
            // into the (zeroed) frame at the first DEC trap.
            let main = Box::leak(Box::new(Tcb::zeroed()));
            tcbs_mut()[0] = Some(main);
            COUNT = 1; // main is runnable while scheduler up even alone
            CURRENT = 0;
            // point SPRG2 (current ctx save target) at main's frame
            core::arch::asm!("mtsprg2 {}", in(reg) (*main).frame.bytes.as_mut_ptr());
            // arm the decrementer
            core::arch::asm!("mtspr 22, {}; sync", in(reg) DEC_PERIOD_TICKS as u32);
        }
    }
}

static INITED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

impl Tcb {
    fn zeroed() -> Tcb {
        Tcb { frame: Frame { bytes: [0; 0x2c0] }, state: State::Runnable, wake_tick: 0, done_ret: 0 }
    }
}

/// Spawn `entry(arg)` on a fresh stack; preemption is unpreventable: the new
/// thread may start running as soon as it's queued.
pub fn spawn(entry: extern "C" fn(u32) -> usize, arg: u32, stack_size: usize) -> Result<Thread, SpawnError> {
    let _g = crate::irq::IrqLock::take();
    if !scheduler_running() {
        init();
    }
    unsafe {
        // pick free slot
        let mut slot = usize::MAX;
        for i in 0..MAX_THREADS {
            if tcbs()[i].is_none() {
                slot = i;
                break;
            }
        }
        if slot == usize::MAX {
            return Err(SpawnError);
        }

        let tcb = Box::leak(Box::new(Tcb::zeroed()));
        let mut stack = alloc::vec![0u8; stack_size].into_boxed_slice();
        let sp_base = stack.as_mut_ptr() as usize;
        let sp_top = (sp_base + stack_size - 16) & !0xf;
        core::mem::forget(stack); // leak with the thread (never freed)

        let f = (*tcb).frame.bytes.as_mut_ptr();
        // pc / msr / lr / gpr1 / gpr3
        (f as *mut u32).write(entry as u32);                  // +0x00
        (f.add(1) as *mut u32).write(0x0000_B032);            // +0x04 msr
        (f.add(3) as *mut u32).write(lwp_thread_exit as *const () as u32); // +0x0c lr
        (f.add(2) as *mut u32).write(0);                       // +0x08 cr = 0
        (f.add(4) as *mut u32).write(0);                       // ctr
        (f.add(5) as *mut u32).write(0);                       // xer
        let gpr = f.add(0x18) as *mut u32;
        gpr.add(1).write(sp_top as u32);                      // gpr1 = sp
        gpr.add(3).write(arg);                                 // gpr3 = arg

        (*tcb).state = State::Runnable;
        tcbs_mut()[slot] = Some(tcb);
        COUNT += 1;
        Ok(Thread { idx: slot })
    }
}

/// Yield = hand the rest of this timeslice to another thread.
#[inline(always)]
pub fn yield_now() {
    if scheduler_running() {
        #[cfg(target_arch = "powerpc")]
        unsafe {
            core::arch::asm!("mtspr 22, {}", "sync", in(reg) 0u32);
        }
    }
}

/// Sleep this thread for `ms` milliseconds.
pub fn sleep_ms(ms: u32) {
    if !scheduler_running() {
        // single-threaded callers: busy wait, honestly
        let deadline = hw::mftb() + ms as u64 * (TB_HZ / 1000);
        while hw::mftb() < deadline {}
        return;
    }
    let wake = hw::mftb() + (ms as u64 * (TB_HZ / 1000));
    unsafe {
        let cur = (&raw const CURRENT).read_volatile();
        if let Some(t) = tcbs()[cur] {
            (*t).state = State::Sleeping;
            (*t).wake_tick = wake;
            while (*t).state == State::Sleeping {
                yield_now();
            }
        }
    }
}

/// Wait for `t` to exit; returns its exit code. Never returns for slots
/// that aren't spawned threads.
pub fn join(t: Thread) -> usize {
    loop {
        unsafe {
            if let Some(tb) = tcbs()[t.idx] {
                if (*tb).state == State::Done {
                    return (*tb).done_ret as usize;
                }
            }
        }
        yield_now();
    }
}

/// Exit the calling thread Shim: `entry` returns → this runs.
extern "C" fn lwp_thread_exit() -> ! {
    unsafe {
        let cur = CURRENT;
        if let Some(tb) = tcbs()[cur] {
            (*tb).state = State::Done;
            (*tb).done_ret = 0;
        }
    }
    loop {
        yield_now();
    }
}

// ---------------------------------------------------------------------------
// DEC trampoline (vector 0x0900): saves the interruped thread's full state,
// calls the scheduler, restores the next thread's state.
// ---------------------------------------------------------------------------

core::arch::global_asm!(
    r#"
/**
 * DEC handler. Saves the interrupted thread's full PPCContext into the TCB
 * that SPRG2 currently labels (the current thread's frame address),
 * switches to the thread chosen by __lwp_dec_dispatch, then rfi's into it.
 *
 * Register use: r12 = frame ptr, r13 = scratch. SPRG0=r13, SPRG1=r12 parking.
 */
.section .text,"ax"
.balign 32
.global __lwp_dec_entry
__lwp_dec_entry:
    mtsprg0 13
    mtsprg1 12
    mfsprg2 12

    /* GPRs (r12's interrupted value via sprg1, r13's via sprg0) */
    stw 0, 24(12)
    stw 1, 28(12)
    stw 2, 32(12)
    stw 3, 36(12)
    stw 4, 40(12)
    stw 5, 44(12)
    stw 6, 48(12)
    stw 7, 52(12)
    stw 8, 56(12)
    stw 9, 60(12)
    stw 10, 64(12)
    stw 11, 68(12)
    stw 14, 80(12)
    stw 15, 84(12)
    stw 16, 88(12)
    stw 17, 92(12)
    stw 18, 96(12)
    stw 19, 100(12)
    stw 20, 104(12)
    stw 21, 108(12)
    stw 22, 112(12)
    stw 23, 116(12)
    stw 24, 120(12)
    stw 25, 124(12)
    stw 26, 128(12)
    stw 27, 132(12)
    stw 28, 136(12)
    stw 29, 140(12)
    stw 30, 144(12)
    stw 31, 148(12)
    /* r13's interrupted value */
    mfsprg0 13
    stw 13, 0x018+13*4(12)
    /* r12's interrupted value */
    mfsprg1 13
    stw 13, 0x018+12*4(12)

    /* specials */
    mfcr 13
    stw 13, 0x008(12)
    mflr 13
    stw 13, 0x00c(12)
    mfctr 13
    stw 13, 0x010(12)
    mfxer 13
    stw 13, 0x014(12)
    mfsrr0 13
    stw 13, 0x000(12)
    mfsrr1 13
    stw 13, 0x004(12)

    /* force FP on (so stfd/lfd don't fault); handler keeps EE off */
    mfmsr 13
    ori 13, 13, 0x2000
    mtmsr 13
    isync

    stfd 0, 160(12)
    stfd 1, 168(12)
    stfd 2, 176(12)
    stfd 3, 184(12)
    stfd 4, 192(12)
    stfd 5, 200(12)
    stfd 6, 208(12)
    stfd 7, 216(12)
    stfd 8, 224(12)
    stfd 9, 232(12)
    stfd 10, 240(12)
    stfd 11, 248(12)
    stfd 12, 256(12)
    stfd 13, 264(12)
    stfd 14, 272(12)
    stfd 15, 280(12)
    stfd 16, 288(12)
    stfd 17, 296(12)
    stfd 18, 304(12)
    stfd 19, 312(12)
    stfd 20, 320(12)
    stfd 21, 328(12)
    stfd 22, 336(12)
    stfd 23, 344(12)
    stfd 24, 352(12)
    stfd 25, 360(12)
    stfd 26, 368(12)
    stfd 27, 376(12)
    stfd 28, 384(12)
    stfd 29, 392(12)
    stfd 30, 400(12)
    stfd 31, 408(12)
    mffs 0
    stfd 0, 0x098(12)
    mfspr 13, 912
    stw 13, 416(12)
    mfspr 13, 913
    stw 13, 420(12)
    mfspr 13, 914
    stw 13, 424(12)
    mfspr 13, 915
    stw 13, 428(12)
    mfspr 13, 916
    stw 13, 432(12)
    mfspr 13, 917
    stw 13, 436(12)
    mfspr 13, 918
    stw 13, 440(12)
    mfspr 13, 919
    stw 13, 444(12)

    /* dec stack + dispatch */
    lis 1, __lwp_dec_stack_top@h
    addi 1, 1, __lwp_dec_stack_top@l
    addi 1, 1, -16
    li 3, 0
    bl __lwp_dec_dispatch

    /* r3 = next index */
    mr 13, 3
    /* ctx := TCBS[idx].frame  == __lwp_ctx_of(idx) inline? call it */
    mr 3, 13
    bl __lwp_ctx_of
    mr 12, 3

    /* restore FPU state (force FP on first — the restore path's msr may
       have FP off) */
    mfmsr 13
    ori 13, 13, 0x2000
    mtmsr 13
    isync
    lfd 0, 0x098(12)
    mtfsf 0xff, 0
    lfd 0, 160(12)
    lfd 1, 168(12)
    lfd 2, 176(12)
    lfd 3, 184(12)
    lfd 4, 192(12)
    lfd 5, 200(12)
    lfd 6, 208(12)
    lfd 7, 216(12)
    lfd 8, 224(12)
    lfd 9, 232(12)
    lfd 10, 240(12)
    lfd 11, 248(12)
    lfd 12, 256(12)
    lfd 13, 264(12)
    lfd 14, 272(12)
    lfd 15, 280(12)
    lfd 16, 288(12)
    lfd 17, 296(12)
    lfd 18, 304(12)
    lfd 19, 312(12)
    lfd 20, 320(12)
    lfd 21, 328(12)
    lfd 22, 336(12)
    lfd 23, 344(12)
    lfd 24, 352(12)
    lfd 25, 360(12)
    lfd 26, 368(12)
    lfd 27, 376(12)
    lfd 28, 384(12)
    lfd 29, 392(12)
    lfd 30, 400(12)
    lfd 31, 408(12)
    lwz 13, 416(12)
    mtspr 912, 13
    lwz 13, 420(12)
    mtspr 913, 13
    lwz 13, 424(12)
    mtspr 914, 13
    lwz 13, 428(12)
    mtspr 915, 13
    lwz 13, 432(12)
    mtspr 916, 13
    lwz 13, 436(12)
    mtspr 917, 13
    lwz 13, 440(12)
    mtspr 918, 13
    lwz 13, 444(12)
    mtspr 919, 13
    /* specials */
    lwz 13, 0x008(12)
    mtcr 13
    lwz 13, 0x00c(12)
    mtlr 13
    lwz 13, 0x010(12)
    mtctr 13
    lwz 13, 0x014(12)
    mtxer 13
    lwz 13, 0x000(12)
    mtsrr0 13
    lwz 13, 0x004(12)
    mtsrr1 13
    /* GPRs (r12 last!) */
    lwz 0, 24(12)
    lwz 1, 28(12)
    lwz 2, 32(12)
    lwz 3, 36(12)
    lwz 4, 40(12)
    lwz 5, 44(12)
    lwz 6, 48(12)
    lwz 7, 52(12)
    lwz 8, 56(12)
    lwz 9, 60(12)
    lwz 10, 64(12)
    lwz 11, 68(12)
    lwz 14, 80(12)
    lwz 15, 84(12)
    lwz 16, 88(12)
    lwz 17, 92(12)
    lwz 18, 96(12)
    lwz 19, 100(12)
    lwz 20, 104(12)
    lwz 21, 108(12)
    lwz 22, 112(12)
    lwz 23, 116(12)
    lwz 24, 120(12)
    lwz 25, 124(12)
    lwz 26, 128(12)
    lwz 27, 132(12)
    lwz 28, 136(12)
    lwz 29, 140(12)
    lwz 30, 144(12)
    lwz 31, 148(12)
    lwz 13, 0x018+13*4(12)   /* the interrupted r13 */
    lwz 12, 0x018+12*4(12)   /* the interrupted r12, last */
    rfi

.section .bss,"aw",@nobits
.balign 16
.global __lwp_dec_stack
__lwp_dec_stack:
.skip 8192
.global __lwp_dec_stack_top
__lwp_dec_stack_top:
    "#
);
