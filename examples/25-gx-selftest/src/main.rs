//! Example 25 — gx-selftest: GX bring-up checks you can read off the TV.
//!
//! The text console stays on screen while GX renders into the EFB behind
//! it. Each test draws something, waits for the GP with a draw-sync token,
//! reads pixels back through the CPU's EFB window (`gx::peek_argb`) and
//! prints PASS/FAIL with the raw values. Lines appear one by one, so if the
//! console stops mid-way the last line printed is where it got stuck.
//!
//! Press START to exit.

#![no_std]
#![no_main]

use gc_std::{gu, gx, input::button, println};

static mut FAILS: u32 = 0;

fn mark(ok: bool) -> &'static str {
    if !ok {
        unsafe { FAILS += 1 };
    }
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

/// Queue a draw-sync token and wait for the GP to reach it.
fn sync(token: u16) -> bool {
    gx::set_draw_sync(token);
    for _ in 0..2_000_000 {
        if gx::draw_sync_token() == token {
            return true;
        }
    }
    false
}

fn rgb(argb: u32) -> [u8; 3] {
    [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b.iter()).all(|(&x, &y)| (i16::from(x) - i16::from(y)).abs() <= 12)
}

fn print_fifo(label: &str) {
    let f = gx::fifo_state();
    println!(
        "  {label}: CPSR {:04x} CPCR {:04x} dist {:x} rp {:x} wp {:x}",
        f.cp_status, f.cp_ctrl, f.distance, f.read_ptr, f.write_ptr
    );
    println!(
        "    cp base {:x} end {:x} | pi base {:x} end {:x} wr {:x}",
        f.base, f.end, f.pi_base, f.pi_end, f.pi_write
    );
}

fn tri(verts: [(f32, f32, f32); 3], c: [u8; 3]) {
    gx::begin(gx::Primitive::Triangles, gx::VTXFMT0, 3);
    for (x, y, z) in verts {
        gx::position3f32(x, y, z);
        gx::color4u8(c[0], c[1], c[2], 255);
    }
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32, z: f32, c: [u8; 3]) {
    gx::begin(gx::Primitive::Quads, gx::VTXFMT0, 4);
    for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
        gx::position3f32(x, y, z);
        gx::color4u8(c[0], c[1], c[2], 255);
    }
}

const BG: [u8; 3] = [16, 32, 64];
const GREEN: [u8; 3] = [40, 220, 60];
const RED: [u8; 3] = [230, 40, 40];
/// Wound clockwise on screen = front-facing in GX.
const CW: [(f32, f32, f32); 3] = [(0.0, 1.0, 0.0), (1.0, -1.0, 0.0), (-1.0, -1.0, 0.0)];
const CCW: [(f32, f32, f32); 3] = [(0.0, 1.0, 0.0), (-1.0, -1.0, 0.0), (1.0, -1.0, 0.0)];

