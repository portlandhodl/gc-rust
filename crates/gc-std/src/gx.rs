#![allow(clippy::all)]
#![allow(dead_code, non_upper_case_globals, unused_variables)]
//! GX graphics processor driver — pure-Rust port of the immediate-mode
//! subset of libogc's `gx.c` (zlib license, (c) devkitPro).
//!
//! Model: a CPU-side shadow copy of the GPU registers we program, with
//! functions that dirty-mark and a dirty-state flush at `begin`/`flush`
//! time — exactly like libogc (`__GX_SetDirtyState`).
//!
//! We do not use a command FIFO: everything streams through the
//! write-gather pipe (immediate mode), which the GP accepts when CP is in
//! its power-on state with no FIFO attached.

use crate::gctypes::{GXColor, Mtx, Mtx44};
use crate::hw::{self, WG_PIPE};
use crate::video::Video;

// ---------------------------------------------------------------------------
// wgPipe writes
// ---------------------------------------------------------------------------

/// One entry to the write-gather pipe.
#[inline(always)]
fn wg_u8(v: u8) {
    unsafe { (WG_PIPE as *mut u8).write_volatile(v) };
}
#[inline(always)]
fn wg_u32(v: u32) {
    unsafe { (WG_PIPE as *mut u32).write_volatile(v) };
}
#[inline(always)]
fn wg_f32(v: f32) {
    unsafe { (WG_PIPE as *mut f32).write_volatile(v) };
}

/// GX_LOAD_BP_REG.
#[inline(always)]
fn load_bp(v: u32) {
    wg_u8(0x61);
    wg_u32(v);
}

/// GX_LOAD_CP_REG.
#[inline(always)]
fn load_cp(reg: u8, v: u32) {
    wg_u8(0x08);
    wg_u8(reg);
    wg_u32(v);
}

/// GX_LOAD_XF_REG (single).
#[inline(always)]
fn load_xf(reg: u16, v: u32) {
    wg_u8(0x10);
    wg_u32(reg as u32);
    wg_u32(v);
}

// ---------------------------------------------------------------------------
// portable constants (mirroring ogc/gx.h values)
// ---------------------------------------------------------------------------

pub const GX_DISABLE: u8 = 0;
pub const GX_ENABLE: u8 = 1;
pub const GX_TRUE: u8 = 1;
pub const GX_FALSE: u8 = 0;

pub const GX_QUADS: u8 = 0x80;
pub const GX_TRIANGLES: u8 = 0x90;
pub const GX_TRIANGLESTRIP: u8 = 0x98;
pub const GX_TRIANGLEFAN: u8 = 0xA0;
pub const GX_LINES: u8 = 0xA8;
pub const GX_LINESTRIP: u8 = 0xB0;
pub const GX_POINTS: u8 = 0xB8;

pub const GX_VA_POS: u8 = 9;
pub const GX_VA_NRM: u8 = 10;
pub const GX_VA_CLR0: u8 = 11;
pub const GX_VA_TEX0: u8 = 13;

pub const GX_NONE: u8 = 0;
pub const GX_DIRECT: u8 = 1;
pub const GX_INDEX8: u8 = 2;
pub const GX_INDEX16: u8 = 3;

pub const GX_POS_XY: u8 = 0;
pub const GX_POS_XYZ: u8 = 1;
pub const GX_NRM_XYZ: u8 = 0;
pub const GX_CLR_RGBA: u8 = 1;
pub const GX_TEX_S: u8 = 0;
pub const GX_TEX_ST: u8 = 1;
pub const GX_U8: u8 = 0;
pub const GX_S8: u8 = 1;
pub const GX_U16: u8 = 2;
pub const GX_S16: u8 = 3;
pub const GX_F32: u8 = 4;
pub const GX_RGBA8: u8 = 5;
pub const GX_VTXFMT0: u8 = 0;
pub const VTXFMT0: u8 = GX_VTXFMT0;

pub const GX_TEVSTAGE0: u8 = 0;

pub const GX_MODULATE: u8 = 0;
pub const GX_REPLACE: u8 = 3;
pub const GX_PASSCLR: u8 = 4;

pub const GX_COLOR0A0: u8 = 4;
pub const GX_COLORNULL: u8 = 0xff;
pub const GX_TEXMAP_NULL: u8 = 0xff;
pub const GX_TEXCOORDNULL: u8 = 0xff;
pub const GX_TEXMAP0: u8 = 0;
pub const GX_TEXCOORD0: u8 = 0;

pub const GX_TG_MTX2x4: u32 = 1;
pub const GX_TG_TEX0: u32 = 4;
pub const GX_IDENTITY: u8 = 60;
pub const GX_DTTIDENTITY: u8 = 125;

pub const GX_SRC_REG: u8 = 0;
pub const GX_SRC_VTX: u8 = 1;
pub const GX_LIGHTNULL: u32 = 0x000;
pub const GX_DF_NONE: u8 = 0;
pub const GX_AF_NONE: u8 = 2;

pub const GX_PNMTX0: u8 = 0;
pub const GX_PERSPECTIVE: u8 = 0;

pub const GX_LEQUAL: u8 = 3;
pub const GX_ALWAYS: u8 = 7;

pub const GX_CULL_NONE: u8 = 0;
pub const GX_CULL_BACK: u8 = 2;

pub const GX_GM_1_0: u8 = 0;
pub const GX_PF_RGB8_Z24: u8 = 0;
pub const GX_ZC_LINEAR: u8 = 0;
pub const GX_TF_RGB565: u8 = 0x4;
pub const GX_CLAMP: u8 = 0;
pub const GX_REPEAT: u8 = 1;

// (GXEnum-ified re-exports for examples)

