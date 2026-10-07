//! Example 04 — video-info: dump the video mode libogc selected for the
//! attached display.

#![no_std]
#![no_main]

use gc_std::{input::button, println};

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    let mode = gc.video().mode();

    println!("\nExample 04 - video mode info");
    println!();
    println!("  viTVMode       = {:#010x}", mode.viTVMode);
    println!("  fbWidth        = {}", mode.fbWidth);
    println!("  efbHeight      = {}", mode.efbHeight);
    println!("  xfbHeight      = {}", mode.xfbHeight);
    println!("  viXOrigin      = {}", mode.viXOrigin);
    println!("  viYOrigin      = {}", mode.viYOrigin);
    println!("  viWidth        = {}", mode.viWidth);
    println!("  viHeight       = {}", mode.viHeight);
    println!("  xfbMode        = {}", mode.xfbMode);
    println!("  field_rendering= {}", mode.field_rendering);
    println!("  anti-aliasing  = {}", mode.aa);
    println!("  interlaced     = {}", mode.viTVMode & 1 == 0);
    println!();
    println!("Press START to exit.");

    loop {
        gc_std::video::wait_vsync();
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
