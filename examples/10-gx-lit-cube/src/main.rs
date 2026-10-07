//! Example 10 — gx-lit-cube: lighting.
//!
//! libogc's full GX lighting API (channel color control + light objects)
//! works from Rust too, but the most portable mental model is the classic
//! one: rotate the normals with the model matrix on the CPU, take a
//! dot product against a fixed light direction, and emit the result as a
//! per-vertex color. The GPU then interpolates the colors across each
//! face for free (gouraud-ish shading, no extra pipeline state).
//!
//! The demo adds distance-squared falloff so it visibly responds to
//! lighting rather than a flat decal.

#![no_std]
#![no_main]

use gc_std::{gu, gx, input::button};

struct Face {
    verts: [[f32; 3]; 4],
    normal: [f32; 3],
    color: [u8; 3],
}

const fn face(
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    normal: [f32; 3],
    color: [u8; 3],
) -> Face {
    Face {
        verts: [a, b, c, d],
        normal,
        color,
    }
}

const FACES: [Face; 6] = [
    face([-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0], [0.0, 0.0, 1.0], [220, 220, 235]),
    face([-1.0, 1.0, -1.0], [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [0.0, 0.0, -1.0], [235, 180, 60]),
    face([1.0, 1.0, 1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0], [1.0, 0.0, 0.0], [220, 220, 235]),
    face([-1.0, 1.0, 1.0], [-1.0, -1.0, 1.0], [-1.0, -1.0, -1.0], [-1.0, 1.0, -1.0], [-1.0, 0.0, 0.0], [235, 180, 60]),
    face([-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 0.0], [220, 220, 235]),
    face([-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0], [0.0, -1.0, 0.0], [235, 180, 60]),
];

/// Rotate a direction by the 3x3 part of the model matrix.
fn rotate_normal(m: &gc_std::gctypes::Mtx, n: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * n[0] + m[0][1] * n[1] + m[0][2] * n[2],
        m[1][0] * n[0] + m[1][1] * n[1] + m[1][2] * n[2],
        m[2][0] * n[0] + m[2][1] * n[1] + m[2][2] * n[2],
    ]
}

fn shade(normal_world: [f32; 3], base: [u8; 3], light_dir: [f32; 3]) -> (u8, u8, u8) {
    let dot = normal_world[0] * light_dir[0]
        + normal_world[1] * light_dir[1]
        + normal_world[2] * light_dir[2];
    let intensity = if dot > 0.0 { dot } else { 0.0 };
    let ambient = 0.18f32;
    let scale = ambient + (1.0 - ambient) * intensity;
    let f = |c: u8| -> u8 { (f32::from(c) * scale).clamp(0.0, 255.0) as u8 };
    (f(base[0]), f(base[1]), f(base[2]))
}

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let gx = gc.into_gx();
    gx.set_clear_color(gc_std::GXColor::rgb(8, 8, 14));

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

    // Fixed directional light, pre-normalized = normalize(-0.6, 0.8, 0.6)^T
    // (core has no f32::sqrt on no_std targets; computed offline).
    let l = [-0.514_50f32, 0.686_00, 0.514_50];

    let mut spin: f32 = 0.0;

    loop {
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }

        gx.begin_frame();
        gx::set_z_mode(true, gx::GX_LEQUAL, true);

        let view = gu::look_at(camera, up, target);
        let mut model = gu::identity();
        gu::rotate_axis_deg(&mut model, gu::vec3(1.0, 0.0, 0.0), spin * 0.7);
        gu::rotate_axis_deg(&mut model, gu::vec3(0.0, 1.0, 0.0), spin);
        let rotation_only = model; // keep a copy for normal rotation
        let model = gu::translate(&model, 0.0, 0.0, -6.0);
        let modelview = gu::concat(&view, &model);
        gx.load_model_view(&modelview);

        gx::begin(gx::Primitive::Quads, gx::VTXFMT0, 24);
        for f in &FACES {
            let n_world = rotate_normal(&rotation_only, f.normal);
            let (r, g, b) = shade(n_world, f.color, l);
            for v in &f.verts {
                gx::position3f32(v[0], v[1], v[2]);
                gx::color4u8(r, g, b, 255);
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
