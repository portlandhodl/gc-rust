//! Example 21 — thread-sync: the background-agent pattern.
//!
//! The main loop (the "game") produces work items on a channel when you
//! press B. The background agent thread parks on the channel (sleeping),
//! wakes per item, pretends to work briefly, and acks back on a second
//! channel. A Mutex guards console output so lines never interleave.
//! START exits.

#![no_std]
#![no_main]

extern crate alloc;
use alloc::boxed::Box;

use gc_std::{
    input::{self, button},
    lwp, lwp_sync::{Channel, Mutex},
    println,
};

struct Wire {
    jobs: Channel<i32>,
    done: Channel<i32>,
    print: Mutex,
}

impl Wire {
    fn new() -> Wire {
        Wire { jobs: Channel::into_self(8), done: Channel::into_self(8), print: Mutex::new() }
    }
    fn say(&self, s: &str) {
        self.print.lock();
        println!("{s}");
        self.print.unlock();
    }
}

extern "C" fn agent(ptr: u32) -> usize {
    let w = unsafe { &mut *(ptr as *mut Wire) };
    let mut count = 0i32;
    loop {
        w.say("[agent] waiting for work");
        // blocks here until the game posts something (thread parked)
        if let Some(job) = w.jobs.recv() {
            count += 1;
            w.print.lock();
            println!("[agent] working on job #{job} (total {count})");
            w.print.unlock();
            lwp::sleep_ms(250); // pretend to work
            let ack = job.wrapping_mul(0x6E37);
            w.done.send(ack);
        }
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    println!("\nExample 21 - thread-sync");
    println!("B = enqueue a job for the agent; START exits.");

    let wire: &'static mut Wire = Box::leak(Box::new(Wire::new()));
    lwp::spawn(agent, wire as *mut Wire as u32, 16 * 1024).expect("spawn agent");
    println!("agent spawned.");

    let mut jobs = 0i32;
    loop {
        gc_std::video::wait_vsync();
        input::scan();

        if input::buttons_down(0).contains(button::B) {
            jobs += 1;
            wire.jobs.send(jobs);
            println!("fg: queued job #{jobs}");
        }
        while let Some(ack) = wire.done.try_recv() {
            println!("fg: ack {ack:#010x}");
        }
        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
