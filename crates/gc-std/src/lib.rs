//! # gc-std
//!
//! The standard-library layer for Nintendo GameCube development in Rust —
//! **no C toolchain, no libogc binary, no devkitPro.**
//!
//! * 100% Rust: startup code (`crt0`), runtime glue, MMIO drivers for the
//!   VI (video), SI (controllers), the GX graphics processor, and a heap
//!   allocator enabling the full `alloc` crate (`Vec`, `String`, `Box`,
//!   `format!`, ...).
//! * Console text over the framebuffer: [`print!`] / [`println!`].
//! * 3D graphics through the [`gx`] module (immediate-mode rendering).
//! * Matrix math ([`gu`]), video mode control ([`video`]), and controller
//!   input ([`input`]).
//!
//! Entry point convention is the same as libogc-based projects: define
//!
//! ```rust,no_run
//! #[no_mangle]
//! extern "C" fn main() -> i32 {
//!     let mut gc = gc_std::init();
//!     gc.enable_console();
//!     println!("Hello, GameCube!");
//!     loop {}
//! }
//! ```
//!
//! inside a `#![no_std] #![no_main]` binary.

#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

pub mod aesnd;
pub mod aram;
pub mod audio;
pub mod card;
pub mod console;
pub mod dsp;
pub mod dspcode;
pub mod exi;
pub mod gctypes;
pub mod gu;
pub mod gx;
mod heap;
pub mod hw;
pub mod input;
pub mod irq;
pub mod sram;
pub mod system;
pub mod timebase;
pub mod usbgecko;
pub mod video;

mod crt0;
mod font;
mod runtime;

#[doc(no_inline)]
pub use gctypes::{GXColor, GXRModeObj, Mtx as RawMtx, Mtx44};
pub use video::Video;

use core::sync::atomic::{AtomicBool, Ordering};

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static CONSOLE_ON: AtomicBool = AtomicBool::new(false);
static GX_ON: AtomicBool = AtomicBool::new(false);

static mut CONSOLE: Option<console::Console> = None;

const CONSOLE_MARGIN: usize = 20;

/// Handle to the initialized machine.
pub struct Gc {
    video: Video,
    _not_send: core::marker::PhantomData<*const ()>,
}

impl Gc {
    pub fn video(&self) -> &Video {
        &self.video
    }

    /// Start the framebuffer text console. `print!`/`println!` output will
    /// now be visible. Consumes the "text or graphics" choice.
    pub fn enable_console(&mut self) -> &mut Self {
        if CONSOLE_ON.swap(true, Ordering::AcqRel) || GX_ON.load(Ordering::Acquire) {
            panic!("gc_std: console already enabled (or GX in use)");
        }
        let con = console::Console::new(&self.video, CONSOLE_MARGIN);
        unsafe {
            *(&raw mut CONSOLE) = Some(con);
        }
        self
    }

    /// Initialize the GX 3D pipeline. `gc.video()` remains available for
    /// the low-level pieces; 3D rendering goes through the returned context.
    pub fn into_gx(self) -> gx::Context {
        if GX_ON.swap(true, Ordering::AcqRel) || CONSOLE_ON.load(Ordering::Acquire) {
            panic!("gc_std: GX already in use (or console enabled)");
        }
        gx::init(&self.video)
    }
}

/// Initialize the machine (video hardware, controller ports, framebuffer,
/// heap). Exactly once.
pub fn init() -> Gc {
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        panic!("gc_std::init() called twice");
    }
    irq::init();
    input::init();
    let video = video::init();
    Gc {
        video,
        _not_send: core::marker::PhantomData,
    }
}

// internal: get the console for print macros (None until enable_console)
#[doc(hidden)]
pub fn _console() -> &'static mut Option<console::Console> {
    unsafe { &mut *core::ptr::addr_of_mut!(CONSOLE) }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __console_print_impl {
    ($($arg:tt)*) => {{
        if let Some(con) = $crate::_console().as_mut() {
            use core::fmt::Write;
            let _ = con.write_fmt(core::format_args!($($arg)*));
        }
    }};
}

/// Print to the framebuffer console (if enabled).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::__console_print_impl!($($arg)*) };
}

/// Print to the framebuffer console, with newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::__console_print_impl!($($arg)*);
        $crate::__console_print_impl!("\n");
    }};
}