/// primitive enum
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Primitive {
    Quads = GX_QUADS,
    Triangles = GX_TRIANGLES,
    TriangleStrip = GX_TRIANGLESTRIP,
    TriangleFan = GX_TRIANGLEFAN,
    Lines = GX_LINES,
    LineStrip = GX_LINESTRIP,
    Points = GX_POINTS,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum WrapMode {
    Clamp = GX_CLAMP,
    Repeat = GX_REPEAT,
}

// ---------------------------------------------------------------------------
// shadow state / context
// ---------------------------------------------------------------------------

struct GxRegs {
    tev_color_env: [u32; 16],
    tev_alpha_env: [u32; 16],
    tev_ras_order: [u32; 11],
    su_ssize: [u32; 8],
    su_tsize: [u32; 8],
    gen_mode: u32,
    sci_tl: u32,
    sci_br: u32,
    lp_width: u32,
    pe_zmode: u32,
    pe_cmode0: u32,
    pe_cmode1: u32,
    pe_cntrl: u32,
    disp_copy_tl: u32,
    disp_copy_wh: u32,
    disp_copy_dst: u32,
    disp_copy_cntrl: u32,
    disp_copy_yscale: u32,
    vcd_lo: u32,
    vcd_hi: u32,
    vcd_nrms: u32,
    vcd_clear: u32,
    vat0: [u32; 8],
    vat1: [u32; 8],
    vat2: [u32; 8],
    vat_dirty: u16,
    chn_cntrl: [u32; 4],
    tex_coord_gen: [u32; 8],
    tex_coord_gen2: [u32; 8],
    tex_map_size: [u32; 8],
    tex_map_wrap: [u32; 8],
    tex_coord_manually: u32,
    mtx_idx_lo: u32,
    mtx_idx_hi: u32,
    dirty: u32,
    tex_coord_dirty: u32,
    tev_tex_coord_enable: u32,
    clear_color: GXColor,
    clear_z: u32,
}

impl GxRegs {
    const fn new() -> Self {
        GxRegs {
            tev_color_env: [0; 16],
            tev_alpha_env: [0; 16],
            tev_ras_order: [0; 11],
            su_ssize: [0; 8],
            su_tsize: [0; 8],
            gen_mode: 0,
            sci_tl: 0x20 << 24,
            sci_br: 0x21 << 24,
            lp_width: 0x22 << 24,
            pe_zmode: 0x40 << 24,
            pe_cmode0: 0x41 << 24,
            pe_cmode1: 0x42 << 24,
            pe_cntrl: 0x43 << 24,
            disp_copy_tl: 0x49 << 24,
            disp_copy_wh: 0x4a << 24,
            disp_copy_dst: 0x4d << 24,
            disp_copy_cntrl: 0x52 << 24,
            disp_copy_yscale: 0x4e << 24,
            vcd_lo: 0,
            vcd_hi: 0,
            vcd_nrms: 0,
            vcd_clear: 0,
            vat0: [0; 8],
            vat1: [0; 8],
            vat2: [0; 8],
            vat_dirty: 0,
            chn_cntrl: [0; 4],
            tex_coord_gen: [0; 8],
            tex_coord_gen2: [0; 8],
            tex_map_size: [0; 8],
            tex_map_wrap: [0; 8],
            tex_coord_manually: 0,
            mtx_idx_lo: 0,
            mtx_idx_hi: 0,
            dirty: 0,
            tex_coord_dirty: 0,
            tev_tex_coord_enable: 0,
            clear_color: GXColor::BLACK,
            clear_z: 0x00ff_ffff,
        }
    }
}

static mut GX: *mut GxRegs = core::ptr::null_mut();

#[inline(always)]
fn gx() -> &'static mut GxRegs {
    unsafe { &mut *GX }
}

/// GX context returned by `Gc::into_gx`.
pub struct Context {
    _private: (),
}

impl Context {
    pub fn set_clear_color(&self, color: GXColor) {
        gx().clear_color = color;
        gx().clear_z = 0x00ff_ffff;
        set_copy_clear(color, 0x00ff_ffff);
    }

    /// Load a perspective projection (perspective matrix + z range).
    pub fn load_perspective(&self, proj: &Mtx44) {
        load_projection_mtx(proj, GX_PERSPECTIVE)
    }

    pub fn load_model_view(&self, mv: &Mtx) {
        load_pos_mtx_imm(mv, GX_PNMTX0 as u32);
        set_current_mtx(GX_PNMTX0 as u32);
    }

    /// Per-frame setup: invalidate vertex cache (like the libogc examples).
    pub fn begin_frame(&self) {
        unsafe {
            (WG_PIPE as *mut u8).write_volatile(0x48);
        }
    }

    /// Block until the GPU has consumed all issued commands.
    pub fn draw_done(&self) {
        flush();
    }

    /// Copy the finished frame into the XFB and wait for the next
    /// vertical retrace.
    pub fn end_frame(&self) {
        copy_disp();
        crate::video::wait_vsync();
    }
}

/// Flush the pipe with 8 zero bytes (GX_Flush without fifo).
#[inline(always)]
pub fn flush() {
    unsafe {
        let p = WG_PIPE as *mut u32;
        for _ in 0..8 {
            p.write_volatile(0);
        }
        hw::sync();
    }
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

pub(crate) fn init(video: &Video) -> Context {
    unsafe {
        GX = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(GxRegs::new()));
    }

    // "GX_Init" baseline: set up magic regs written by libogc __gx_init.
    // Divider constants (bus clock = 162 MHz):
    let divis_res = 162_000_000u32 / 500;
    let res2 = divis_res / 4224;
    flush();
    load_bp(0x4600_0000 | (res2 | 0x0200));

    // default BP/CP writes from GX_Init
    load_cp(0x20, 0);
    load_xf(0x1006, 0);
    load_bp(0x2300_0000);
    load_bp(0x2400_0000);
    load_bp(0x6700_0000);

    // "Rev bits"
    for i in 0..8u8 {
        // vat1[i] = 0x80000000 written to both arrays
        gx().vat1[i as usize] = 0x8000_0000;
        load_cp(0x80 + i, 0x8000_0000);
    }
    load_xf(0x1000, 0x3f);
    load_xf(0x1012, 0x01);
    load_bp(0x5800_000f);

    // __GX_InitGX subset (sane defaults our examples use)
    setup_defaults(video);

    Context { _private: () }
}

