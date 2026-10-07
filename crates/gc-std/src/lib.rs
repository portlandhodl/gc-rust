//! # gc-std
//!
//! The "standard library" for writing Nintendo GameCube software in Rust.
//!
//! This crate sits on top of devkitPro's [libogc] and provides:
//!
//! * the Rust runtime glue (`#[panic_handler]`, `#[alloc_error_handler]`,
//!   a malloc-backed `#[global_allocator]`) so `core` and `alloc` work —
//!   `Vec`, `String`, `Box`, `format!` and friends are all usable;
//! * a text console over [`console`] with [`print!`] / [`println!`];
//! * safe wrappers around the console hardware: [`video`], controller
//!   [`input`], the [`gx`] graphics processor and [`gu`] matrix math;
//! * [`system::exit`] to return to the loader.
//!
//! Initialization comes in two flavors:
//!
//! ```rust,no_run
//! #![no_std]
//! #![no_main]
//! use gc_std::{input::button, println};
//!
//! #[no_mangle]
//! extern "C" fn main() -> i32 {
//!     // Text console mode (println! etc.)
//!     let mut gc = gc_std::init();
//!     gc.enable_console();
//!     println!("Hello from Rust!");
//!
//!     loop {
//!         gc_std::video::wait_vsync();
//!         gc_std::input::scan();
//!         if gc_std::input::buttons_down(0).contains(button::START) {
//!             gc_std::system::exit(0);
//!         }
//!     }
//! }
//! ```
//!
//! or for 3D:
//!
//! ```rust,no_run
//! # let _ = |gc: gc_std::Gc| {
//!     // GX accelerated graphics mode
//!     let gx = gc.into_gx();
//! # };
//! ```
//!
//! [libogc]: https://github.com/devkitPro/libogc

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod console;
pub mod ffi;
pub mod gu;
pub mod gx;
pub mod input;
pub mod system;
pub mod video;

mod allocator;
mod runtime;

use core::sync::atomic::{AtomicU32, Ordering};
use video::Video;

const UNINITIALIZED: u32 = 0;
const BASE_INITIALIZED: u32 = 1;
const CONSOLE_ENABLED: u32 = 2;
const _GX_ENABLED: u32 = 3;

static INIT_STATE: AtomicU32 = AtomicU32::new(UNINITIALIZED);

/// Token proving the console hardware has been initialized.
///
/// Owned handle to the machine; decide early whether you want a text
/// console ([`Gc::enable_console`]) or the GPU ([`Gc::into_gx`]).
pub struct Gc {
    video: Video,
    _not_send: core::marker::PhantomData<*const ()>,
}

impl Gc {
    /// Access to the video subsystem (mode info, framebuffer, vsync).
    pub fn video(&self) -> &Video {
        &self.video
    }

    /// Install the framebuffer text console (`print!`/`println!` start
    /// producing visible output) and start the display.
    ///
    /// # Panics
    ///
    /// Panics if the console was already enabled.
    pub fn enable_console(&mut self) -> &mut Self {
        if INIT_STATE
            .compare_exchange(
                BASE_INITIALIZED,
                CONSOLE_ENABLED,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            panic!("gc_std: console already enabled (or GX in use)");
        }
        console::init(&self.video);
        self.video.show();
        self
    }

    /// Hand the machine over to the GX graphics processor.
    ///
    /// Performs full GX pipeline bring-up (command FIFO, copy setup,
    /// viewport/scissor) pointed at the allocated framebuffer, and starts
    /// the display.
    pub fn into_gx(self) -> gx::Context {
        INIT_STATE.store(_GX_ENABLED, Ordering::Release);
        let ctx = gx::init(Video::clone_handle(&self.video));
        self.video.show();
        ctx
    }
}

/// Initialize the GameCube: video hardware, controller ports and the
/// external framebuffer.
///
/// Afterwards choose your output path: [`Gc::enable_console`] for text or
/// [`Gc::into_gx`] for 3D graphics.
///
/// # Panics
///
/// Panics if called more than once.
pub fn init() -> Gc {
    if INIT_STATE
        .compare_exchange(
            UNINITIALIZED,
            BASE_INITIALIZED,
            Ordering::Acquire,
            Ordering::Relaxed,
        )
        .is_err()
    {
        panic!("gc_std::init() called more than once");
    }

    let video = video::init();
    input::init();

    Gc {
        video,
        _not_send: core::marker::PhantomData,
    }
}
