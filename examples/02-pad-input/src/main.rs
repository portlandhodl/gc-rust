//! Example 02 — pad-input: buttons, held state and analog sticks/triggers.
//!
//! Shows: `buttons_down`, `buttons_held`, `input::analog` (main stick,
//! C-stick, L/R trigger gradations).

#![no_std]
#![no_main]

use gc_std::{input::{self, button}, print, println};

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 02 - pad input");
    println!("Wiggle sticks, hold buttons. START exits.");
    println!();

    loop {
        gc_std::video::wait_vsync();
        input::scan();

        let down = input::buttons_down(0);
        let held = input::buttons_held(0);
        let pad = input::analog(0);

        // A line of "pressed" events (only on the frame they go down).
        if down.contains(button::A) { println!("A pressed"); }
        if down.contains(button::B) { println!("B pressed"); }
        if down.contains(button::X) { println!("X pressed"); }
        if down.contains(button::Y) { println!("Y pressed"); }
        if down.contains(button::Z) { println!("Z pressed"); }

        // Continuous readout (carriage return keeps it on one line).
        print!(
            "\rheld:{:04x}  stick:({:4},{:4})  cstick:({:4},{:4})  L:{:3} R:{:3}   ",
            held.bits(),
            pad.stick_x,
            pad.stick_y,
            pad.substick_x,
            pad.substick_y,
            pad.trigger_l,
            pad.trigger_r,
        );

        if down.contains(button::START) {
            println!("\nReturning to loader...");
            gc_std::system::exit(0);
        }
    }
}