/// The "known-good baseline" of `__GX_InitGX` in libogc, pruned to what we
/// use: 1 channel w/ vertex color, 1 tev stage PASSCLR, no textures...
/// (folded into the config_vertex_color_pipeline below).
fn setup_defaults(video: &Video) {
    let mode = video.mode();

    // viewport/scissor to full EFB
    let w = f32::from(mode.fbWidth);
    let h = f32::from(mode.efbHeight);
    set_viewport(0.0, 0.0, w, h, 0.0, 1.0);
    set_scissor(0, 0, mode.fbWidth as u32, mode.efbHeight as u32);

    set_copy_clear(GXColor::BLACK, 0x00ff_ffff);

    // disp copy (EFB -> XFB)
    set_disp_copy_src(0, 0, mode.fbWidth, mode.efbHeight);
    set_disp_copy_dst(mode.fbWidth, mode.xfbHeight);
    let yscale = get_yscale_factor(mode.efbHeight, mode.xfbHeight);
    set_disp_copy_yscale(yscale);
    set_copy_filter(mode.aa, &mode.sample_pattern, 1, &mode.vfilter);

    // disable DPMS clipping? libogc culls BACK faces by default (0x00000101 = flat); set zmode/color update
    set_z_mode(true, GX_LEQUAL, true);
    set_color_update(true);
    set_cull_mode(GX_CULL_BACK);

    // identity proj/model for now; examples will overwrite
    let ident: Mtx = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
    load_pos_mtx_imm(&ident, GX_PNMTX0 as u32);
    set_current_mtx(GX_PNMTX0 as u32);

    // default channel: vertex color passing through a single PASSCLR stage
    set_num_chans(1);
    set_num_tex_gens(0);
    set_tev_order(GX_TEVSTAGE0, GX_TEXCOORDNULL, GX_TEXMAP_NULL, GX_COLOR0A0);
    set_tev_op(GX_TEVSTAGE0, GX_PASSCLR);
    set_num_tev_stages(1);
    set_chan_ctrl_color0a0();

    clear_vtx_desc();
    set_vtx_desc(GX_VA_POS, GX_DIRECT);
    set_vtx_desc(GX_VA_CLR0, GX_DIRECT);
    set_vtx_attr_fmt(0, GX_VA_POS, GX_POS_XYZ, GX_F32, 0);
    set_vtx_attr_fmt(0, GX_VA_CLR0, GX_CLR_RGBA, GX_RGBA8, 0);
}

// ---------------------------------------------------------------------------
// viewport / scissor / copy (libogc ports)
// ---------------------------------------------------------------------------

pub fn set_viewport(x: f32, y: f32, w: f32, h: f32, n: f32, f: f32) {
    let g = gx();
    // SetViewportJitter with field=1
    let x0 = w * 0.5;
    let y0 = -h * 0.5;
    let x1 = (x + w * 0.5) + 342.0;
    let y1 = (y + h * 0.5) + 342.0;
    let nz = n * 16_777_215.0;
    let fz = f * 16_777_215.0;
    let zd = fz - nz;
    unsafe {
        // XF regs 0x101a..0x101f (6 floats)
        (WG_PIPE as *mut u8).write_volatile(0x10);
        (WG_PIPE as *mut u32).write_volatile((6 - 1) << 16 | 0x101a);
        (WG_PIPE as *mut f32).write_volatile(x0);
        (WG_PIPE as *mut f32).write_volatile(y0);
        (WG_PIPE as *mut f32).write_volatile(zd);
        (WG_PIPE as *mut f32).write_volatile(x1);
        (WG_PIPE as *mut f32).write_volatile(y1);
        (WG_PIPE as *mut f32).write_volatile(fz);
    }
}

pub fn set_scissor(x: u32, y: u32, w: u32, h: u32) {
    let g = gx();
    let xo = x + 0x156;
    let yo = y + 0x156;
    let nwd = xo + (w - 1);
    let nht = yo + (h - 1);
    g.sci_tl = (g.sci_tl & !0x7ff) | (yo & 0x7ff);
    g.sci_tl = (g.sci_tl & !0x7ff_000) | hw::shiftl(xo, 12, 11);
    g.sci_br = (g.sci_br & !0xfff) | (nht & 0xfff);
    g.sci_br = (g.sci_br & !0x7ff_000) | hw::shiftl(nwd, 12, 11);
    load_bp(g.sci_tl);
    load_bp(g.sci_br);
}

pub fn set_disp_copy_src(x: u16, y: u16, w: u16, h: u16) {
    let g = gx();
    let xy = |a: u16, b: u16| ((b as u32) << 10) | a as u32;
    g.disp_copy_tl = (g.disp_copy_tl & !0x00ff_ffff) | xy(x, y);
    g.disp_copy_wh = (g.disp_copy_wh & !0x00ff_ffff) | xy(w - 1, h - 1);
}

pub fn set_disp_copy_dst(w: u16, _h: u16) {
    let g = gx();
    g.disp_copy_dst = (g.disp_copy_dst & !0x3ff) | hw::shiftr(u32::from(w), 4, 10);
}

pub fn get_yscale_factor(efb_h: u16, xfb_h: u16) -> f32 {
    // simple version of libogc GX_GetYScaleFactor without the iterative fix;
    // our modes always scale 1:1 for 480/576 heights
    let _ = (efb_h, xfb_h);
    efb_h as f32 / xfb_h as f32
}

