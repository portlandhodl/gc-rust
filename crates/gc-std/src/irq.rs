//! Interrupt & exception handling for the GameCube (Gekko 750).
//!
//! Hardware model:
//!
//! 1. Peripherals raise PI interrupts (Processor Interface at 0xCC003000).
//! 2. Pending PI bits (masked by INTMR) raise the PowerPC **external
//!    interrupt** exception — vector offset `0x0500` in virtual space
//!    0x80000500 when the MMU is on.
//! 3. Caught by our asm trampoline, which swaps to a private IRQ stack,
//!    saves the volatile register set, and calls `__gc_irq_dispatch()`.
//!
//! User code: [`register`]/[`unregister`] a [`Source`], then
//! [`enable_source`] it and call [`enable`] once.

use crate::hw;

/// Hardware interrupt sources. Same encoding as libogc's `irq.h`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Source {
    /// PI errors (0-0)
    Error,
    /// CP (command processor) breakpoint (0-1)
    Cp,
    /// DVD drive (0-2)
    Dvd,
    /// Serial interface (0-3)
    Si,
    /// EXI channels (cascade into MEM cause) (0-4)
    Exi,
    /// Audio interface (0-5)
    Ai,
    /// DSP-to-ARAM DMA (0-6)
    DspDma,
    /// VI retrace (0-7)
    Vi,
    /// Pixel engine token (0-8)
    PeToken,
    /// Pixel engine finish (0-9)
    PeFinish,
    /// CP FIFO (0-10)
    CpFifo,
    /// Debug (0-11)
    Debug,
    /// HSP (0-12)
    Hsp,
}

impl Source {
    fn pi_bit(self) -> u32 {
        match self {
            Source::Error => 0x0001,
            Source::Cp => 0x0002,
            Source::Dvd => 0x0004,
            Source::Si => 0x0008,
            Source::Exi => 0x0010,
            Source::Ai => 0x0020,
            Source::DspDma => 0x0040,
            Source::Vi => 0x0100,
            Source::PeToken => 0x0200,
            Source::PeFinish => 0x0400,
            Source::CpFifo => 0x0800,
            Source::Debug => 0x1000,
            Source::Hsp => 0x2000,
        }
    }
    fn from_bit(bit: u32) -> Option<Source> {
        Some(match bit {
            0x0001 => Source::Error,
            0x0002 => Source::Cp,
            0x0004 => Source::Dvd,
            0x0008 => Source::Si,
            0x0010 => Source::Exi,
            0x0020 => Source::Ai,
            0x0040 => Source::DspDma,
            0x0100 => Source::Vi,
            0x0200 => Source::PeToken,
            0x0400 => Source::PeFinish,
            0x0800 => Source::CpFifo,
            0x1000 => Source::Debug,
            _ => return None,
        })
    }
}

/// ISR signature. As `extern "C"` so it's easy to hand off to a C-compiled
/// module later if desired.
pub type Handler = extern "C" fn(Source);

const SOURCES: usize = 13;
static mut HANDLERS: [Option<Handler>; SOURCES] = [None; SOURCES];

/// Register/unregister a handler for `source` (handler = None clears).
pub fn register(source: Source, handler: Option<Handler>) {
    let idx = handler_index(source);
    unsafe {
        let _lock = IrqLock::take();
        let handlers = &mut *core::ptr::addr_of_mut!(HANDLERS);
        handlers[idx] = handler;
    }
}

fn handler_index(source: Source) -> usize {
    match source {
        Source::Error => 0,
        Source::Cp => 1,
        Source::Dvd => 2,
        Source::Si => 3,
        Source::Exi => 4,
        Source::Ai => 5,
        Source::DspDma => 6,
        Source::Vi => 7,
        Source::PeToken => 8,
        Source::PeFinish => 9,
        Source::CpFifo => 10,
        Source::Debug => 11,
        Source::Hsp => 12,
    }
}

/// Enable (unmask) `source` in the PI interrupt mask register.
pub fn enable_source(source: Source) {
    unsafe {
        let mut mr = hw::read32(PI_MASK);
        mr |= source.pi_bit();
        hw::write32(PI_MASK, mr);
    }
}

