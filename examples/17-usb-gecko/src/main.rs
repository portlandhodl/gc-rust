//! Example 17 — usb-gecko: host-visible debug output through a USB Gecko
//! adapter (or Dolphin's "Gecko" EXI device, which is a TCP server on port
//! 0xD6EC = 55020).
//!
//! The demo detects a Gecko in memcard slot B, prints a banner over that
//! debug channel once, then streams a heartbeat line every second. On
//! Dolphin, connect with `nc localhost 55020` (or see
//! tests/usbgecko-e2e.sh). Press START to exit.

#![no_std]
#![no_main]

use gc_std::{
    exi, input::{self, button}, println, usbgecko,
};

const CHN: u32 = exi::EXI_CHANNEL_1; // slot B

fn gecko_print(s: &str) {
    if usbgecko::is_gecko_alive(CHN) {
        usbgecko::send_buffer(CHN, s.as_bytes());
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 17 - usb-gecko");

    if usbgecko::is_gecko_alive(CHN) {
        println!("gecko detected on slot B; streaming to it");
        gecko_print("GC-RUST GECKO OK\r\n");
    } else {
        println!("no gecko on slot B (Dolphin: set SlotB=Gecko)");
    }

    let mut frame = 0u32;
    loop {
        gc_std::video::wait_vsync();
        input::scan();
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
        frame += 1;
        if frame % 60 == 0 {
            gecko_print("heartbeat ");
        }
    }
}