pub fn set_disp_copy_yscale(scale: f32) {
    let g = gx();
    let yscale = ((256.0 / scale) as u32) & 0x1ff;
    load_bp(0x4e00_0000 | yscale);
    g.disp_copy_cntrl = (g.disp_copy_cntrl & !0x400)
        | hw::shiftl((256u32.wrapping_sub(yscale) > 0) as u32, 10, 1);
    // note: returned "xfb lines" not needed: we always frame 1:1
}

pub fn set_copy_clear(c: GXColor, z: u32) {
    load_bp(0x4f00_0000 | (u32::from(c.a) << 8) | u32::from(c.r));
    load_bp(0x5000_0000 | (u32::from(c.g) << 8) | u32::from(c.b));
    load_bp(0x5100_0000 | (z & 0x00ff_ffff));
}

pub fn set_copy_filter(aa: u8, _pattern: &[[u8; 2]; 12], _vf: u8, _vfilter: &[u8; 7]) {
    // Replicating setCopyFilter with vf=1 and the default vfilter.
    // sample_pattern AA=0 -> fixed pattern 0x666666; vfilter default is the
    // usual one extracted from libogc's TVNtsc480IntDf.
    let reg01 = 0x0166_6666u32;
    let reg02 = 0x0266_6666;
    let reg03 = 0x0366_6666;
    let reg04 = 0x0466_6666;
    load_bp(reg01);
    load_bp(reg02);
    load_bp(reg03);
    load_bp(reg04);

    // vf = 1: default vfilter [8, 8, 10, 12, 10, 8, 8]
    let vfilter = _vfilter;
    let mut reg53: u32 = 0x5300_0000 | u32::from(vfilter[0] & 0x3f);
    reg53 = (reg53 & !0xfc0) | hw::shiftl(u32::from(vfilter[1]), 6, 6);
    reg53 = (reg53 & !0x3f000) | hw::shiftl(u32::from(vfilter[2]), 12, 6);
    reg53 = (reg53 & !0xfc0_000) | hw::shiftl(u32::from(vfilter[3]), 18, 6);
    let mut reg54: u32 = 0x5400_0000 | u32::from(vfilter[4] & 0x3f);
    reg54 = (reg54 & !0xfc0) | hw::shiftl(u32::from(vfilter[5]), 6, 6);
    reg54 = (reg54 & !0x3f000) | hw::shiftl(u32::from(vfilter[6]), 12, 6);
    load_bp(reg53);
    load_bp(reg54);
}

// ---------------------------------------------------------------------------
// pixel engine / tev
// ---------------------------------------------------------------------------

pub fn set_z_mode(enable: bool, func: u8, update: bool) {
    let g = gx();
    g.pe_zmode = (g.pe_zmode & !0x1) | (enable as u32);
    g.pe_zmode = (g.pe_zmode & !0xe) | hw::shiftl(u32::from(func), 1, 3);
    g.pe_zmode = (g.pe_zmode & !0x10) | hw::shiftl(update as u32, 4, 1);
    load_bp(g.pe_zmode);
}

pub fn set_color_update(enable: bool) {
    let g = gx();
    g.pe_cmode0 = (g.pe_cmode0 & !0x8) | hw::shiftl(enable as u32, 3, 1);
    load_bp(g.pe_cmode0);
}

pub fn set_cull_mode(mode: u8) {
    let g = gx();
    let cm2hw = [0u8, 2, 1, 3];
    g.gen_mode = (g.gen_mode & !0xC000) | hw::shiftl(u32::from(cm2hw[mode as usize & 3]), 14, 2);
    g.dirty |= 0x0004;
}

pub fn set_num_chans(num: u8) {
    let g = gx();
    g.gen_mode = (g.gen_mode & !0x70) | hw::shiftl(u32::from(num), 4, 3);
    g.dirty |= 0x0100_0004;
}

pub fn set_num_tex_gens(num: u32) {
    let g = gx();
    g.gen_mode = (g.gen_mode & !0xf) | (num & 0xf);
    g.dirty |= 0x0200_0004;
}

pub fn set_num_tev_stages(num: u8) {
    let g = gx();
    g.gen_mode = (g.gen_mode & !0x3C00) | hw::shiftl(u32::from(num - 1), 10, 4);
    g.dirty |= 0x0004;
}

pub fn set_chan_ctrl_color0a0() {
    set_chan_ctrl(
        GX_COLOR0A0,
        GX_DISABLE,
        GX_SRC_REG,
        GX_SRC_VTX,
        GX_LIGHTNULL,
        GX_DF_NONE,
        GX_AF_NONE,
    );
}

pub fn set_chan_ctrl(channel: u8, enable: u8, ambsrc: u8, matsrc: u8, litmask: u32, diff_fn: u8, attn_fn: u8) {
    // libogc GX_SetChanCtrl exact port (simplified: no GX_DF_SIGNED translation)
    let g = gx();
    let difffn = if attn_fn == GX_AF_NONE { diff_fn } else { GX_DF_NONE };
    let val = (u32::from(matsrc & 1))
        | hw::shiftl(u32::from(enable), 1, 1)
        | hw::shiftl(litmask, 2, 4)
        | hw::shiftl(u32::from(ambsrc), 6, 1)
        | hw::shiftl(u32::from(difffn), 7, 2)
        | hw::shiftl(((GX_AF_NONE.wrapping_sub(attn_fn)) > 0) as u32, 9, 1)
        | hw::shiftl((attn_fn > 0) as u32, 10, 1)
        | hw::shiftl(hw::shiftr(litmask, 4, 4), 11, 4);
    let reg = (channel & 3) as usize;
    g.chn_cntrl[reg] = val;
    g.dirty |= 0x1000 << reg;
    if channel == GX_COLOR0A0 {
        g.chn_cntrl[2] = val;
        g.dirty |= 0x5000;
    } else {
        g.chn_cntrl[3] = val;
        g.dirty |= 0xa000;
    }
}

