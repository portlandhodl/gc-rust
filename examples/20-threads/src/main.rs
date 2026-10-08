//! Example 20 — threads: preemptive LWP.
//!
//! The classic shape: a background "file agent" thread — it sleeps on
//! `lwp::sleep_ms`, wakes, processes a tick of work, sleeps again — while
//! the main loop is a fullscreen animation that never waits for it. Both
//! run on the same Gekko core; the decrementer tick slices them.
//!
//! Watch the top line (agent tick counter) — it advances while the
//! foreground keeps animating. B START to exit.

#![no_std]
#![no_main]

use gc_std::{
    input::{self, button},
    lwp, println,
};

const SPIN: &[u8] = b"|/-\\";

// -------------------- background agent --------------------------------
//
// Pretend this thread owns a slow peripheral: sleep, tick some state,
// sleep again. Between ticks it owns *nothing* — any work sleeps politely.
extern "C" fn agent(_arg: u32) -> usize {
    let mut tick = 0u32;
    loop {
        // simulate an i/o-bound background task; real builds would call
        // usbgecko/host interfaces here.
        println!("[agent] tick {tick} (delivered data: {} b)", tick * 1234 % 8192);
        lwp::sleep_ms(400);
        tick = tick.wrapping_add(1);
    }
}

// -------------------- foreground "game" --------------------------------
#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 20 - threads");
    println!("foreground loop animating while agent sleeps/works. START exits.");

    let _agent = lwp::spawn(agent, 0, 16 * 1024).expect("spawn agent");
    println!("agent thread started.");

    let mut frame = 0u32;
    loop {
        gc_std::video::wait_vsync();
        input::scan();
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        // cheap visual proof that the main loop never blocks
        frame += 1;
        let x = (frame * 3) % 460;
        let c = SPIN[(frame as usize >> 3) & 3] as char;
        if frame % 15 == 0 {
            println!("fg {c} x={x}");
        }
    }
}
