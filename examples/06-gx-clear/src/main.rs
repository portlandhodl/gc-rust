//! Example 06 — gx-clear: bring up the GX graphics processor and clear the
//! screen to a smoothly pulsing color.
//!
//! Every 3D example starts exactly like this. The EFB (embedded
//! framebuffer, where GX renders) is copied into the XFB after clearing,
//! so changing the *copy clear* color paints the whole screen.

#![no_std]
#![no_main]

use gc_std::input::button;

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let gx = gc.into_gx();

    let mut phase: u32 = 0;

    loop {
        // exit check
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        // Baby's first shader: RGB from three phase-shifted counters.
        let r = ((phase >> 1) & 0xff) as u8;
        let g = (phase.wrapping_add(85) & 0xff) as u8;
        let b = (phase.wrapping_add(170) & 0xff) as u8;
        gx.set_clear_color(gc_std::ffi::GXColor::rgba(r, g, b, 0xff));

        gx.begin_frame();
        gx.draw_done();
        gx.end_frame();

        phase = phase.wrapping_add(2);
    }
}