pub fn set_tev_order(stage: u8, texcoord: u8, texmap: u8, color: u8) {
    let g = gx();
    let reg = 3usize + hw::shiftr(u32::from(stage), 1, 3) as usize;
    g.tex_map_size[0] = g.tex_map_size[0]; // noop keep
    // track last used texmap for SU regs pass-through
    // (we don't use the dynamic SU-table machinery)
    let texm = if texmap == GX_TEXMAP_NULL { 0 } else { (texmap & 7) as u32 };
    let texc = if texcoord == GX_TEXCOORDNULL { 0 } else { (texcoord & 7) as u32 };

    const GXTEVCOLID: [u32; 9] = [0, 1, 0, 1, 0, 1, 7, 5, 6];
    let colid = if color == GX_COLORNULL { 5 } else { GXTEVCOLID[color as usize & 8] };

    if stage & 1 != 0 {
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x7000) | hw::shiftl(texm, 12, 3);
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x3_8000) | hw::shiftl(texc, 15, 3);
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x38_0000) | hw::shiftl(colid, 19, 3);
        let tmp = if texmap == GX_TEXMAP_NULL { 0 } else { 1 };
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x4_0000) | hw::shiftl(tmp, 18, 1);
    } else {
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x7) | (texm & 0x7);
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x38) | hw::shiftl(texc, 3, 3);
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x380) | hw::shiftl(colid, 7, 3);
        let tmp = if texmap == GX_TEXMAP_NULL { 0 } else { 1 };
        g.tev_ras_order[reg] = (g.tev_ras_order[reg] & !0x40) | hw::shiftl(tmp, 6, 1);
    }
    load_bp(g.tev_ras_order[reg]);
    g.dirty |= 0x0001;
}

pub fn set_tev_op(stage: u8, mode: u8) {
    let (defcolor, defalpha);
    if stage == GX_TEVSTAGE0 {
        defcolor = 0x0f; // GX_CC_RASC
        defalpha = 0x07; // GX_CA_RASA
    } else {
        defcolor = 0x00; // GX_CC_CPREV
        defalpha = 0x00; // GX_CA_APREV
    }

    match mode {
        GX_MODULATE => {
            set_tev_color_in(stage, 0, 0x0e, defcolor, 0);
            set_tev_alpha_in(stage, 0, 0x07, defalpha, 0);
        }
        GX_REPLACE => {
            set_tev_color_in(stage, 0, 0, 0, 0x0e);
            set_tev_alpha_in(stage, 0, 0, 0, 0x07);
        }
        GX_PASSCLR => {
            set_tev_color_in(stage, 0, 0, 0, defcolor);
            set_tev_alpha_in(stage, 0, 0, 0, defalpha);
        }
        _ => {}
    }
    // TEV ops: add, no bias, *1, clamped, output to PREV (standard MIN/ADD set)
    set_tev_color_op_simple(stage);
    set_tev_alpha_op_simple(stage);
}

fn set_tev_color_in(stage: u8, a: u8, b: u8, c: u8, d: u8) {
    let g = gx();
    let reg = stage as usize & 0xf;
    let id = 0xc0u32 + 2 * reg as u32;
    let mut v = (g.tev_color_env[reg] & !0xff00_0000) | (id << 24);
    v = (v & !0xF000) | hw::shiftl(u32::from(a), 12, 4);
    v = (v & !0xF00) | hw::shiftl(u32::from(b), 8, 4);
    v = (v & !0xF0) | hw::shiftl(u32::from(c), 4, 4);
    v = (v & !0xF) | u32::from(d & 0xf);
    g.tev_color_env[reg] = v;
    load_bp(v);
}

fn set_tev_alpha_in(stage: u8, a: u8, b: u8, c: u8, d: u8) {
    let g = gx();
    let reg = stage as usize & 0xf;
    let id = 0xc1u32 + 2 * reg as u32;
    let mut v = (g.tev_alpha_env[reg] & !0xff00_0000) | (id << 24);
    v = (v & !0xE000) | hw::shiftl(u32::from(a), 13, 3);
    v = (v & !0x1C00) | hw::shiftl(u32::from(b), 10, 3);
    v = (v & !0x380) | hw::shiftl(u32::from(c), 7, 3);
    v = (v & !0x70) | hw::shiftl(u32::from(d), 4, 3);
    g.tev_alpha_env[reg] = v;
    load_bp(v);
}

fn set_tev_color_op_simple(stage: u8) {
    let g = gx();
    let reg = stage as usize & 0xf;
    let id = 0xc0u32 + 2 * reg as u32;
    let mut v = (g.tev_color_env[reg] & !0xff00_0000) | (id << 24);
    // tevop=add(0), bias=0, scale=1(0? libogc GX_CS_SCALE_1=?), clamp=1, reg=0
    v = (v & !0x40_000) | 0; // add op
    v = (v & !0x30_0000) | hw::shiftl(0, 20, 2); // GX_CS_SCALE_1 == 0 in libogc
    v = (v & !0x3_0000) | 0; // bias zero
    v = (v & !0x8_0000) | hw::shiftl(1, 19, 1); // clamp
    v = (v & !0xC0_0000) | hw::shiftl(0, 22, 2); // TEVPREV
    g.tev_color_env[reg] = v;
    load_bp(v);
}

fn set_tev_alpha_op_simple(stage: u8) {
    let g = gx();
    let reg = stage as usize & 0xf;
    let id = 0xc1u32 + 2 * reg as u32;
    let mut v = (g.tev_alpha_env[reg] & !0xff00_0000) | (id << 24);
    v = (v & !0x40_000) | 0;
    v = (v & !0x30_0000) | hw::shiftl(0, 20, 2);
    v = (v & !0x3_0000) | 0;
    v = (v & !0x8_0000) | hw::shiftl(1, 19, 1);
    v = (v & !0xC0_0000) | hw::shiftl(0, 22, 2);
    g.tev_alpha_env[reg] = v;
    load_bp(v);
}

