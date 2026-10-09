//! Example 24 — gx-diag: a GX bring-up test card for real hardware.
//!
//! The screen is split into four quadrants (white divider lines) and each
//! one draws the same spinning rainbow triangle with exactly one pipeline
//! setting changed, so a photo of the TV tells which stage misbehaves:
//!
//! ```text
//!  +------------------------+------------------------+
//!  | 2D: orthographic       | 3D depth test: a green |
//!  | no culling, no z test  | triangle in front, red |
//!  |                        | one behind, drawn LAST |
//!  +------------------------+------------------------+
//!  | 3D + back-face culling | 3D + back-face culling |
//!  | triangle wound CCW     | triangle wound CW      |
//!  +------------------------+------------------------+
//! ```
//!
//! Expected: top-left rainbow triangle; top-right green triangle with red
//! peeking out behind it (red covering green = broken depth buffer);
//! bottom-left EMPTY (CCW is back-facing in GX); bottom-right rainbow.
//! The background pulses so a frozen frame is obvious.

#![no_std]
#![no_main]

use gc_std::{gu, gx, input::button};

const CCW: [([f32; 3], [u8; 4]); 3] = [
    ([0.0, 1.0, 0.0], [255, 0, 0, 255]),
    ([-1.0, -1.0, 0.0], [0, 255, 0, 255]),
    ([1.0, -1.0, 0.0], [0, 0, 255, 255]),
];
const CW: [([f32; 3], [u8; 4]); 3] = [CCW[0], CCW[2], CCW[1]];

fn triangle(verts: &[([f32; 3], [u8; 4]); 3]) {
    gx::begin(gx::Primitive::Triangles, gx::VTXFMT0, 3);
    for (p, c) in verts {
        gx::position3f32(p[0], p[1], p[2]);
        gx::color4u8(c[0], c[1], c[2], c[3]);
    }
}

fn solid_triangle(z: f32, dx: f32, c: [u8; 3]) {
    gx::begin(gx::Primitive::Triangles, gx::VTXFMT0, 3);
    for (x, y) in [(0.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        gx::position3f32(x + dx, y, z);
        gx::color4u8(c[0], c[1], c[2], 255);
    }
}

fn quad(x0: f32, y0: f32, x1: f32, y1: f32) {
    gx::begin(gx::Primitive::Quads, gx::VTXFMT0, 4);
    for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
        gx::position3f32(x, y, -1.0);
        gx::color4u8(255, 255, 255, 255);
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let mode = *gc.video().mode();
    let (w, h) = (f32::from(mode.fbWidth), f32::from(mode.efbHeight));
    let gx = gc.into_gx();

    gx::clear_vtx_desc();
    gx::set_vtx_desc_direct(gx::GX_VA_POS);
    gx::set_vtx_desc_direct(gx::GX_VA_CLR0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_POS, gx::GX_POS_XYZ, gx::GX_F32, 0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_CLR0, gx::GX_CLR_RGBA, gx::GX_RGBA8, 0);
    gx::config_vertex_color_pipeline();

    let persp = gu::perspective(60.0, w / h, 1.0, 100.0);
    let ortho_unit = gu::ortho(1.25, -1.25, -1.25 * w / h, 1.25 * w / h, 0.1, 10.0);
    let ortho_px = gu::ortho(0.0, h, 0.0, w, 0.1, 10.0);

    let mut angle: f32 = 0.0;
    let mut frame: u32 = 0;
    loop {
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        let pulse = (frame % 120) as u8;
        let pulse = if pulse < 60 { pulse } else { 120 - pulse };
        gx.set_clear_color(gc_std::GXColor::rgb(20, 20 + pulse, 60 + pulse));
        gx.begin_frame();

        let mut spin = gu::identity();
        gu::rotate_axis_deg(&mut spin, gu::vec3(0.0, 0.0, 1.0), angle);

        for q in 0..4u32 {
            let (qx, qy) = ((q % 2) as f32 * w * 0.5, (q / 2) as f32 * h * 0.5);
            gx::set_viewport(qx, qy, w * 0.5, h * 0.5, 0.0, 1.0);
            if q == 0 {
                gx::load_projection_mtx(&ortho_unit, gx::GX_ORTHOGRAPHIC);
                gx::set_cull_mode(gx::GX_CULL_NONE);
                gx::set_z_mode(false, gx::GX_ALWAYS, false);
                gx.load_model_view(&gu::translate(&spin, 0.0, 0.0, -1.0));
            } else {
                gx::load_projection_mtx(&persp, gx::GX_PERSPECTIVE);
                gx::set_cull_mode(if q == 1 { gx::GX_CULL_NONE } else { gx::GX_CULL_BACK });
                gx::set_z_mode(true, gx::GX_LEQUAL, true);
                gx.load_model_view(&gu::translate(&spin, 0.0, 0.0, -6.0));
            }
            if q == 1 {
                // near green first, then the far red one: only depth
                // testing keeps the red one behind
                gx.load_model_view(&gu::translate(&spin, 0.0, 0.0, 0.0));
                solid_triangle(-5.0, 0.0, [40, 220, 60]);
                solid_triangle(-7.0, 0.7, [230, 40, 40]);
            } else {
                triangle(if q == 3 { &CW } else { &CCW });
            }
        }

        // white dividers, drawn in 2D pixel coordinates over everything
        gx::set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
        gx::load_projection_mtx(&ortho_px, gx::GX_ORTHOGRAPHIC);
        gx::set_cull_mode(gx::GX_CULL_NONE);
        gx::set_z_mode(false, gx::GX_ALWAYS, false);
        gx.load_model_view(&gu::identity());
        quad(w * 0.5 - 2.0, 0.0, w * 0.5 + 2.0, h);
        quad(0.0, h * 0.5 - 2.0, w, h * 0.5 + 2.0);

        gx.draw_done();
        gx.end_frame();
        angle = if angle >= 358.5 { 0.0 } else { angle + 1.5 };
        frame = frame.wrapping_add(1);
    }
}
