//! Example 22 — net-echo: BBA (Broadband Adapter) ethernet bring-up.
//!
//! Sequence: probe the BBA chip, init it, ARP for the Dolphin slirp gateway,
//! then ICMP-ping it every couple of seconds from the background thread.
//! Each pong gets counted and reported on the console.
//!
//! Works with Dolphin's built-in BBA emulation ("Serial Port 2 = Ethernet
//! Built-in"). Press START to exit.

#![no_std]
#![no_main]

use gc_std::{
    bba,
    input::{self, button},
    lwp, net, observe,
    println,
};

extern "C" fn agent(_arg: u32) -> usize {
    observe::set(10, 0xDEAD_BEEF); // "agent live"
    // wait for a MAC to be known, kick a ping every 2 s
    let mut seq = 1u16;
    loop {
        if net::arp_lookup(net::GATEWAY).is_none() {
            net::arp_resolve(net::GATEWAY);
        }
        net::ping(net::GATEWAY, 0xBEEF, seq);
        seq = seq.wrapping_add(1);
        lwp::sleep_ms(2000);
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();

    observe::heartbeat(22);
    println!("\nExample 22 - net-echo");

    if !bba::probe() {
        println!("no BBA on this console (exi ch0 dev2)");
        exit_loop();
    }
    println!("BBA found.");
    if !bba::init(None) {
        println!("bba init failed (no link?)");
        exit_loop();
    }
    println!("BBA up.  MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bba::mac()[0], bba::mac()[1], bba::mac()[2],
        bba::mac()[3], bba::mac()[4], bba::mac()[5]);

    lwp::spawn(agent, 0, 16 * 1024).expect("agent");

    println!("agent pinging gateway {}.{}.{}.{}...",
        net::GATEWAY[0], net::GATEWAY[1], net::GATEWAY[2], net::GATEWAY[3]);

    let mut n = 0u32;
    loop {
        gc_std::video::wait_vsync();
        input::scan();

        net::poll();

        // aggregate counters (mailbox = watchable by tests)
        observe::set(11, net::icmp_pongs());

        if input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
        n = n.wrapping_add(1);
        if n % 300 == 0 {
            println!("pongs so far: {}", net::icmp_pongs());
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