/// One-shot pipeline state for the unlit "vertex color" case used by most
/// examples: 1 color channel taken from the vertex, 1 TEV stage PASSCLR.
pub fn config_vertex_color_pipeline() {
    set_num_chans(1);
    set_num_tex_gens(0);
    set_num_tev_stages(1);
    set_chan_ctrl(
        GX_COLOR0A0, GX_DISABLE, GX_SRC_REG, GX_SRC_VTX, GX_LIGHTNULL, GX_DF_NONE, GX_AF_NONE,
    );
    set_tev_order(GX_TEVSTAGE0, GX_TEXCOORDNULL, GX_TEXMAP_NULL, GX_COLOR0A0);
    set_tev_op(GX_TEVSTAGE0, GX_PASSCLR);
}

/// One-shot pipeline state for "texture modulated by vertex color".
pub fn config_textured_pipeline() {
    set_num_chans(1);
    set_num_tex_gens(1);
    set_num_tev_stages(1);
    set_chan_ctrl(
        GX_COLOR0A0, GX_DISABLE, GX_SRC_REG, GX_SRC_VTX, GX_LIGHTNULL, GX_DF_NONE, GX_AF_NONE,
    );
    // texcoord 0 from TEX0 attr, identity matrix
    let g = gx();
    g.tex_coord_manually &= !1;
    // SUTexReg writes (texture sizing) happen as part of tex loading.
    set_tex_coord_gen(0);
    set_tev_order(GX_TEVSTAGE0, GX_TEXCOORD0, GX_TEXMAP0, GX_COLOR0A0);
    set_tev_op(GX_TEVSTAGE0, GX_MODULATE);
}

pub fn set_tex_coord_gen(coord: u8) {
    // GX_TG_MTX2x4, source = GX_TG_TEX0, identity DTT
    let g = gx();
    let texcoords: u32 = 0x0 | (1 << 2) | (5 << 7); // stq=1, vtxrow=5 (TEX0)
    g.tex_coord_gen[(coord & 7) as usize] = texcoords;
    g.tex_coord_gen2[(coord & 7) as usize] = 0; // normalize off, DTT IDENTITY
    g.dirty |= 0x0001_0000 << coord;
}

// ---------------------------------------------------------------------------
// vertex desc/format (libogc's __SETVCDATTR/__SETVCDFMT ports)
// ---------------------------------------------------------------------------

pub fn clear_vtx_desc() {
    let g = gx();
    g.vcd_nrms = 0;
    g.vcd_clear = (g.vcd_clear & !0x0600) | 0x0200;
    g.vcd_lo = 0;
    g.vcd_hi = 0;
    g.dirty |= 0x0008;
}

/// Convenience: mark `attr` as directly streamed.
pub fn set_vtx_desc_direct(attr: u8) {
    set_vtx_desc(attr, GX_DIRECT);
}

pub fn set_vtx_desc(attr: u8, ty: u8) {
    let g = gx();
    let ty32 = ty as u32;
    match attr {
        // PTNMTXIDX/TEXnMTXIDX ignored (we don't use them)
        GX_VA_POS => {
            g.vcd_lo = (g.vcd_lo & !0x600) | hw::shiftl(ty32, 9, 2);
        }
        GX_VA_NRM => {
            g.vcd_lo = (g.vcd_lo & !0x1800) | hw::shiftl(ty32, 11, 2);
            g.vcd_nrms = if ty == GX_NONE { 0 } else { 1 };
        }
        GX_VA_CLR0 => {
            g.vcd_lo = (g.vcd_lo & !0x6000) | hw::shiftl(ty32, 13, 2);
        }
        GX_VA_TEX0 => {
            g.vcd_hi = (g.vcd_hi & !0x3) | (ty32 & 0x3);
        }
        _ => {}
    }
    g.dirty |= 0x0008;
}

pub fn set_vtx_attr_fmt(vtxfmt: u8, attr: u8, comptype: u8, compfmt: u8, frac: u8) {
    let g = gx();
    let vat = (vtxfmt & 7) as usize;
    let (ct, cf) = (comptype as u32, compfmt as u32);

    match attr {
        GX_VA_POS if (comptype == GX_POS_XY || comptype == GX_POS_XYZ) && compfmt <= GX_F32 => {
            g.vat0[vat] = (g.vat0[vat] & !0x1) | (ct & 1);
            g.vat0[vat] = (g.vat0[vat] & !0xe) | hw::shiftl(cf, 1, 3);
            g.vat0[vat] = (g.vat0[vat] & !0x1f0) | hw::shiftl(u32::from(frac), 4, 5);
            if frac != 0 {
                g.vat0[vat] |= 0x4000_0000;
            }
        }
        GX_VA_NRM => {
            g.vat0[vat] &= !0x200;
            g.vat0[vat] = (g.vat0[vat] & !0x1C00) | hw::shiftl(cf, 10, 3);
            g.vat0[vat] &= !0x8000_0000;
        }
        GX_VA_CLR0 => {
            g.vat0[vat] = (g.vat0[vat] & !0x2000) | hw::shiftl(ct, 13, 1);
            g.vat0[vat] = (g.vat0[vat] & !0x1C000) | hw::shiftl(cf, 14, 3);
        }
        GX_VA_TEX0 => {
            g.vat0[vat] = (g.vat0[vat] & !0x20_0000) | hw::shiftl(ct, 21, 1);
            g.vat0[vat] = (g.vat0[vat] & !0x1C0_0000) | hw::shiftl(cf, 22, 3);
            g.vat0[vat] = (g.vat0[vat] & !0x3E00_0000) | hw::shiftl(u32::from(frac), 25, 5);
            if frac != 0 {
                g.vat0[vat] |= 0x4000_0000;
            }
        }
        _ => {}
    }
    g.vat_dirty |= 1 << vat;
    g.dirty |= 0x0010;
}

// ---------------------------------------------------------------------------
// matrices
// ---------------------------------------------------------------------------

