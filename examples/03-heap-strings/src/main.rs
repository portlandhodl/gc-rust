//! Example 03 — heap-strings: the `alloc` crate on bare-metal GameCube.
//!
//! gc-std wires `#[global_allocator]` to newlib's malloc (which libogc
//! places on top of the system heap), so `Vec`, `String`, `BTreeMap`,
//! `format!`, `Box` ... all just work.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use gc_std::{input::button, println};

fn fib(n: u32) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        (a, b) = (b, a + b);
    }
    a
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 03 - alloc / heap / strings");
    println!();

    // 1. Vec: grow, compute, sort.
    let mut values: Vec<i32> = (0..20i32).map(|i| i.wrapping_mul(1_103_515_245) % 100).collect();
    values.sort_unstable();
    println!("sorted vec:  {:?}", values);

    // 2. String + format!.
    let mut s = String::from("gamecube");
    s.make_ascii_uppercase();
    println!("string ops:  {s}");
    println!("format!:     {}", format!("fib(30) = {}", fib(30)));

    // 3. BTreeMap: high scores!
    let mut scores = BTreeMap::new();
    scores.insert("AAA", 900_000u32);
    scores.insert("SAMUS", 481_516);
    scores.insert("LINK", 2_147_483);
    println!("high scores:");
    for (name, score) in &scores {
        println!("  {name:6} {score:10}");
    }

    // 4. Box
    let boxed: Box<u64> = Box::new(0xDEAD_BEEF_CAFE_BABE);
    println!("boxed:       {:#x}", *boxed);

    println!();
    println!("Press START to exit.");
    loop {
        gc_std::video::wait_vsync();
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
