//! Runtime glue required by `core`/`alloc`: the panic handler and the
//! allocation-failure handler.

use core::alloc::Layout;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("\n===== PANIC =====");
    crate::println!("{info}");
    loop {
        core::hint::spin_loop();
    }
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    crate::println!("\n===== OUT OF MEMORY =====");
    crate::println!("requested layout: {layout:?}");
    loop {
        core::hint::spin_loop();
    }
}
