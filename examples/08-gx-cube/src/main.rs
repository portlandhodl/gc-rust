//! Example 08 — gx-cube: a real 3D object.
//!
//! A cube built from 6 quads (24 vertices), spinning on two axes with the
//! depth buffer enabled, each face a different color. This is the baseline
//! for anything 3D on the console.

#![no_std]
#![no_main]

use gc_std::{gu, gx, input::button};

/// A quad face: 4 vertices + a color.
struct Face {
    verts: [[f32; 3]; 4],
    color: [u8; 4],
}

const fn face(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], color: [u8; 4]) -> Face {
    Face {
        verts: [a, b, c, d],
        color,
    }
}

const FACES: [Face; 6] = [
    // front (+Z) red
    face([-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0], [255, 0, 0, 255]),
    // back (-Z) cyan
    face([-1.0, 1.0, -1.0], [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [0, 255, 255, 255]),
    // right (+X) green
    face([1.0, 1.0, 1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0], [0, 255, 0, 255]),
    // left (-X) magenta
    face([-1.0, 1.0, 1.0], [-1.0, -1.0, 1.0], [-1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [255, 0, 255, 255]),
    // top (+Y) blue
    face([-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [0, 0, 255, 255]),
    // bottom (-Y) yellow
    face([-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0], [255, 255, 0, 255]),
];

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let gx = gc.into_gx();
    gx.set_clear_color(gc_std::ffi::GXColor::rgb(20, 24, 48));

    let proj = gu::perspective(60.0, 1.33, 1.0, 100.0);
    gx.load_perspective(&proj);

    gx::clear_vtx_desc();
    gx::set_vtx_desc_direct(gx::GX_VA_POS);
    gx::set_vtx_desc_direct(gx::GX_VA_CLR0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_POS, gx::GX_POS_XYZ, gx::GX_F32, 0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_CLR0, gx::GX_CLR_RGBA, gx::GX_RGBA8, 0);
    gx::config_vertex_color_pipeline();

    let camera = gu::vec3(0.0, 0.0, 0.0);
    let up = gu::vec3(0.0, 1.0, 0.0);
    let target = gu::vec3(0.0, 0.0, -1.0);

    let mut spin: f32 = 0.0;

    loop {
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        gx.begin_frame();
        gx::set_z_mode(true, gx::GX_LEQUAL, true);

        // tumble the cube in front of the camera
        let view = gu::look_at(camera, up, target);
        let mut model = gu::identity();
        gu::rotate_axis_deg(&mut model, gu::vec3(1.0, 0.0, 0.0), spin * 0.5);
        gu::rotate_axis_deg(&mut model, gu::vec3(0.0, 1.0, 0.0), spin);
        let model = gu::translate(&model, 0.0, 0.0, -6.0);
        let modelview = gu::concat(&view, &model);
        gx.load_model_view(&modelview);

        gx::begin(gx::Primitive::Quads, gx::VTXFMT0, 24);
        for f in &FACES {
            for v in &f.verts {
                gx::position3f32(v[0], v[1], v[2]);
                gx::color4u8(f.color[0], f.color[1], f.color[2], f.color[3]);
            }
        }

        gx.draw_done();
        gx.end_frame();

        spin += 1.0;
        if spin >= 360.0 {
            spin -= 360.0;
        }
    }
}
