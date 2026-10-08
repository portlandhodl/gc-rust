//! Example 11 — irq-timer: the PI interrupt system.
//!
//! Instead of busy-polling `wait_vsync`, we register a handler for the
//! VI-retrace interrupt. The ISR increments a counter; the main loop just
//! reads it. This is the pattern all async drivers (audio, DVD, EXI)
//! build on top of.
//!
//! Prints per-frame tick deltas so you can see the retrace cadence.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};
use gc_std::{input::{self, button}, irq, println};

static TICKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_vi_retrace(_: irq::Source) {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 11 — IRQ retrace + PI interrupts");
    println!("----------------------------------------------");
    println!("frame = nticks in VI retrap completion order.");
    println!("");

    // Route and enable the VI retrace interrupt, then enable external ints.
    irq::register(irq::Source::Vi, Some(on_vi_retrace));
    irq::enable_source(irq::Source::Vi);
    irq::enable();

    println!("irqs on; running. every 60 ticks we print. START exits.");

    let mut last_seen = 0u32;
    let mut frame = 0u32;

    loop {
        let now = TICKS.load(Ordering::Relaxed);
        if now != last_seen {
            frame += 1;
            last_seen = now;
            if frame % 60 == 0 {
                println!("vi frame {} (irq ticks seen: {})", frame, now);
            }
        }

        input::scan();
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
