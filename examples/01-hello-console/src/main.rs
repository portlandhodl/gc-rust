//! Example 01 — hello-console: the classic text-mode hello world.
//!
//! Shows: `gc_std::init()`, `enable_console()`, `println!`, controller
//! polling. Press A to print, START to exit back to the loader.

#![no_std]
#![no_main]

use gc_std::{input::button, println};

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nHello World from Rust on the Nintendo GameCube!");
    println!("----------------------------------------------");
    println!("compiled with rustc, core + alloc, zero C code.");
    println!();
    println!("Press START to exit, A to greet.");

    loop {
        gc_std::video::wait_vsync();
        gc_std::input::scan();

        let pressed = gc_std::input::buttons_down(0);
        if pressed.contains(button::A) {
            println!("Button A pressed.");
        }
        if pressed.contains(button::START) {
            println!("Returning to loader...");
            gc_std::system::exit(0);
        }
    }
}