/// Disable (mask) `source`.
pub fn disable_source(source: Source) {
    unsafe {
        let mut mr = hw::read32(PI_MASK);
        mr &= !source.pi_bit();
        hw::write32(PI_MASK, mr);
    }
}

/// Globally enable external interrupts (MSR_EE). Call once after all
/// handlers are registered and the sources are unmasked.
pub fn enable() {
    unsafe {
        core::arch::asm!(
            "mfmsr 3",
            "ori 3, 3, 0x8000",     // MSR_EE
            "mtmsr 3",
            "isync",
        );
    }
}

/// Globally disable external interrupts (MSR_EE off).
pub fn disable() {
    unsafe {
        core::arch::asm!(
            "mfmsr 3",
            "rlwinm 3, 3, 0, 17, 15", // clear bit 16 (EE)
            "mtmsr 3",
            "isync",
        );
    }
}

/// RAII critical section (interrupts off while the guard is alive).
pub struct IrqLock {
    was_on: bool,
}

impl IrqLock {
    /// Disable external interrupts until drop.
    pub fn take() -> IrqLock {
        let enabled = unsafe {
            let mut msr: u32;
            core::arch::asm!("mfmsr {0}", out(reg) msr);
            let on = (msr & 0x8000) != 0;
            if on {
                disable();
            }
            on
        };
        IrqLock { was_on: enabled }
    }
}

impl Drop for IrqLock {
    fn drop(&mut self) {
        if self.was_on {
            enable();
        }
    }
}

// ---------------------------------------------------------------------------
// PI register map (Processor Interface at 0xCC003000)
// ---------------------------------------------------------------------------

const PI_INTSR: u32 = 0xCC00_3000; // INT channel status (pending)
const PI_INTMR: u32 = 0xCC00_3004; // INT channel mask
const PI_MASK: u32 = PI_INTMR;

// ---------------------------------------------------------------------------
// Vector-trampoline installation + asm entry
// ---------------------------------------------------------------------------

extern "C" {
    /// asm ISR entry (defined in the global_asm block below).
    fn __gc_irq_entry();
}

/// Install the exception vectors. Called once from `init`.
///
/// On the GC, exception vectors live at physical 0x0500 (external int),
/// 0x0900 (decrementer), etc. With our identity BAT mapping the cached
/// virtual aliases are 0x80000500 etc. Each slot is 256 bytes; we write a
/// single absolute branch into our handler there.
pub(crate) fn init() {
    unsafe {
        install_vector(0x0500, __gc_irq_entry as *const () as u32);
        install_vector(
            0x0900,
            crate::lwp::__lwp_dec_entry as *const () as u32,
        );
        hw::dc_flush_range((hw::MEM_BASE_CACHED + 0x0500) as *mut u32, 0x20);
        hw::dc_flush_range((hw::MEM_BASE_CACHED + 0x0900) as *mut u32, 0x20);
    }
}

unsafe fn install_vector(offset: u32, target: u32) {
    let slot = (hw::MEM_BASE_CACHED + offset) as *mut u32;
    // absolute (AA=0) branch: writes `b <diff>` — the slot's own address is
    // the base of the branch target computation, like on classic PPC.
    let addr = slot as u32;
    let diff = target.wrapping_sub(addr);
    // `b` instruction: 0x4800_0000 | LI, with LI = diff/4 (26-bit signed)
    let li = (diff & 0x03FF_FFFC) >> 2;
    slot.write_volatile(0x4800_0000 | li);
}