pub fn load_projection_mtx(mt: &Mtx44, ty: u8) {
    unsafe {
        let t = [[mt[0][0], mt[0][2], mt[1][1], mt[1][2], mt[2][2], mt[2][3], ty as f32]];
        let mut tf = [0f32; 7];
        tf[0] = t[0][0];
        tf[1] = t[0][1];
        tf[2] = t[0][2];
        tf[3] = t[0][3];
        tf[4] = t[0][4];
        tf[5] = t[0][5];
        tf[6] = u32::from(ty) as f32;
        // XF load 7 floats
        (WG_PIPE as *mut u8).write_volatile(0x10);
        (WG_PIPE as *mut u32).write_volatile(((7 - 1) << 16) | 0x1020);
        for v in tf {
            (WG_PIPE as *mut f32).write_volatile(v);
        }
    }
}

pub fn load_pos_mtx_imm(mt: &Mtx, pnidx: u32) {
    unsafe {
        (WG_PIPE as *mut u8).write_volatile(0x10);
        let addr = (pnidx & 0x3f) << 2;
        (WG_PIPE as *mut u32).write_volatile(((12 - 1) << 16) | addr);
        for r in mt.iter() {
            for &v in r.iter() {
                (WG_PIPE as *mut f32).write_volatile(v);
            }
        }
    }
}

pub fn set_current_mtx(mtx: u32) {
    let g = gx();
    g.mtx_idx_lo = (g.mtx_idx_lo & !0x3f) | (mtx & 0x3f);
    g.dirty |= 0x0400_0000;
}

// ---------------------------------------------------------------------------
// immediate-mode render
// ---------------------------------------------------------------------------

/// GX_Begin; flushes dirty state first.
pub fn begin(primitive: Primitive, vtxfmt: u8, count: u16) {
    set_dirty_state();
    unsafe {
        (WG_PIPE as *mut u8).write_volatile(primitive as u8 | (vtxfmt & 7));
        (WG_PIPE as *mut u16).write_volatile(count.to_be());
    }
}

#[inline(always)]
pub fn position3f32(x: f32, y: f32, z: f32) {
    unsafe {
        (WG_PIPE as *mut f32).write_volatile(x);
        (WG_PIPE as *mut f32).write_volatile(y);
        (WG_PIPE as *mut f32).write_volatile(z);
    }
}

#[inline(always)]
pub fn color4u8(r: u8, g: u8, b: u8, a: u8) {
    unsafe {
        (WG_PIPE as *mut u8).write_volatile(r);
        (WG_PIPE as *mut u8).write_volatile(g);
        (WG_PIPE as *mut u8).write_volatile(b);
        (WG_PIPE as *mut u8).write_volatile(a);
    }
}

#[inline(always)]
pub fn color(c: GXColor) {
    color4u8(c.r, c.g, c.b, c.a)
}

#[inline(always)]
pub fn texcoord2f32(s: f32, t: f32) {
    unsafe {
        (WG_PIPE as *mut f32).write_volatile(s);
        (WG_PIPE as *mut f32).write_volatile(t);
    }
}

// inventory: set_disp_copy_src/dst write into state; copy_disp issues the copy.
fn copy_disp() {
    let g = gx();
    // clear path: write peZMode/peCMode0 cleared, then copy regs
    load_bp((g.pe_zmode & !0xf) | 0xf);
    load_bp(g.pe_cmode0 & !0x3);

    load_bp(g.disp_copy_tl);
    load_bp(g.disp_copy_wh);
    load_bp(g.disp_copy_dst);

    let fb = crate::video::current_xfb();
    let v = 0x4b00_0000 | (hw::shiftr(hw::virt_to_phys(fb), 5, 24));
    load_bp(v);
    let mut cntrl = (0x52u32 << 24) | (g.disp_copy_cntrl & 0x00ff_ffff);
    cntrl = (cntrl & !0x800) | hw::shiftl(1, 11, 1); // clear=1
    cntrl = (cntrl & !0x4000) | 0x4000;
    load_bp(cntrl);

    // restore
    load_bp(g.pe_zmode);
    load_bp(g.pe_cmode0);
}

/// __GX_SetDirtyState reduced to the pieces our examples actually mutate.
fn set_dirty_state() {
    let g = gx();
    let dirty = g.dirty;

    if dirty & 0x0008 != 0 {
        // VCD
        load_cp(0x50, g.vcd_lo);
        load_cp(0x60, g.vcd_hi);
        // xf vtx specs (nrms/cols/texs)
        let cols = ((g.vcd_lo & 0x6000) != 0) as u32;
        let nrms = g.vcd_nrms.min(1);
        let texs = ((g.vcd_hi & 0x3) != 0) as u32;
        let vtxspecs = hw::shiftl(texs, 4, 4) | hw::shiftl(nrms, 2, 2) | (cols & 0x3);
        load_xf(0x1008, vtxspecs);
    }
    if dirty & 0x0010 != 0 {
        for i in 0..8 {
            if g.vat_dirty & (1 << i) != 0 {
                load_cp(0x70 + i as u8, g.vat0[i]);
                load_cp(0x80 + i as u8, g.vat1[i]);
                load_cp(0x90 + i as u8, g.vat2[i]);
            }
        }
        g.vat_dirty = 0;
    }
    if dirty & 0x0004 != 0 {
        load_bp(g.gen_mode);
    }
    if dirty & 0x0100_0000 != 0 {
        load_xf(0x1009, hw::shiftr(g.gen_mode, 4, 3));
    }
    // tex coord gens
    if dirty & 0x02ff_0000 != 0 {
        if dirty & 0x0200_0000 != 0 {
            load_xf(0x103f, g.gen_mode & 0xf);
        }
        let mut mask = hw::shiftr(dirty, 16, 8);
        let mut texcoord: u16 = 0x1040;
        let mut i = 0usize;
        while mask != 0 {
            if mask & 1 != 0 {
                load_xf(texcoord, g.tex_coord_gen[i]);
                load_xf(texcoord + 0x10, g.tex_coord_gen2[i]);
            }
            mask >>= 1;
            texcoord += 1;
            i += 1;
        }
    }
    if dirty & 0x0000_f000 != 0 {
        // channel enable control regs
        let mut mask = hw::shiftr(dirty, 12, 4);
        let mut chan = 0x100eu16;
        let mut i = 0usize;
        while mask != 0 {
            if mask & 1 != 0 {
                load_xf(chan, g.chn_cntrl[i]);
            }
            mask >>= 1;
            chan += 1;
            i += 1;
        }
    }
    if dirty & 0x0400_0000 != 0 {
        load_cp(0x30, g.mtx_idx_lo);
        load_xf(0x1018, g.mtx_idx_lo);
        load_cp(0x40, g.mtx_idx_hi);
        load_xf(0x1019, g.mtx_idx_hi);
    }
    g.dirty = 0;
}

