//! Example 13 — audio-beep: a synthesized sine tone via the AI DMA.
//!
//! We ship a 256-entry sine LUT into main memory, then advance a phase
//! accumulator each output frame. The AI DMA interrupt refills the
//! just-consumed buffer.
//!
//! Pitch is fixed at A4 (440 Hz). Press START to exit.

#![no_std]
#![no_main]

extern crate alloc;

use gc_std::{audio, input::{self, button}, println};

const NOTE_HZ: f32 = 440.0;
const OUT_RATE: f32 = 48_000.0;

// -------------------- static sine table ----------------------------
// Written by `const fn` so it lives in .rodata; evaluated at compile time.
static SINE_TABLE: [i16; 256] = build_sine_tab();

const fn build_sine_tab() -> [i16; 256] {
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (sine(-core::f32::consts::PI + (i as f32) * (core::f32::consts::PI * 2.0 / 256.0)) * 16000.0) as i16;
        i += 1;
    }
    t
}

const fn sine(mut x: f32) -> f32 {
    // fold to [-pi, pi]
    while x > core::f32::consts::PI { x -= core::f32::consts::PI * 2.0 }
    while x < -core::f32::consts::PI { x += core::f32::consts::PI * 2.0 }
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0))))
}

// -------------------- main -------------------------------------------
#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 13 — audio-beep (AI DMA playback)");
    println!("440 Hz sine on both channels.");
    println!("Press START to exit.");

    let a = audio::init();
    audio::on_refill(&a, refill);

    loop {
        gc_std::video::wait_vsync();
        input::scan();
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}

// The audio callback — runs on the AI DMA interrupt. Keep it quick.
extern "C" fn refill(buf: &mut [i16]) {
    // phase in Q16.16 within the 256-entry LUT
    static PHASE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let step: u32 = ((NOTE_HZ / OUT_RATE) * 65536.0 * 256.0) as u32;
    for frame in 0..(buf.len() / 2) {
        let p = PHASE.load(core::sync::atomic::Ordering::Relaxed);
        let idx = (p >> 8) as usize & 0xff;
        let v = SINE_TABLE[idx];
        buf[frame * 2] = v;
        buf[frame * 2 + 1] = v;
        PHASE.store(p.wrapping_add(step), core::sync::atomic::Ordering::Relaxed);
    }
}
