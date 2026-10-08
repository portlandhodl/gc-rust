//! Example 16 — memcard: create, write, read and list a save file on a
//! GameCube memory card in slot A (libogc `CARD_*`-shaped API).
//!
//! The demo creates `gcrust-save.sav` (one sector), stamps a counter into
//! its first bytes, and re-reads/verifies it. Run it twice: the counter
//! increments — proof the data survived on the card (or in Dolphin's
//! `MemoryCardA.USA.raw` image). Press START to exit.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use gc_std::{
    card,
    input::{self, button},
    println,
};

const FILENAME: &str = "gcrust-save.sav";
const MAGIC: &[u8; 16] = b"GC-RUST-SAVE v01";

fn card_error_str(e: i32) -> &'static str {
    match e {
        0 => "ready",
        -1 => "busy",
        -2 => "wrong device",
        -3 => "no card",
        -4 => "no file",
        -5 => "io error",
        -6 => "broken filesystem",
        -7 => "exists",
        -8 => "no free dir entry",
        -9 => "not enough space",
        -11 => "limit",
        _ => "fatal",
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 16 - memcard (slot A)");

    card::init(None, None);

    let chn = card::CARD_SLOTA;
    let r = card::mount(chn);
    if r != card::CARD_ERROR_READY {
        println!("mount failed: {} ({})", card_error_str(r), r);
        exit_loop();
    }
    let ss = card::sector_size(chn).unwrap_or(8192) as usize;
    let free = card::free_blocks(chn).unwrap_or(0);
    println!("mounted ok: sector={ss} bytes, free blocks={free}");

    // ---- directory listing -------------------------------------------
    println!("-- files on card --");
    let mut shown = 0u32;
    if let Ok(mut dir) = card::find_first(chn, true) {
        loop {
            let name_len = dir.filename.iter().position(|&c| c == 0).unwrap_or(32);
            let name = core::str::from_utf8(&dir.filename[..name_len]).unwrap_or("<?>");
            println!("  [{:2}] {:32} {:>6} bytes  perm={:#04x}", dir.fileno, name, dir.filelen, dir.permissions);
            shown += 1;
            if card::find_next(&mut dir) != card::CARD_ERROR_READY {
                break;
            }
        }
    }
    if shown == 0 {
        println!("  (empty)");
    }

    // ---- open or create ----------------------------------------------
    let mut file = match card::open(chn, FILENAME) {
        Ok(f) => {
            println!("existing save found (len {} B)", f.len);
            f
        }
        Err(card::CARD_ERROR_NOFILE) => {
            println!("no save yet; creating {FILENAME} (1 sector)");
            match card::create(chn, FILENAME, ss as u32) {
                Ok(mut f) => {
                    let mut blank = vec![0u8; ss];
                    // content is written below; start from a known-clean page
                    blank[0..16].copy_from_slice(MAGIC);
                    let r = card::write(&mut f, &blank, 0);
                    if r != card::CARD_ERROR_READY {
                        println!("initial write failed: {}", card_error_str(r));
                        exit_loop();
                    }
                    f
                }
                Err(e) => {
                    println!("create failed: {}", card_error_str(e));
                    exit_loop();
                }
            }
        }
        Err(e) => {
            println!("open failed: {}", card_error_str(e));
            exit_loop();
        }
    };

    // ---- read back, bump counter, write again ------------------------
    let mut buf = vec![0u8; ss];
    let r = card::read(&mut file, &mut buf, 0);
    if r != card::CARD_ERROR_READY {
        println!("read failed: {}", card_error_str(r));
        exit_loop();
    }
    if buf[0..16] == MAGIC[..] {
        let n = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
        println!("save verified; this console has booted it {n} time(s)");
        buf[16..20].copy_from_slice(&(n + 1).to_be_bytes());
        let r = card::write(&mut file, &buf, 0);
        println!("counter bumped (write -> {})", card_error_str(r));
        let r2 = card::read(&mut file, &mut buf, 0);
        let ok = r2 == card::CARD_ERROR_READY
            && u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]) == n + 1;
        println!("re-read verify: {}", if ok { "OK" } else { "MISMATCH!" });
    } else {
        println!("save magic mismatch — card contents unexpected");
    }

    println!("free blocks now: {}", card::free_blocks(chn).unwrap_or(0));
    println!("\nPress START to exit.");
    loop {
        gc_std::video::wait_vsync();
        input::scan();
        if input::buttons_down(0).contains(button::START) {
            card::unmount(chn);
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