#[no_mangle]
extern "C" fn main() -> i32 {
    let mut gc = gc_std::init();
    gc.enable_console();
    let mode = *gc.video().mode();
    let (w, h) = (f32::from(mode.fbWidth), f32::from(mode.efbHeight));
    let (cx, cy) = ((w * 0.5) as u16, (h * 0.5) as u16);

    println!("gc-rust GX self-test   ({}x{} EFB)", mode.fbWidth, mode.efbHeight);
    let (msr, hid0, hid2, wpar): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!("mfmsr {0}", out(reg) msr);
        core::arch::asm!("mfspr {0}, 1008", out(reg) hid0);
        core::arch::asm!("mfspr {0}, 920", out(reg) hid2);
        core::arch::asm!("mfspr {0}, 921", out(reg) wpar);
    }
    println!("MSR {msr:08x} HID0 {hid0:08x} HID2 {hid2:08x} WPAR {wpar:08x}");
    print_fifo("loader left");

    let gx = gc.into_gx_with_console();
    print_fifo("after GX init");

    // [1] does the GP consume our FIFO at all?
    let ok = sync(0x1111);
    println!("[1] {} GP reaches draw-sync token (got {:04x})", mark(ok), gx::draw_sync_token());
    print_fifo("after token");

    gx::clear_vtx_desc();
    gx::set_vtx_desc_direct(gx::GX_VA_POS);
    gx::set_vtx_desc_direct(gx::GX_VA_CLR0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_POS, gx::GX_POS_XYZ, gx::GX_F32, 0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_CLR0, gx::GX_CLR_RGBA, gx::GX_RGBA8, 0);
    gx::config_vertex_color_pipeline();
    let ortho = gu::ortho(0.0, h, 0.0, w, 0.1, 10.0);
    let persp = gu::perspective(60.0, w / h, 1.0, 100.0);
    let full_viewport = || gx::set_viewport(0.0, 0.0, w, h, 0.0, 1.0);

    // "clear": a full-screen 2D quad at the far plane that also resets z
    let clear = || {
        full_viewport();
        gx::load_projection_mtx(&ortho, gx::GX_ORTHOGRAPHIC);
        gx.load_model_view(&gu::identity());
        gx::set_cull_mode(gx::GX_CULL_NONE);
        gx::set_z_mode(true, gx::GX_ALWAYS, true);
        rect(0.0, 0.0, w, h, -9.9, BG);
    };

    // [2] 2D fill → EFB
    clear();
    let ok = sync(0x2222);
    let (a, b) = (gx::peek_argb(40, 40), gx::peek_argb(cx, cy));
    println!(
        "[2] {} 2D fill: px(40,40)={:06x} centre={:06x} want {:02x}{:02x}{:02x} (sync {})",
        mark(ok && near(rgb(a), BG) && near(rgb(b), BG)),
        a & 0xffffff,
        b & 0xffffff,
        BG[0],
        BG[1],
        BG[2],
        ok
    );
    println!("    z at centre after clear: {:06x}", gx::peek_z(cx, cy));

    // [3] 3D perspective triangle, no culling, no z
    clear();
    gx::load_projection_mtx(&persp, gx::GX_PERSPECTIVE);
    gx.load_model_view(&gu::translate(&gu::identity(), 0.0, 0.0, -6.0));
    gx::set_z_mode(false, gx::GX_ALWAYS, false);
    tri(CW, GREEN);
    let ok = sync(0x3333);
    let c = gx::peek_argb(cx, cy);
    let e = gx::peek_argb(20, 20);
    println!(
        "[3] {} 3D triangle: centre={:06x} corner={:06x}",
        mark(ok && near(rgb(c), GREEN) && near(rgb(e), BG)),
        c & 0xffffff,
        e & 0xffffff
    );

    // [4] depth test: near green first, far red second
    clear();
    gx::load_projection_mtx(&persp, gx::GX_PERSPECTIVE);
    gx.load_model_view(&gu::identity());
    gx::set_z_mode(true, gx::GX_LEQUAL, true);
    tri([(0.0, 1.0, -5.0), (1.0, -1.0, -5.0), (-1.0, -1.0, -5.0)], GREEN);
    let zn = {
        sync(0x4440);
        gx::peek_z(cx, cy)
    };
    tri([(0.0, 1.4, -7.0), (1.4, -1.4, -7.0), (-1.4, -1.4, -7.0)], RED);
    let ok = sync(0x4444);
    let c = gx::peek_argb(cx, cy);
    println!(
        "[4] {} depth test: centre={:06x} (green=pass, red=z broken) z={:06x} zn={:06x}",
        mark(ok && near(rgb(c), GREEN)),
        c & 0xffffff,
        gx::peek_z(cx, cy),
        zn
    );

    // [5] back-face culling: CW must draw, CCW must not
    clear();
    gx::load_projection_mtx(&persp, gx::GX_PERSPECTIVE);
    gx.load_model_view(&gu::translate(&gu::identity(), 0.0, 0.0, -6.0));
    gx::set_z_mode(false, gx::GX_ALWAYS, false);
    gx::set_cull_mode(gx::GX_CULL_BACK);
    tri(CW, GREEN);
    sync(0x5550);
    let front = gx::peek_argb(cx, cy);
    clear();
    gx::load_projection_mtx(&persp, gx::GX_PERSPECTIVE);
    gx.load_model_view(&gu::translate(&gu::identity(), 0.0, 0.0, -6.0));
    gx::set_z_mode(false, gx::GX_ALWAYS, false);
    gx::set_cull_mode(gx::GX_CULL_BACK);
    tri(CCW, RED);
    let ok = sync(0x5555);
    let back = gx::peek_argb(cx, cy);
    println!(
        "[5] {} cull back: CW={:06x} (want green) CCW={:06x} (want bg)",
        mark(ok && near(rgb(front), GREEN) && near(rgb(back), BG)),
        front & 0xffffff,
        back & 0xffffff
    );

    // [6] viewport: draw into the top-left quarter only
    clear();
    gx::set_viewport(0.0, 0.0, w * 0.5, h * 0.5, 0.0, 1.0);
    gx::load_projection_mtx(&persp, gx::GX_PERSPECTIVE);
    gx.load_model_view(&gu::translate(&gu::identity(), 0.0, 0.0, -3.0));
    gx::set_z_mode(false, gx::GX_ALWAYS, false);
    gx::set_cull_mode(gx::GX_CULL_NONE);
    tri(CW, GREEN);
    let ok = sync(0x6666);
    let (q, r) = (gx::peek_argb(cx / 2, cy / 2), gx::peek_argb(cx + cx / 2, cy + cy / 2));
    println!(
        "[6] {} viewport TL: inside={:06x} (green) other quarter={:06x} (bg)",
        mark(ok && near(rgb(q), GREEN) && near(rgb(r), BG)),
        q & 0xffffff,
        r & 0xffffff
    );

    // [7] a 20k-vertex burst (FIFO wrap/throughput), then a final check
    clear();
    gx::load_projection_mtx(&ortho, gx::GX_ORTHOGRAPHIC);
    gx.load_model_view(&gu::identity());
    gx::set_z_mode(false, gx::GX_ALWAYS, false);
    for i in 0..20u32 {
        let c = if i == 19 { GREEN } else { RED };
        gx::begin(gx::Primitive::Triangles, gx::VTXFMT0, 999);
        for k in 0..333u32 {
            let x = (k % 32) as f32 * 20.0;
            let y = (k / 32) as f32 * 40.0;
            for (dx, dy) in [(0.0, 0.0), (20.0, 0.0), (0.0, 40.0)] {
                gx::position3f32(x + dx, y + dy, -1.0);
                gx::color4u8(c[0], c[1], c[2], 255);
            }
        }
    }
    let ok = sync(0x7777);
    let p = gx::peek_argb(2, 2);
    println!("[7] {} 20k-vertex burst: px={:06x} (green) sync {}", mark(ok && near(rgb(p), GREEN)), p & 0xffffff, ok);
    print_fifo("after burst");

    let fails = unsafe { FAILS };
    println!();
    if fails == 0 {
        println!("ALL PASS");
    } else {
        println!("{fails} FAILED  - please photograph this screen");
    }
    println!("START = exit");

    loop {
        gc_std::video::wait_vsync();
        gc_std::input::scan();
        if gc_std::input::buttons_down(0).contains(button::START) {
            gc_std::system::exit(0);
        }
    }
}
