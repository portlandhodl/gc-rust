//! Runtime glue for a bare-metal no-OS target.

use core::alloc::Layout;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // try to get it on screen if the console is alive
    crate::println!("\n===== PANIC =====\n{info}");
    loop {
        unsafe { crate::hw::isync() };
    }
}

#[alloc_error_handler]
fn oom(layout: Layout) -> ! {
    crate::println!("\n===== OUT OF MEMORY =====\nlayout: {layout:?}");
    loop {
        unsafe { crate::hw::isync() };
    }
}
