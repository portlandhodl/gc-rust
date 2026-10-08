//! Rust apploader: the three-callback protocol of the GameCube ISO boot
//! process (`iAppLoaderEntry`/`Init`/`Main`/`Close`), implemented for
//! Dolphin's emulated BS2 (`CBoot::RunApploader` runs exactly these).
//!
//! `Main` returns (ram len, disc offset) pairs for the DOL's sections; the
//! table is baked in by `tools/gc-iso` at pack time (it patches the magic
//! marker region).

#![no_std]
#![no_main]

/// Magic sentinel the gc-iso packer searches for and rewrites in-place to
/// the real section table. Layout after the magic:
/// `[count][off0,len0,addr0][off1,len1,addr1]...` (u32 each, big-endian).
#[no_mangle]
pub static mut DOL_SECTION_TABLE: [u32; 96] = {
    let mut t = [0u32; 96];
    t[0] = 0xAC1D_5000;
    t[1] = 0x5EEC_A11E;
    t[2] = 0x0000_0000; // count (patched)
    t
};

/// `_start`: the linked entry symbol (lld ENTRY default); tail-calls the
/// three-callback entry. Never returns: Dolphin's RunFunction rewinds on
/// the magic return-address it plants.
#[no_mangle]
pub extern "C" fn _start(init: *mut *const (), main: *mut *const (), close: *mut *const ()) -> ! {
    apploader_entry(init, main, close);
    loop {}
}

/// Return address Dolphin leaves for RunFunction callees (write the fn ptrs).
#[no_mangle]
pub extern "C" fn apploader_entry(
    init_out: *mut *const (),
    main_out: *mut *const (),
    close_out: *mut *const (),
) {
    unsafe {
        // fresh start each boot
        (&raw mut DONE).write_volatile(0);
        init_out.write_volatile(apploader_init as *const ());
        main_out.write_volatile(apploader_main as *const ());
        close_out.write_volatile(apploader_close as *const ());
    }
}

/// Init receives the osreport HLE hook address in r3 — unused.
#[no_mangle]
pub extern "C" fn apploader_init(_osreport: usize) {}

/// "main loop": one call per DOL section to load.
#[no_mangle]
pub extern "C" fn apploader_main() -> u32 {
    unsafe {
        let done_read = (&raw const DONE).read_volatile();
        let table = (&raw const DOL_SECTION_TABLE) as *const u32;
        let count = table.add(2).read_volatile() as usize;
        if done_read >= count {
            return 0;
        }
        let off = table.add(3 + done_read * 3 + 0).read_volatile();
        let len = table.add(3 + done_read * 3 + 1).read_volatile();
        let addr = table.add(3 + done_read * 3 + 2).read_volatile();
        // Dolphin copies [off, off+len) of the disc to `addr` in MEM1.
        // (GC: raw byte offset; no shift — the <<2 applies to Wii only.)
        (0x8130_0004 as *mut u32).write_volatile(addr);
        (0x8130_0008 as *mut u32).write_volatile(len);
        (0x8130_000C as *mut u32).write_volatile(off);
        (&raw mut DONE).write_volatile(done_read + 1);
    }
    1
}

/// Close returns the entry point (DOL's `_start`).
#[no_mangle]
pub extern "C" fn apploader_close() -> u32 {
    unsafe { (&raw const DOL_ENTRY).read_volatile() }
}

/// Set by gc-iso: final entry address (DOL header entrypoint).
#[no_mangle]
pub static mut DOL_ENTRY: u32 = 0x8000_3100;

static mut DONE: usize = 0;

// no unwinding, no allocation
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
