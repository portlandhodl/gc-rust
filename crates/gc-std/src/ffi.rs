//! Raw FFI bindings to libogc (devkitPro) and newlib.
//!
//! Everything in here maps 1:1 to the C headers in
//! `$DEVKITPRO/libogc/include`. Safe wrappers live in the parent modules;
//! prefer those over calling these directly.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use core::ffi::{c_char, c_int, c_void};

pub type BOOL = c_int;

/// `GXRModeObj` from `ogc/video_types.h` (via `ogc/gx_struct.h`).
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GXRModeObj {
    pub viTVMode: u32,
    pub fbWidth: u16,
    pub efbHeight: u16,
    pub xfbHeight: u16,
    pub viXOrigin: u16,
    pub viYOrigin: u16,
    pub viWidth: u16,
    pub viHeight: u16,
    pub xfbMode: u32,
    pub field_rendering: u8,
    pub aa: u8,
    pub sample_pattern: [[u8; 2]; 12],
    pub vfilter: [u8; 7],
}

/// `GXColor` from `ogc/gx.h`.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GXColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl GXColor {
    pub const BLACK: GXColor = GXColor::rgb(0, 0, 0);
    pub const WHITE: GXColor = GXColor::rgb(255, 255, 255);

    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> GXColor {
        GXColor { r, g, b, a: 255 }
    }

    #[inline]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> GXColor {
        GXColor { r, g, b, a }
    }
}

/// `guVector` from `ogc/gu.h`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct guVector {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// `guQuaternion` from `ogc/gu.h`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct guQuaternion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// `Mtx`: `typedef f32 Mtx[3][4]` (`attribute(aligned(32))` at use sites).
pub type Mtx = [[f32; 4]; 3];
/// `Mtx44`: `typedef f32 Mtx44[4][4]`.
pub type Mtx44 = [[f32; 4]; 4];

/// Opaque `GXFifoObj` handle returned by `GX_Init`.
pub type GXFifoObj = c_void;

/// Opaque storage matching `GXTexObj`; the C definition is internal to
/// libogc, so we keep a generously sized, aligned slot and only ever handle
/// it by pointer.
#[repr(C, align(32))]
pub struct GXTexObjOpaque {
    _opaque: [u32; 32],
}

impl GXTexObjOpaque {
    pub const fn new() -> Self {
        GXTexObjOpaque { _opaque: [0; 32] }
    }
}

