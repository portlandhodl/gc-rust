//! Example 19 — dvd-read: bootable ISO + reading a file from the disc.
//!
//! `make dvd-read-iso` packs this example with the Rust apploader into a
//! GCM image. Booting that ISO goes through the hardware boot path
//! (IPL-less BS2 emulation → our apploader → gc-std `main()`); the example
//! then issues a raw DI read of `/README.TXT`, whose bytes sit at a fixed
//! disc offset (`README_LBA`) baked by the packer.
//!
//! Press START to exit.

#![no_std]
#![no_main]

use gc_std::{
    audio, dvd, observe,
    input::{self, button},
    println,
};

/// Disc offset the packer placed readme.txt at (2048-aligned).
const README_LBA: u64 = 0x10_0000;

// 256-entry sine, 440 Hz*6 writing is the "disc works" tone.
static SINE: [i16; 256] = build_sine();

const fn build_sine() -> [i16; 256] {
    let mut t = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (sine(-core::f32::consts::PI + (i as f32) * (core::f32::consts::PI * 2.0 / 256.0)) * 14000.0) as i16;
        i += 1;
    }
    t
}

const fn sine(mut x: f32) -> f32 {
    while x > core::f32::consts::PI { x -= core::f32::consts::PI * 2.0; }
    while x < -core::f32::consts::PI { x += core::f32::consts::PI * 2.0; }
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0))))
}

#[allow(improper_ctypes_definitions)]
extern "C" fn success_tone(buf: &mut [i16]) {
    static PHASE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    // 2500 Hz at 48 kHz
    const STEP: u32 = (2500u64 * 65536 * 256 / 48000) as u32;
    for frame in 0..(buf.len() / 2) {
        let p = PHASE.load(core::sync::atomic::Ordering::Relaxed);
        let v = SINE[((p >> 8) & 0xff) as usize];
        buf[frame * 2] = v;
        buf[frame * 2 + 1] = v;
        PHASE.store(p.wrapping_add(STEP), core::sync::atomic::Ordering::Relaxed);
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    observe::heartbeat(19);
    println!("\nExample 19 - dvd-read (DI drive)");
    println!("booted via custom bootstrap; now talking to the DI directly.");

    // note: at this point the drive was acting as the boot medium; direct
    // reads may hit DVD_ERROR_IO with no disc — that's the expected null
    // path when running the .dol directly.

    #[repr(align(32))]
    struct Aligned([u8; 32]);
    let mut id = Aligned([0u8; 32]);
    let r = dvd::read_disk_id(&mut id.0);
    observe::set(1, r as u32); // 0 = disc id OK
    match r {
        0 => println!(
            "disc id: {}{}{}{} v{}",
            id.0[0] as char,
            id.0[1] as char,
            id.0[2] as char,
            id.0[3] as char,
            id.0[7]
        ),
        e => println!("read_disk_id: {e} (no drive/disc?)"),
    }

    // fetch the bundled text file from the disc
    #[repr(align(32))]
    struct Buf([u8; 2048]);
    let mut b = Buf([0u8; 2048]);
    let r = dvd::read_abs(&mut b.0, README_LBA, 2048);
    observe::set(2, r as u32); // 0 = disc read OK
    match r {
        0 => {
            println!("--- /README.TXT @lba {README_LBA:#x} ---");
            for line in b.0[..1024].split(|&c| c == b'\n') {
                if line.is_empty() || line.iter().all(|&c| c == b'\r' || c < b' ') {
                    break;
                }
                if let Ok(l) = core::str::from_utf8(line) {
                    println!("{l}");
                }
            }
            // magic cookie that only lands if the SSE payload roundtripped
            let got = &b.0[..23];
            let ok = got == b"Hello from the disc! gc".as_slice();
            observe::set(3, if ok { 0xDA7A_600D } else { 0xBAAD_F00D });
            println!("--- eof (disc read OK) ---");
            // Tell the outside world: a 2.5 kHz tone for ~2 s (audio pipeline
            // proves the disc read worked — used by tests/dvd-iso-e2e.sh).
            let a = audio::init();
            audio::on_refill(&a, success_tone);
            for _ in 0..120 {
                gc_std::video::wait_vsync();
            }
        }
        e => println!("dvd read err {e} (expected when booted as .dol)"),
    }

    println!("\nPress START to exit.");
    loop {
        gc_std::video::wait_vsync();
        input::scan();
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
