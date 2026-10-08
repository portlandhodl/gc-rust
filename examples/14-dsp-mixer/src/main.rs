//! Example 14 — dsp-mixer: polyphonic audio through the DSP mixer microcode
//! (AESND port, libogc-shaped API).
//!
//! Three looped voices form a chord that changes pitch every second, while a
//! fourth voice plays one-shot "ping" accents on top. All mixing happens on
//! the GameCube's DSP — the CPU only updates voice parameter blocks.
//!
//! On Dolphin you need DSP LLE for actual sound (DSP HLE emulates the boot
//! handshake but not the libaesnd mixer ucode); the demo stays stable either
//! way. Press START to exit.

#![no_std]
#![no_main]

use gc_std::{
    aesnd,
    input::{self, button},
    println,
};

// -------------------- instrument sample -----------------------------
// A sine wave at 256 Hz / 32 kHz mono s16, 10 cycles (1250 samples) —
// 125 samples per cycle keeps the loop click-free at the base pitch.
const BASE_HZ: f32 = 256.0;
const SAMPLE_RATE: f32 = 32_000.0;
const SAMPLES: usize = 1_250;

static INSTRUMENT: [i16; SAMPLES] = build_instrument();

const fn build_instrument() -> [i16; SAMPLES] {
    let mut t = [0i16; SAMPLES];
    let mut i = 0;
    while i < SAMPLES {
        t[i] = (sine((i as f32 % 125.0) * (core::f32::consts::PI * 2.0 / 125.0)) * 24000.0) as i16;
        i += 1;
    }
    t
}

const fn sine(mut x: f32) -> f32 {
    while x > core::f32::consts::PI {
        x -= core::f32::consts::PI * 2.0;
    }
    while x < -core::f32::consts::PI {
        x += core::f32::consts::PI * 2.0;
    }
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0))))
}

fn sample_bytes(t: &'static [i16]) -> &'static [u8] {
    // SAFETY: reinterpret i16 slice as bytes of the same extent. AESND 16-bit
    // formats are big-endian — which is exactly how this big-endian target
    // stores i16 in memory.
    unsafe { core::slice::from_raw_parts(t.as_ptr() as *const u8, t.len() * 2) }
}

// -------------------- the "song" ------------------------------------
// Chord progression: I - vi - IV - V (root freqs, Hz).
const CHORDS: [[f32; 3]; 4] = [
    [261.63, 329.63, 392.00], // C E G
    [220.00, 261.63, 329.63], // A C E
    [174.61, 220.00, 261.63], // F A C
    [196.00, 246.94, 293.66], // G B D
];
const CHORD_NAMES: [&str; 4] = ["C-E-G", "A-C-E", "F-A-C", "G-B-D"];
const ACCENT_HZ: f32 = 1046.5; // C6 ping

const FRAMES_PER_CHORD: u32 = 60; // 1 s at 60 Hz VI

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 14 - dsp-mixer (AESND polyphony)");
    println!("3 looped chord voices + 1 one-shot accent via DSP mixing.");

    if let Err(e) = aesnd::init() {
        println!("aesnd::init failed: {:?} (audio stays silent)", e);
        loop {
            gc_std::video::wait_vsync();
            input::scan();
            if input::buttons_down(0).contains(button::START) {
                gc_std::system::exit(0);
            }
        }
    }

    unsafe {
        let v0 = aesnd::allocate_voice(None);
        let v1 = aesnd::allocate_voice(None);
        let v2 = aesnd::allocate_voice(None);
        let accent = aesnd::allocate_voice(None);
        let chord_voices = [v0, v1, v2];
        assert!(!v0.is_null() && !v1.is_null() && !v2.is_null() && !accent.is_null());

        // accents: one-shot, thinner volume
        aesnd::set_volume(accent, 0x80, 0x80);

        let data = sample_bytes(&INSTRUMENT);
        let mut chord = 0usize;
        let mut frame = 0u32;

        println!("Voices up. Playing chord loop. START exits.");

        // start the first chord + kick the accent once
        for (i, voice) in chord_voices.iter().enumerate() {
            let hz = CHORDS[chord][i];
            aesnd::play_voice(*voice, aesnd::VOICE_MONO16, data.as_ptr(), data.len(), play_rate(hz), 0, true);
        }
        aesnd::play_voice(accent, aesnd::VOICE_MONO16, data.as_ptr(), data.len(), play_rate(ACCENT_HZ), 0, false);

        loop {
            gc_std::video::wait_vsync();
            input::scan();
            if input::buttons_down(0).contains(button::START) {
                gc_std::system::exit(0);
            }

            frame += 1;
            if frame % FRAMES_PER_CHORD == 0 {
                chord = (chord + 1) % CHORDS.len();
                for (i, voice) in chord_voices.iter().enumerate() {
                    // retune the looped voices
                    aesnd::set_frequency(*voice, play_rate(CHORDS[chord][i]));
                }
                println!("chord: {}", CHORD_NAMES[chord]);
            }
            if frame % (FRAMES_PER_CHORD * 2) == 0 {
                // re-trigger the accent ping every other chord
                aesnd::play_voice(accent, aesnd::VOICE_MONO16, data.as_ptr(), data.len(), play_rate(ACCENT_HZ), 0, false);
            }
        }
    }
}

/// Playback rate to pitch the 256 Hz instrument to `note_hz`.
fn play_rate(note_hz: f32) -> f32 {
    SAMPLE_RATE * (note_hz / BASE_HZ)
}
