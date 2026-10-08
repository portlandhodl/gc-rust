//! Example 15 — exi-sram: talk to the EXI bus and read the system SRAM.
//!
//! EXI (the "external interface" SPI-like bus) is where the console keeps
//! its 64-byte settings SRAM: video mode, language, sound mode, RTC bias,
//! memory-card flash IDs. This example proves the EXI driver end to end
//! (device-ID queries on all channels + the SRAM DMA read path) and prints
//! what it finds. Press START to exit.

#![no_std]
#![no_main]

use gc_std::{
    exi,
    input::{self, button},
    print, println, sram,
};

const LANGS: [&str; 6] = ["english", "german", "french", "spanish", "italian", "dutch"];
const VIDEO_MODES: [&str; 3] = ["NTSC", "PAL", "MPAL"];

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 15 - exi-sram");

    // ---- EXI presence + device IDs ------------------------------------
    for chn in 0..3u32 {
        let name = ["ch0 (memcard slot A)", "ch1 (memcard slot B)", "ch2 (BBA/SP1)"][chn as usize];
        let present = exi::probe(chn);
        println!("{name}: ext={} ", if present { "y" } else { "n" });
        if let Some(id) = exi::get_id(chn, exi::EXI_DEVICE_0) {
            println!("    dev0 id = {id:#010x}");
        } else {
            println!("    dev0 id = <no answer>");
        }
    }

    // ---- SRAM main block ----------------------------------------------
    match sram::read_raw() {
        None => println!("SRAM: read failed or bad checksum"),
        Some(raw) => {
            let vm = raw[19] & sram::SRAM_VIDEO_MODE_BITS;
            let lang = raw[18];
            println!(
                "video mode : {} {}",
                VIDEO_MODES.get(vm as usize).unwrap_or(&"?"),
                if raw[19] & sram::SRAM_PROGSCAN_BIT != 0 { "+ progscan" } else { "" }
            );
            println!(
                "sound mode : {}",
                if raw[19] & sram::SRAM_SOUND_MODE_BIT != 0 { "stereo" } else { "mono" }
            );
            println!("language   : {}", LANGS.get(lang as usize).unwrap_or(&"?"));
            println!("rtc bias   : {}", u32::from_be_bytes([raw[12], raw[13], raw[14], raw[15]]));
            println!("display_off: {} ntd = {:#04x}", raw[16] as i8, raw[17]);
            println!("checksum   : {:#06x} (verified ok)", u16::from_be_bytes([raw[0], raw[1]]));
        }
    }

    // ---- SRAM extended block ------------------------------------------
    match sram::read_sram_ex() {
        None => println!("SRAM-EX: unavailable"),
        Some(ex) => {
            print!("flash id0: ");
            for b in ex.flash_id[0] { print!("{b:02x}"); }
            println!();
            print!("flash id1: ");
            for b in ex.flash_id[1] { print!("{b:02x}"); }
            println!();
            println!("dvd error  : {:#04x}", ex.dvderr_code);
            println!("kbd id     : {:#010x}", ex.wireless_kbd_id);
            for (i, id) in ex.wireless_pad_id.iter().enumerate() {
                println!("pad id[{i}]  : {id:#06x}");
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