impl Default for GXTexObjOpaque {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// constants (ogc/video_types.h, ogc/gx.h, ogc/pad.h, ogc/system.h)
// ---------------------------------------------------------------------------

/// Multiplier to get real pixel size in bytes.
pub const VI_DISPLAY_PIX_SZ: u32 = 2;
/// Video mode NON INTERLACED flag.
pub const VI_NON_INTERLACE: u32 = 1;

pub const FALSE: BOOL = 0;
pub const TRUE: BOOL = 1;

pub const GX_FALSE: u8 = 0;
pub const GX_TRUE: u8 = 1;
pub const GX_DISABLE: u8 = 0;
pub const GX_ENABLE: u8 = 1;

// primitive types
pub const GX_QUADS: u8 = 0x80;
pub const GX_TRIANGLES: u8 = 0x90;
pub const GX_TRIANGLESTRIP: u8 = 0x98;
pub const GX_TRIANGLEFAN: u8 = 0xA0;
pub const GX_LINES: u8 = 0xA8;
pub const GX_LINESTRIP: u8 = 0xB0;
pub const GX_POINTS: u8 = 0xB8;

// vertex attributes
pub const GX_VA_POS: u8 = 9;
pub const GX_VA_NRM: u8 = 10;
pub const GX_VA_CLR0: u8 = 11;
pub const GX_VA_TEX0: u8 = 13;

// attribute input types
pub const GX_NONE: u8 = 0;
pub const GX_DIRECT: u8 = 1;
pub const GX_INDEX8: u8 = 2;
pub const GX_INDEX16: u8 = 3;

// component counts
pub const GX_POS_XY: u8 = 0;
pub const GX_POS_XYZ: u8 = 1;
pub const GX_NRM_XYZ: u8 = 0;
pub const GX_CLR_RGBA: u8 = 1;
pub const GX_TEX_S: u8 = 0;
pub const GX_TEX_ST: u8 = 1;

// component formats
pub const GX_U8: u8 = 0;
pub const GX_S8: u8 = 1;
pub const GX_U16: u8 = 2;
pub const GX_S16: u8 = 3;
pub const GX_F32: u8 = 4;
pub const GX_RGBA8: u8 = 5;

// vertex format table slots
pub const GX_VTXFMT0: u8 = 0;
pub const GX_MAX_VTXFMT: u8 = 8;

// TEV
pub const GX_TEVSTAGE0: u8 = 0;
pub const GX_MODULATE: u8 = 0;
pub const GX_DECAL: u8 = 1;
pub const GX_REPLACE: u8 = 3;
pub const GX_PASSCLR: u8 = 4;

// color channels / texcoord / texmap selects
pub const GX_COLOR0A0: u8 = 4;
pub const GX_COLORNULL: u8 = 0xff;
pub const GX_TEXMAP_NULL: u8 = 0xff;
pub const GX_TEXCOORDNULL: u8 = 0xff;
pub const GX_TEXMAP0: u8 = 0;
pub const GX_TEXCOORD0: u8 = 0;

// texgen
pub const GX_TG_MTX3x4: u8 = 0;
pub const GX_TG_MTX2x4: u8 = 1;
pub const GX_TG_POS: u8 = 0;
pub const GX_TG_TEX0: u8 = 4;
pub const GX_IDENTITY: u8 = 60;

// channel control
pub const GX_SRC_REG: u8 = 0;
pub const GX_SRC_VTX: u8 = 1;
pub const GX_LIGHTNULL: u32 = 0x000;
pub const GX_DF_NONE: u8 = 0;
pub const GX_DF_SIGNED: u8 = 1;
pub const GX_AF_NONE: u8 = 2;
pub const GX_AF_SPOT: u8 = 1;
pub const GX_AF_SPEC: u8 = 0;

// matrix / projection
pub const GX_PNMTX0: u8 = 0;
pub const GX_PERSPECTIVE: u8 = 0;
pub const GX_ORTHOGRAPHIC: u8 = 1;

// compare functions
pub const GX_NEVER: u8 = 0;
pub const GX_LESS: u8 = 1;
pub const GX_EQUAL: u8 = 2;
pub const GX_LEQUAL: u8 = 3;
pub const GX_GREATER: u8 = 4;
pub const GX_NEQUAL: u8 = 5;
pub const GX_GEQUAL: u8 = 6;
pub const GX_ALWAYS: u8 = 7;

// misc
pub const GX_CULL_NONE: u8 = 0;
pub const GX_CULL_FRONT: u8 = 1;
pub const GX_CULL_BACK: u8 = 2;
pub const GX_CULL_ALL: u8 = 3;
pub const GX_GM_1_0: u8 = 0;
pub const GX_PF_RGB8_Z24: u8 = 0;
pub const GX_ZC_LINEAR: u8 = 0;
pub const GX_TF_RGB565: u8 = 0x4;
pub const GX_TF_RGB5A3: u8 = 0x5;
pub const GX_TF_RGBA8: u8 = 0x6;
pub const GX_CLAMP: u8 = 0;
pub const GX_REPEAT: u8 = 1;
pub const GX_MIRROR: u8 = 2;

/// Cached memory base (`SYS_BASE_CACHED`).
pub const SYS_BASE_CACHED: u32 = 0x8000_0000;
/// Uncached memory base (`SYS_BASE_UNCACHED`).
pub const SYS_BASE_UNCACHED: u32 = 0xC000_0000;

/// `MEM_K0_TO_K1(x)`: cached -> uncached virtual address.
#[inline]
pub const fn mem_k0_to_k1(x: *mut c_void) -> *mut c_void {
    x.wrapping_byte_add((SYS_BASE_UNCACHED - SYS_BASE_CACHED) as usize)
}

/// Hardware address of the "write gather pipe" that immediate-mode GX
/// attribute data is streamed through (see `wgPipe` in libogc).
pub const WG_PIPE: u32 = 0xCC00_8000;

// ---------------------------------------------------------------------------
// libogc
// ---------------------------------------------------------------------------
extern "C" {
    // video subsystem (ogc/video.h)
    pub fn VIDEO_Init();
    pub fn VIDEO_GetPreferredMode(mode: *mut GXRModeObj) -> *mut GXRModeObj;
    pub fn VIDEO_Configure(rmode: *const GXRModeObj);
    pub fn VIDEO_SetNextFramebuffer(fb: *mut c_void);
    pub fn VIDEO_SetBlack(black: BOOL);
    pub fn VIDEO_Flush();
    pub fn VIDEO_WaitVSync();

    // system (ogc/system.h)
    pub fn SYS_AllocateFramebuffer(rmode: *const GXRModeObj) -> *mut c_void;

    // console (ogc/console.h)
    pub fn CON_Init(
        framebuffer: *mut c_void,
        xstart: c_int,
        ystart: c_int,
        xres: c_int,
        yres: c_int,
        stride: c_int,
    );

    // gamecube pad (ogc/pad.h)
    pub fn PAD_Init() -> u32;
    pub fn PAD_ScanPads() -> u32;
    pub fn PAD_ButtonsDown(pad: c_int) -> u16;
    pub fn PAD_ButtonsHeld(pad: c_int) -> u16;
    pub fn PAD_StickX(pad: c_int) -> i8;
    pub fn PAD_StickY(pad: c_int) -> i8;
    pub fn PAD_SubStickX(pad: c_int) -> i8;
    pub fn PAD_SubStickY(pad: c_int) -> i8;
    pub fn PAD_TriggerL(pad: c_int) -> u8;
    pub fn PAD_TriggerR(pad: c_int) -> u8;

    // data cache (ogc/cache.h)
    pub fn DCFlushRange(addr: *const c_void, size: u32);

    // GX: bring-up / EFB->XFB copy pipeline (ogc/gx.h)
    pub fn GX_Init(fifo_base: *mut c_void, fifo_size: u32) -> *mut GXFifoObj;
    pub fn GX_SetViewport(x: f32, y: f32, w: f32, h: f32, n: f32, f: f32);
    pub fn GX_SetScissor(x: u32, y: u32, w: u32, h: u32);
    pub fn GX_SetDispCopySrc(x: u16, y: u16, w: u16, h: u16);
    pub fn GX_SetDispCopyDst(w: u16, h: u16);
    pub fn GX_SetDispCopyYScale(scale: f32) -> u32;
    pub fn GX_GetYScaleFactor(efb_height: u16, xfb_height: u16) -> f32;
    pub fn GX_SetCopyFilter(aa: u8, sample_pattern: *const u8, vf: u8, vfilter: *const u8);
    pub fn GX_SetCopyClear(clear_color: GXColor, clear_z: u32);
    pub fn GX_SetPixelFmt(pix_fmt: u8, z_fmt: u8);
    pub fn GX_SetFieldMode(field_rendering: u8, even_ratio: u8);
    pub fn GX_SetCullMode(mode: u8);
    pub fn GX_CopyDisp(dest: *mut c_void, clear: u8);
    pub fn GX_SetDispCopyGamma(gamma: u8);
    pub fn GX_SetZMode(enable: u8, func: u8, update_enable: u8);
    pub fn GX_SetColorUpdate(enable: u8);
    pub fn GX_SetAlphaUpdate(enable: u8);
    pub fn GX_SetBlendMode(mode: u8, src: u8, dst: u8, logic: u8);
    pub fn GX_Flush();
    pub fn GX_DrawDone() -> u32;
    pub fn GX_InvVtxCache();
    pub fn GX_InvalidateTexAll();

    // GX: vertex description / attribute streams
    pub fn GX_ClearVtxDesc();
    pub fn GX_SetVtxDesc(attr: u8, input_type: u8);
    pub fn GX_SetVtxAttrFmt(vtxfmt: u8, attr: u8, comptype: u8, compfmt: u8, frac: u8);

    // GX: immediate mode
    pub fn GX_Begin(primitive: u8, vtxfmt: u8, count: u16);

    // GX: transforms
    pub fn GX_LoadProjectionMtx(mtx: *const Mtx44, ptype: u8);
    pub fn GX_LoadPosMtxImm(mtx: *const Mtx, pnidx: u32);
    pub fn GX_SetCurrentMtx(mtx: u32);

    // GX: TEV / texgen / channels
    pub fn GX_SetNumChans(num: u8);
    pub fn GX_SetNumTexGens(num: u32);
    pub fn GX_SetNumTevStages(num: u8);
    pub fn GX_SetTevOp(stage: u8, mode: u8);
    pub fn GX_SetTevOrder(stage: u8, texcoord: u8, texmap: u8, color: u8);
    pub fn GX_SetChanCtrl(
        channel: u8,
        enable: u8,
        ambsrc: u8,
        matsrc: u8,
        litmask: u32,
        diff_fn: u8,
        attn_fn: u8,
    );
    pub fn GX_SetTexCoordGen(texcoord: u16, tfunc: u32, src: u32, mtx: u32);

    // GX: textures
    pub fn GX_InitTexObj(
        obj: *mut GXTexObjOpaque,
        img_ptr: *const c_void,
        width: u16,
        height: u16,
        format: u8,
        wrap_s: u8,
        wrap_t: u8,
        mipmap: u8,
    );
    pub fn GX_LoadTexObj(obj: *const GXTexObjOpaque, mapid: u8);

    // gu matrix math (ogc/gu.h — GEKKO maps guMtx* onto the ps_* variants)
    pub fn guLookAt(m: *mut Mtx, cam: *const guVector, up: *const guVector, look: *const guVector);
    pub fn guPerspective(m: *mut Mtx44, fovY: f32, aspect: f32, n: f32, f: f32);
    pub fn ps_guMtxIdentity(m: *mut Mtx);
    pub fn ps_guMtxConcat(a: *const Mtx, b: *const Mtx, ab: *mut Mtx);
    pub fn ps_guMtxTransApply(src: *const Mtx, dst: *mut Mtx, x: f32, y: f32, z: f32);
    pub fn ps_guMtxRotAxisRad(m: *mut Mtx, axis: *const guVector, rad: f32);
}

// ---------------------------------------------------------------------------
// newlib (powerpc-eabi libc shipped with devkitPPC)
// ---------------------------------------------------------------------------
extern "C" {
    pub fn printf(fmt: *const c_char, ...) -> c_int;
    pub fn exit(status: c_int) -> !;

    pub fn malloc(size: usize) -> *mut c_void;
    pub fn free(ptr: *mut c_void);
    pub fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    pub fn memalign(align: usize, size: usize) -> *mut c_void;
    pub fn memset(ptr: *mut c_void, c: c_int, n: usize) -> *mut c_void;
}