// ---------------------------------------------------------------------------
// textures
// ---------------------------------------------------------------------------

/// Opaque texture object (shadow of GXTexObj).
#[repr(C, align(32))]
pub struct TexObj {
    tex_filt: u32,
    tex_lod: u32,
    tex_size: u32,
    tex_maddr: u32,
    tex_data: u32,
    tex_fmt: u32,
    tex_tlut: u32,
    tex_tile_type: u8,
    tex_tile_cnt: u8,
    tex_flag: u16,
    _pad: [u8; 12],
}

/// A resident RGB565 texture + its storage.
pub struct Texture {
    obj: TexObj,
    data: *mut u8,
    len: usize,
    width: u16,
    height: u16,
}

impl Texture {
    pub fn from_rgb565(width: u16, height: u16, pixels: &[u16], wrap: WrapMode) -> Texture {
        assert_eq!(
            pixels.len(),
            usize::from(width) * usize::from(height),
            "pixel buffer size mismatch"
        );
        assert!(width % 4 == 0 && height % 4 == 0 && width <= 1024 && height <= 1024);

        // swizzle into 4x4 tiles
        let len = pixels.len() * 2;
        let layout = core::alloc::Layout::from_size_align(len, 32).unwrap();
        let data = unsafe { alloc::alloc::alloc(layout) };
        assert!(!data.is_null());
        let tiled = data.cast::<u16>();
        for ty in (0..height).step_by(4) {
            for tx in (0..width).step_by(4) {
                for iy in 0..4u16 {
                    for ix in 0..4u16 {
                        let src = usize::from(ty + iy) * usize::from(width) + usize::from(tx + ix);
                        let dst = usize::from(ty) * usize::from(width)
                            + usize::from(tx) * 4
                            + usize::from(iy) * 4
                            + usize::from(ix);
                        unsafe { tiled.add(dst).write(pixels[src]) };
                    }
                }
            }
        }
        unsafe { hw::dc_flush_range(data, len) };

        let mut obj = TexObj {
            tex_filt: 0,
            tex_lod: 0,
            tex_size: 0,
            tex_maddr: 0,
            tex_data: 0,
            tex_fmt: 0,
            tex_tlut: 0,
            tex_tile_type: 0,
            tex_tile_cnt: 0,
            tex_flag: 0,
            _pad: [0; 12],
        };

        let wrap_v = wrap as u32;
        obj.tex_filt = (obj.tex_filt & !0x03) | (wrap_v & 3);
        obj.tex_filt = (obj.tex_filt & !0x0c) | hw::shiftl(wrap_v, 2, 2);
        obj.tex_filt = (obj.tex_filt & !0x10) | 0x10;
        // no mipmap: GX_TF_RGB565 -> "linear" filtering disabled per libogc
        obj.tex_filt = (obj.tex_filt & !0xE0) | 0x0080;
        obj.tex_fmt = u32::from(GX_TF_RGB565);
        obj.tex_size = (obj.tex_size & !0x3ff) | (u32::from(width - 1) & 0x3ff);
        obj.tex_size = (obj.tex_size & !0xFFC00)
            | hw::shiftl(u32::from(height - 1), 10, 10);
        obj.tex_size = (obj.tex_size & !0xF0_0000) | hw::shiftl(u32::from(GX_TF_RGB565), 20, 4);
        obj.tex_maddr = (obj.tex_maddr & !0x00ff_ffff)
            | hw::shiftr(hw::virt_to_phys(data), 5, 24);
        obj.tex_tile_type = 2; // RGB565 path
        obj.tex_flag = 0x01; // "dirty"-shadow flag like libogc

        Texture { obj, data, len, width, height }
    }

    /// Load (bind) this texture as TEXMAP0.
    pub fn bind(&self) {
        let o = &self.obj;
        // BP ids for map 0:
        // _gxtexmode0ids[0]=0x80, mode1=0x84, img0=0x88, img3=0x94 (sub=1)
        load_bp((o.tex_filt & !0xff00_0000) | 0x8000_0000);
        load_bp((o.tex_lod & !0xff00_0000) | 0x8400_0000);
        load_bp((o.tex_size & !0xff00_0000) | 0x8800_0000);
        // tmem even/odd for map 0 (img1=0x8c, img2=0x90): our texobj uses
        // precomputed region offsets; mirror libogc's default region cb:
        load_bp(0x8c00_0000 | (((0u32 >> 5) & 0x7fff)) | (3 << 15) | (3 << 18));
        load_bp(0x9000_0000 | (((0x8000u32 >> 5) & 0x7fff)) | (3 << 15) | (3 << 18));
        load_bp((o.tex_maddr & !0xff00_0000) | 0x9400_0000);
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        let layout = core::alloc::Layout::from_size_align(self.len, 32).unwrap();
        unsafe { alloc::alloc::dealloc(self.data, layout) };
    }
}