core::arch::global_asm!(
    r#"
    .section .text,"ax"
    .balign 32
    .global __gc_irq_entry
__gc_irq_entry:
    // SRR0/1 hold the interrupted PC/MSR; MSR_EE already cleared by the CPU.
    // Save r1/r2 immediately and switch to the irq stack.
    mtsprg0 1
    mtsprg1 2
    lis 1, __irq_regs@ha
    addi 1, 1, __irq_regs@l
    // save volatile GPRs into the save area
    stw 0,  0*4(1)
    stw 3,  3*4(1)
    stw 4,  4*4(1)
    stw 5,  5*4(1)
    stw 6,  6*4(1)
    stw 7,  7*4(1)
    stw 8,  8*4(1)
    stw 9,  9*4(1)
    stw 10, 10*4(1)
    stw 11, 11*4(1)
    stw 12, 12*4(1)
    mfctr 0
    stw 0, 13*4(1)
    mflr 0
    stw 0, 14*4(1)
    mfcr 0
    stw 0, 15*4(1)
    mfsrr0 0
    stw 0, 16*4(1)
    mfsrr1 0
    stw 0, 17*4(1)
    // stash original r1,r2 from sprg into the save area slots
    mfsprg0 3
    stw 3, 18*4(1)
    mfsprg1 3
    stw 3, 19*4(1)
    // switch to the IRQ stack
    lis 1, __irq_stack_top@ha
    addi 1, 1, __irq_stack_top@l
    addi 1, 1, -16
    // call the dispatcher
    bl __gc_irq_dispatch
    // restore — order matters: put old r1/r2 back into SPRG first (using r3
    // as ferry — it hasn't been restored yet), then the main set.
    lis 1, __irq_regs@ha
    addi 1, 1, __irq_regs@l
    lwz 0, 16*4(1)
    mtsrr0 0
    lwz 0, 17*4(1)
    mtsrr1 0
    lwz 3, 18*4(1)
    mtsprg0 3
    lwz 3, 19*4(1)
    mtsprg1 3
    lwz 0, 13*4(1)
    mtctr 0
    lwz 0, 14*4(1)
    mtlr 0
    lwz 0, 15*4(1)
    mtcr 0
    lwz 0,  0*4(1)
    lwz 3,  3*4(1)
    lwz 4,  4*4(1)
    lwz 5,  5*4(1)
    lwz 6,  6*4(1)
    lwz 7,  7*4(1)
    lwz 8,  8*4(1)
    lwz 9,  9*4(1)
    lwz 10, 10*4(1)
    lwz 11, 11*4(1)
    lwz 12, 12*4(1)
    // final restore of r1/r2 (from SPRG)
    mfsprg1 2
    mfsprg0 1
    rfi
    "#
);

core::arch::global_asm!(
    r#"
    .section .bss,"aw",@nobits
    .balign 16
    .global __irq_regs
    .global __irq_stack_top
__irq_regs:
    .skip 96
__irq_stack_bottom:
    .skip 8192
__irq_stack_top:
    "#
);

// ---------------------------------------------------------------------------
// dispatcher
// ---------------------------------------------------------------------------

#[no_mangle]
extern "C" fn __gc_irq_dispatch() {
    let pending = unsafe { hw::read32(PI_INTSR) } & unsafe { hw::read32(PI_INTMR) };
    if pending == 0 {
        return;
    }

    // priority order: keep VI/PE/AI etc. responsive
    for bit in [
        Source::Ai.pi_bit(),
        Source::Vi.pi_bit(),
        Source::PeToken.pi_bit(),
        Source::PeFinish.pi_bit(),
        Source::CpFifo.pi_bit(),
        Source::Dvd.pi_bit(),
        Source::Si.pi_bit(),
        Source::Exi.pi_bit(),
        Source::DspDma.pi_bit(),
        Source::Hsp.pi_bit(),
        Source::Debug.pi_bit(),
        Source::Cp.pi_bit(),
        Source::Error.pi_bit(),
    ] {
        if pending & bit != 0 {
            let idx = Source::from_bit(bit)
                .map(handler_index)
                .unwrap_or(0);
            let handler = unsafe { (&*core::ptr::addr_of!(HANDLERS))[idx] };
            if let Some(h) = handler {
                h(Source::from_bit(bit).unwrap_or(Source::Error));
            } else {
                // mask the noisy source so we don't livelock
                unsafe {
                    let mut mr = hw::read32(PI_INTMR);
                    mr &= !bit;
                    hw::write32(PI_INTMR, mr);
                }
            }
            // ack the source (W1C)
            unsafe { hw::write32(PI_INTSR, bit) };
        }
    }
}
