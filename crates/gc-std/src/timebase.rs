//! Timebase access (Gekko's 40.5 MHz `TB`).

/// Read the 64-bit timebase counter.
#[inline]
pub fn now() -> u64 {
    crate::hw::mftb()
}

/// Convert ticks to microseconds.
pub const fn ticks_to_us(ticks: u64) -> u64 {
    // TB runs at bus_clock/4 = 162_000_000/4 = 40.5 MHz on the NTSC GC.
    // use the integer-approximate divider: (ticks * 40) / 405 = 40.5MHz ≈ 40.5 µs-s per tick
    (ticks * 40) / 405
}

/// Busy-wait `us` microseconds.
pub fn delay_us(us: u32) {
    let start = now();
    let wait_ticks = (us as u64 * 405) / 10; // 40.5 ticks/µs
    while now().wrapping_sub(start) < wait_ticks {
        core::hint::spin_loop();
    }
}
