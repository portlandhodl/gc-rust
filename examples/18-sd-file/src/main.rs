//! Example 18 — sd-file: mount a FAT16/FAT32 SD card behind an SD Gecko
//! adapter (EXI slot A) and dump the first text-ish file found.
//!
//! Note: needs an SD Gecko *on hardware* (Dolphin doesn't emulate it). The
//! SD command/state machine and the FAT parser are host-tested against an
//! emulated SD card in tools/gc-host-tests.

#![no_std]
#![no_main]

extern crate alloc;

use gc_std::{
    fat,
    input::{self, button},
    println, sd,
};

/// glue: FAT's BlockIo over the SD card (&mut Sd)
struct SdBlockIo<'a> {
    card: &'a mut sd::Sd,
}

impl fat::BlockIo for SdBlockIo<'_> {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> i32 {
        sd::read_block(self.card, lba, buf)
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 18 - sd-file (SD Gecko in slot A)");

    let mut card = match sd::connect(0) {
        Ok(c) => c,
        Err(e) => {
            println!("sd connect failed: {e} (is an SD Gecko present?)");
            exit_loop();
        }
    };
    println!("SD card up: {} blocks", sd::blocks(&card));

    let mut io = SdBlockIo { card: &mut card };
    let mut fs = match fat::Fat::mount(&mut io) {
        Ok(f) => f,
        Err(e) => {
            println!("fat mount failed: {e}");
            exit_loop();
        }
    };
    println!("FAT mounted. Root directory:");
    match fs.list() {
        Err(e) => {
            println!("list failed: {e}");
            exit_loop();
        }
        Ok(entries) => {
            for e in &entries {
                let nm = e.name_str();
                let end = nm.iter().position(|&c| c == 0).unwrap_or(12);
                let nm = core::str::from_utf8(&nm[..end]).unwrap_or("<?>");
                let kb = (e.size + 1023) / 1024;
                let kind = if e.is_dir { "dir " } else { "file" };
                println!("  {:12} {:>6} KiB {}", nm, kb, kind);
            }
            if entries.is_empty() {
                println!("  (empty)");
            }

            // dump the first small text-ish file
            for e in &entries {
                if e.is_dir || e.size > 8 * 1024 {
                    continue;
                }
                let nm = e.name_str();
                let end = nm.iter().position(|&c| c == 0).unwrap_or(12);
                let name = core::str::from_utf8(&nm[..end]).unwrap_or("<?>");
                let mut buf = alloc::vec![0u8; e.size as usize];
                match fs.read_file(name, &mut buf) {
                    Ok(n) => {
                        println!("\n--- {name} ({n} bytes) ---");
                        for line in buf[..n as usize].split(|&c| c == b'\n') {
                            if let Ok(l) = core::str::from_utf8(line) {
                                println!("{l}");
                            }
                        }
                        println!("--- eof ---");
                    }
                    Err(e) => println!("read {name} failed: {e}"),
                }
                break;
            }
        }
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

fn exit_loop() -> ! {
    println!("Press START to exit.");
    loop {
        gc_std::video::wait_vsync();
        input::scan();
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
