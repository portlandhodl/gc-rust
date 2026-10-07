//! Example 09 — gx-textured-cube: textures!
//!
//! Builds a 64x64 RGB565 checkerboard procedurally, uploads it through
//! [`gx::Texture`] (which performs the 4x4 tile swizzle the GPU requires
//! and flushes the data cache), and draws a spinning textured cube.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;
use gc_std::{gu, gx, input::button};

const TEX: u16 = 64;

/// "Berkeley colors" checkerboard with a border, RGB565.
fn build_checkerboard() -> alloc::vec::Vec<u16> {
    let mut px = vec![0u16; (TEX as usize) * (TEX as usize)];
    let on: u16 = 0xF800; // red 565
    let off: u16 = 0xFFFF; // white
    let bd: u16 = 0x0000; // black border
    for y in 0..TEX {
        for x in 0..TEX {
            let i = (y as usize) * (TEX as usize) + x as usize;
            px[i] = if x == 0 || y == 0 || x == TEX - 1 || y == TEX - 1 {
                bd
            } else if ((x / 8) ^ (y / 8)) & 1 == 0 {
                on
            } else {
                off
            };
        }
    }
    px
}

struct Face {
    verts: [[f32; 3]; 4],
}

const fn face(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> Face {
    Face { verts: [a, b, c, d] }
}

const FACES: [Face; 6] = [
    face([-1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, -1.0, 1.0], [-1.0, -1.0, 1.0]),
    face([-1.0, 1.0, -1.0], [-1.0, -1.0, -1.0], [1.0, -1.0, -1.0], [1.0, 1.0, -1.0]),
    face([1.0, 1.0, 1.0], [1.0, 1.0, -1.0], [1.0, -1.0, -1.0], [1.0, -1.0, 1.0]),
    face([-1.0, 1.0, 1.0], [-1.0, -1.0, 1.0], [-1.0, -1.0, -1.0], [-1.0, 1.0, -1.0]),
    face([-1.0, 1.0, 1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 1.0, 1.0]),
    face([-1.0, -1.0, 1.0], [1.0, -1.0, 1.0], [1.0, -1.0, -1.0], [-1.0, -1.0, -1.0]),
];

const UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let gx = gc.into_gx();
    gx.set_clear_color(gc_std::GXColor::rgb(24, 30, 24));

    let proj = gu::perspective(60.0, 1.33, 1.0, 100.0);
    gx.load_perspective(&proj);

    // texture upload (swizzle + cache flush handled by gx::Texture)
    let checkerboard = gx::Texture::from_rgb565(TEX, TEX, &build_checkerboard(), gx::WrapMode::Repeat);

    gx::clear_vtx_desc();
    gx::set_vtx_desc_direct(gx::GX_VA_POS);
    gx::set_vtx_desc_direct(gx::GX_VA_CLR0);
    gx::set_vtx_desc_direct(gx::GX_VA_TEX0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_POS, gx::GX_POS_XYZ, gx::GX_F32, 0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_CLR0, gx::GX_CLR_RGBA, gx::GX_RGBA8, 0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_TEX0, gx::GX_TEX_ST, gx::GX_F32, 0);
    gx::config_textured_pipeline();

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

        let view = gu::look_at(camera, up, target);
        let mut model = gu::identity();
        gu::rotate_axis_deg(&mut model, gu::vec3(1.0, 0.0, 0.0), spin * 0.5);
        gu::rotate_axis_deg(&mut model, gu::vec3(0.0, 1.0, 0.0), spin);
        let model = gu::translate(&model, 0.0, 0.0, -6.0);
        let modelview = gu::concat(&view, &model);
        gx.load_model_view(&modelview);

        checkerboard.bind();

        gx::begin(gx::Primitive::Quads, gx::VTXFMT0, 24);
        for f in &FACES {
            for (vi, v) in f.verts.iter().enumerate() {
                gx::position3f32(v[0], v[1], v[2]);
                gx::color4u8(255, 255, 255, 255); // let the texture dominate
                gx::texcoord2f32(UV[vi][0], UV[vi][1]);
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
