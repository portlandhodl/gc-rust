//! Text console over the external framebuffer, plus the `print!` /
//! `println!` machinery.
//!
//! Writing goes through newlib's `printf`, which libogc routes to the
//! framebuffer once [`crate::init`] has installed the console.

use core::ffi::c_char;
use core::fmt::{self, Write};

use crate::ffi;
use crate::video::Video;

const CONSOLE_MARGIN: i32 = 20;

pub(crate) fn init(video: &Video) {
    let mode = video.mode();
    unsafe {
        ffi::CON_Init(
            video.framebuffer(),
            CONSOLE_MARGIN,
            CONSOLE_MARGIN,
            i32::from(mode.fbWidth),
            i32::from(mode.xfbHeight),
            i32::from(mode.fbWidth) * ffi::VI_DISPLAY_PIX_SZ as i32,
        );
    }
}

/// Streams formatted output straight into `printf` chunk by chunk, so no
/// heap allocation or intermediate buffer is required.
struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if !s.is_empty() {
            unsafe {
                // "%.*s" works on &[u8]-style (ptr, len) pairs and avoids the
                // need for a NUL-terminated copy.
                ffi::printf(
                    b"%.*s\0".as_ptr().cast::<c_char>(),
                    s.len() as i32,
                    s.as_ptr().cast::<c_char>(),
                );
            }
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let _ = ConsoleWriter.write_fmt(args);
}

/// Prints to the framebuffer console.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::console::_print(core::format_args!($($arg)*));
    }};
}

/// Prints to the framebuffer console, with a newline.
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::console::_print(core::format_args!("{}\n", core::format_args!($($arg)*)));
    }};
}
