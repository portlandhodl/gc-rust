//! Observability mailbox — a fixed-address region of MEM1 that examples and
//! tests can poke results into. The GameCube leaves `0x80001C00..0x80002FFF`
//! unused (loader/exception area); we reserve 128 tag slots there:
//!
//! ```text
//!   0x80001C00 + tag*4 = value
//! ```
//!
//! Dolphin's MemoryWatcher (watching those addresses over a UNIX socket)
//! turns this into an observability channel for tests; on hardware the same
//! writes can be recovered via a debugger/UART trace.
//!
//! Probes: `set(tag, value)` writes (noisy); `observe("boot")`… no — do the
//! simple thing.

/// Mailbox base (loader scratch area, unused by gc-std).
pub const OBSERVE_BASE: u32 = 0x8000_1C00;

/// Header tag written when an example starts: personal sentinel so the host
/// watcher can tell "emulation started" from garbage.
pub const TAG_HEARTBEAT: u32 = 0;

/// Store a 32-bit result value into mailbox slot `tag` (0..128).
#[inline(always)]
pub fn set(tag: u32, value: u32) {
    if tag < 128 {
        unsafe {
            ((OBSERVE_BASE + tag * 4) as *mut u32).write_volatile(value);
        }
    }
}

/// Read back a slot (for on-console checks).
pub fn get(tag: u32) -> u32 {
    unsafe { ((OBSERVE_BASE + tag * 4) as *const u32).read_volatile() }
}

/// Host-side test helper: heartbeat at startup so tests know it's us.
pub fn heartbeat(code: u32) {
    set(TAG_HEARTBEAT, 0xBEEF_0000 | (code & 0xFFFF));
}
