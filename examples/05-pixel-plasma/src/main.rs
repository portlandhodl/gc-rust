//! Example 05 — pixel-plasma: software rendering without GX.
//!
//! The framebuffer the GameCube's video interface (VI) displays is not RGB:
//! it's YUV 4:2:2 ("YUY2"). Every 32-bit word holds two horizontally
//! adjacent pixels as `[Y0][U][Y1][V]`. This example renders a classic
//! plasma effect into it with a sine LUT + palette lookup (fixed point, no
//! `libm`): one more thing you can only enjoy on bare metal.
//!
//! Rendering happens into the back buffer of the VI flip chain: each frame
//! `video.flip()` (vsync + swap) publishes it, so the beam never samples a
//! half-drawn frame.

#![no_std]
#![no_main]

use gc_std::input::button;

/// 256-entry sine LUT, output 0..=255.
fn build_sine_lut() -> [u8; 256] {
    let mut lut = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        // sin(2*pi*i/256) computed the boring way (build-time-ish, once).
        let x = (i as f32) * (core::f32::consts::PI * 2.0 / 256.0);
        lut[i] = (128.0 + 127.0 * libm::sinf(x)) as u8;
        i += 1;
    }
    lut
}

/// Smooth 256-color rainbow palette in RGB.
fn build_palette() -> [[u8; 3]; 256] {
    let mut pal = [[0u8; 3]; 256];
    let mut i = 0usize;
    while i < 256 {
        let t = i as f32;
        pal[i][0] = (128.0 + 127.0 * libm::sinf(t * 0.0246)) as u8;
        pal[i][1] = (128.0 + 127.0 * libm::sinf(t * 0.0246 + 2.0)) as u8;
        pal[i][2] = (128.0 + 127.0 * libm::sinf(t * 0.0246 + 4.0)) as u8;
        i += 1;
    }
    pal
}

/// RGB -> YCbCr (BT.601, studio swing), integer math.
fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    (
        y.clamp(0, 255) as u8,
        u.clamp(0, 255) as u8,
        v.clamp(0, 255) as u8,
    )
}

/// Minimal sine lookup — we deliberately avoid libm; this tiny Taylor
/// series with argument reduction is plenty for a demo LUT.
mod libm {
    pub fn sinf(x: f32) -> f32 {
        const TWO_PI: f32 = core::f32::consts::PI * 2.0;
        let mut x = x % TWO_PI;
        if x > core::f32::consts::PI {
            x -= TWO_PI;
        } else if x < -core::f32::consts::PI {
            x += TWO_PI;
        }
        let x2 = x * x;
        x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0))))
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    gc.video().show();

    let mode = gc.video().mode();
    let width = mode.fbWidth as usize;
    let height = mode.xfbHeight as usize;
    let mut vid = *gc.video();

    let lut = build_sine_lut();
    let palette = build_palette();

    // Precompute per-index Y/U/V once.
    let mut py = [0u8; 256];
    let mut pu = [0u8; 256];
    let mut pv = [0u8; 256];
    for i in 0..256 {
        let (y, u, v) = rgb_to_yuv(palette[i][0], palette[i][1], palette[i][2]);
        py[i] = y;
        pu[i] = u;
        pv[i] = v;
    }

    let mut t: u32 = 0;

    loop {
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        let xfb = vid.framebuffer().cast::<u32>();
        for y in 0..height {
            let row = unsafe { xfb.add(y * width / 2) };
            let ly = lut[((y as u32 / 2) & 255) as usize] as u32;
            let lyt = lut[(((y as u32) + t) & 255) as usize] as u32;

            let mut x = 0usize;
            while x < width {
                // Two-pair pixels share one 32-bit YUYV word.
                let c0 = lut[((x as u32 + t) & 255) as usize] as u32
                    + ly
                    + lut[(((x as u32 + y as u32) / 2 + t) & 255) as usize] as u32
                    + lyt;
                let c1 = lut[((x as u32 + 1 + t) & 255) as usize] as u32
                    + ly
                    + lut[(((x as u32 + 1 + y as u32) / 2 + t) & 255) as usize] as u32
                    + lyt;
                let i0 = (c0 / 4) as u8;
                let i1 = (c1 / 4) as u8;
                let (y0, y1) = (py[i0 as usize], py[i1 as usize]);
                let uu = ((pu[i0 as usize] as u16 + pu[i1 as usize] as u16) / 2) as u8;
                let vv = ((pv[i0 as usize] as u16 + pv[i1 as usize] as u16) / 2) as u8;

                let word = u32::from_be_bytes([y0, uu, y1, vv]);
                unsafe { row.add(x / 2).write_volatile(word) };
                x += 2;
            }
        }

        t = t.wrapping_add(3);
        vid = vid.flip();
    }
}
