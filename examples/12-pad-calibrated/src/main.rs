//! Example 12 — pad-calibrated: origin correction + hot plug detection.
//!
//! At startup (and when a pad is freshly attached), gc-std issues SI command
//! `0x41` to grab the pad's stored origin used for dead-zone/stick-center
//! normalization. Unplugging/replugging now clears hot state correctly.

#![no_std]
#![no_main]

use gc_std::{input::{self, button}, println, print};

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 12 — Pad calibration");
    println!("Plug/unplug a pad; sticks should centre at 0\n");

    loop {
        gc_std::video::wait_vsync();
        input::scan();

        for p in 0..4 {
            if input::connected(p) {
                let a = input::analog(p);
                print!(
                    "\rpad{}: stick=({:4},{:4}) sub=({:4},{:4}) L={:3} R={:3}   ",
                    p, a.stick_x, a.stick_y, a.substick_x, a.substick_y, a.trigger_l, a.trigger_r
                );
            } else {
                print!("\rpad{}: (not connected)                       \r", p);
            }
        }

        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
