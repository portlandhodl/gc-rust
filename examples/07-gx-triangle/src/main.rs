//! Example 07 — gx-triangle: the graphics-programming hello world.
//!
//! A rainbow triangle spinning around the camera (which looks down -Z),
//! drawn with immediate-mode vertex streaming: position + color per vertex.

#![no_std]
#![no_main]

use gc_std::{gu, gx, input::button};

const VERTS: [([f32; 3], [u8; 4]); 3] = [
    ([0.0, 1.0, 0.0], [255, 0, 0, 255]),
    ([-1.0, -1.0, 0.0], [0, 255, 0, 255]),
    ([1.0, -1.0, 0.0], [0, 0, 255, 255]),
];

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let gx = gc.into_gx();

    // --- one-time pipeline setup ---------------------------------------
    let proj = gu::perspective(60.0, 1.33, 1.0, 100.0);
    gx.load_perspective(&proj);

    gx::clear_vtx_desc();
    gx::set_vtx_desc_direct(gx::GX_VA_POS);
    gx::set_vtx_desc_direct(gx::GX_VA_CLR0);
    gx::set_vtx_attr_fmt(
        gx::VTXFMT0,
        gx::GX_VA_POS,
        gx::GX_POS_XYZ,
        gx::GX_F32,
        0,
    );
    gx::set_vtx_attr_fmt(
        gx::VTXFMT0,
        gx::GX_VA_CLR0,
        gx::GX_CLR_RGBA,
        gx::GX_RGBA8,
        0,
    );
    gx::config_vertex_color_pipeline();
    // the triangle is wound counter-clockwise, which GX treats as
    // back-facing; the default GX_CULL_BACK would hide it entirely
    gx::set_cull_mode(gx::GX_CULL_NONE);

    let camera = gu::vec3(0.0, 0.0, 0.0);
    let up = gu::vec3(0.0, 1.0, 0.0);
    let target = gu::vec3(0.0, 0.0, -1.0);

    let mut angle: f32 = 0.0;

    loop {
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        gx.begin_frame();

        let view = gu::look_at(camera, up, target);
        let mut model = gu::identity();
        gu::rotate_axis_deg(&mut model, gu::vec3(0.0, 0.0, 1.0), angle);
        let model = gu::translate(&model, 0.0, 0.0, -6.0);
        let modelview = gu::concat(&view, &model);
        gx.load_model_view(&modelview);

        gx::begin(gx::Primitive::Triangles, gx::VTXFMT0, 3);
        for (pos, col) in VERTS {
            gx::position3f32(pos[0], pos[1], pos[2]);
            gx::color4u8(col[0], col[1], col[2], col[3]);
        }

        gx.draw_done();
        gx.end_frame();

        angle += 1.5;
        if angle >= 360.0 {
            angle -= 360.0;
        }
    }
}
