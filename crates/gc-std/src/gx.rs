//! GX: the GameCube's graphics processor.
//!
//! [`Context`] performs the full bring-up of the GX pipeline (command FIFO,
//! EFB->XFB copy setup, viewport, scissor) exactly like the canonical
//! libogc examples, and owns the video hardware afterwards. Geometry is
//! streamed in *immediate mode* through the write-gather pipe via
//! [`begin`], [`position3f32`], [`color4u8`], [`normal3f32`] and
//! [`texcoord2f32`].
//!
//! Typical frame:
//!
//! ```rust,no_run
//! # let mut gx: gc_std::gx::Context = todo!();
//! gx.begin_frame();
//! gx::begin(gx::Primitive::Quads, 0, 24);
//! // ... stream vertices ...
//! gx.draw_done();
//! gx.end_frame();
//! ```

use core::ffi::c_void;

use crate::ffi::{self, GXColor, GXFifoObj, GXRModeObj, GXTexObjOpaque, Mtx, Mtx44};
use crate::video::Video;

// Ergonomic re-exports of the most commonly used raw constants. The full
// set remains available under [`crate::ffi`].
#[doc(no_inline)]
pub use crate::ffi::{
    GX_CLR_RGBA, GX_CULL_BACK, GX_CULL_FRONT, GX_CULL_NONE, GX_DIRECT, GX_F32, GX_LEQUAL,
    GX_NRM_XYZ, GX_POS_XYZ, GX_RGBA8, GX_TEX_ST, GX_VA_CLR0, GX_VA_NRM, GX_VA_POS, GX_VA_TEX0,
    GX_VTXFMT0 as VTXFMT0,
};

/// Size of the GX command FIFO (same as the libogc examples).
const FIFO_SIZE: usize = 256 * 1024;

/// GX primitive types (`GX_TRIANGLES`, `GX_QUADS`, ...).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Primitive {
    Quads = ffi::GX_QUADS,
    Triangles = ffi::GX_TRIANGLES,
    TriangleStrip = ffi::GX_TRIANGLESTRIP,
    TriangleFan = ffi::GX_TRIANGLEFAN,
    Lines = ffi::GX_LINES,
    LineStrip = ffi::GX_LINESTRIP,
    Points = ffi::GX_POINTS,
}

/// Texture wrap modes (`GX_CLAMP`, ...).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum WrapMode {
    Clamp = ffi::GX_CLAMP,
    Repeat = ffi::GX_REPEAT,
    Mirror = ffi::GX_MIRROR,
}

/// Owner of the initialized GX pipeline.
///
/// Create via [`Gc::into_gx`](crate::Gc::into_gx). Dropping is not
/// supported; the context lives for the rest of the program.
pub struct Context {
    video: Video,
    _fifo: *mut GXFifoObj,
    _fifo_buffer: *mut c_void,
}

impl Context {
    /// The video mode the GX pipeline was configured for.
    pub fn mode(&self) -> &GXRModeObj {
        self.video.mode()
    }

    /// Set the background color (and max-Z) used by [`Self::draw_disp_copy`].
    pub fn set_clear_color(&self, color: GXColor) {
        unsafe { ffi::GX_SetCopyClear(color, 0x00ff_ffff) };
    }

    /// Load a perspective projection matrix (`GX_PERSPECTIVE`).
    pub fn load_perspective(&self, proj: &Mtx44) {
        unsafe { ffi::GX_LoadProjectionMtx(proj, ffi::GX_PERSPECTIVE) };
    }

    /// Load a model-view matrix into position matrix slot 0
    /// (`GX_PNMTX0`) and make it current.
    pub fn load_model_view(&self, mv: &Mtx) {
        unsafe {
            ffi::GX_LoadPosMtxImm(mv, u32::from(ffi::GX_PNMTX0));
            ffi::GX_SetCurrentMtx(u32::from(ffi::GX_PNMTX0));
        }
    }

    /// Per-frame preamble: reset viewport/scissor to the full frame and
    /// invalidate the vertex cache and texture cache (mirrors the libogc
    /// examples).
    pub fn begin_frame(&self) {
        let mode = self.mode();
        unsafe {
            ffi::GX_SetViewport(
                0.0,
                0.0,
                f32::from(mode.fbWidth),
                f32::from(mode.efbHeight),
                0.0,
                1.0,
            );
            ffi::GX_InvVtxCache();
            ffi::GX_InvalidateTexAll();
        }
    }

    /// Block until the GPU has consumed all commands (`GX_DrawDone`).
    pub fn draw_done(&self) {
        unsafe {
            ffi::GX_DrawDone();
        }
    }

    /// Copy the finished EFB frame into the external framebuffer, flush the
    /// FIFO and wait for the next vertical retrace.
    pub fn end_frame(&self) {
        unsafe {
            ffi::GX_SetZMode(ffi::GX_TRUE, ffi::GX_LEQUAL, ffi::GX_TRUE);
            ffi::GX_SetColorUpdate(ffi::GX_TRUE);
            ffi::GX_CopyDisp(self.video.framebuffer(), ffi::GX_TRUE);
            ffi::GX_Flush();
            ffi::VIDEO_WaitVSync();
        }
    }
}

pub(crate) fn init(video: Video) -> Context {
    let mode = *video.mode();

    let fifo_buffer = unsafe {
        let buf = ffi::mem_k0_to_k1(ffi::memalign(32, FIFO_SIZE));
        assert!(!buf.is_null(), "out of memory allocating GX FIFO");
        ffi::memset(buf, 0, FIFO_SIZE);
        buf
    };

    unsafe {
        let fifo = ffi::GX_Init(fifo_buffer, FIFO_SIZE as u32);
        assert!(!fifo.is_null(), "GX_Init failed");

        ffi::GX_SetCopyClear(GXColor::BLACK, 0x00ff_ffff);
        ffi::GX_SetViewport(
            0.0,
            0.0,
            f32::from(mode.fbWidth),
            f32::from(mode.efbHeight),
            0.0,
            1.0,
        );
        ffi::GX_SetDispCopyYScale(
            f32::from(mode.xfbHeight) / f32::from(mode.efbHeight),
        );
        ffi::GX_SetScissor(0, 0, u32::from(mode.fbWidth), u32::from(mode.efbHeight));
        ffi::GX_SetDispCopySrc(0, 0, mode.fbWidth, mode.efbHeight);
        ffi::GX_SetDispCopyDst(mode.fbWidth, mode.xfbHeight);
        ffi::GX_SetCopyFilter(
            mode.aa,
            mode.sample_pattern.as_ptr().cast(),
            ffi::GX_TRUE,
            mode.vfilter.as_ptr(),
        );
        ffi::GX_SetFieldMode(
            mode.field_rendering,
            if mode.viHeight == 2 * mode.xfbHeight {
                ffi::GX_ENABLE
            } else {
                ffi::GX_DISABLE
            },
        );
        ffi::GX_SetCullMode(ffi::GX_CULL_NONE);
        ffi::GX_CopyDisp(video.framebuffer(), ffi::GX_TRUE);
        ffi::GX_SetDispCopyGamma(ffi::GX_GM_1_0);

        Context {
            video,
            _fifo: fifo,
            _fifo_buffer: fifo_buffer,
        }
    }
}

// ---------------------------------------------------------------------------
// vertex stream configuration (thin wrappers over ogc/gx.h)
// ---------------------------------------------------------------------------

/// `GX_ClearVtxDesc`: drop all vertex attribute descriptors.
pub fn clear_vtx_desc() {
    unsafe { ffi::GX_ClearVtxDesc() };
}

/// `GX_SetVtxDesc(attr, GX_DIRECT)`: mark `attr` as directly streamed.
pub fn set_vtx_desc_direct(attr: u8) {
    unsafe { ffi::GX_SetVtxDesc(attr, ffi::GX_DIRECT) };
}

/// `GX_SetVtxAttrFmt`: describe the component layout of `attr` in `vtxfmt`.
pub fn set_vtx_attr_fmt(vtxfmt: u8, attr: u8, comp_type: u8, comp_fmt: u8, frac: u8) {
    unsafe { ffi::GX_SetVtxAttrFmt(vtxfmt, attr, comp_type, comp_fmt, frac) };
}

/// Configure the default "1 color channel sourced from the vertex, one TEV
/// stage passing color through" state used for unlit colored geometry.
pub fn config_vertex_color_pipeline() {
    unsafe {
        ffi::GX_SetNumChans(1);
        ffi::GX_SetNumTexGens(0);
        ffi::GX_SetNumTevStages(1);
        ffi::GX_SetChanCtrl(
            ffi::GX_COLOR0A0,
            ffi::GX_DISABLE,
            ffi::GX_SRC_REG,
            ffi::GX_SRC_VTX,
            ffi::GX_LIGHTNULL,
            ffi::GX_DF_NONE,
            ffi::GX_AF_NONE,
        );
        ffi::GX_SetTevOrder(
            ffi::GX_TEVSTAGE0,
            ffi::GX_TEXCOORDNULL,
            ffi::GX_TEXMAP_NULL,
            ffi::GX_COLOR0A0,
        );
        ffi::GX_SetTevOp(ffi::GX_TEVSTAGE0, ffi::GX_PASSCLR);
    }
}

/// Configure the default "vertex color modulated by texture map 0" state
/// used for textured geometry.
pub fn config_textured_pipeline() {
    unsafe {
        ffi::GX_SetNumChans(1);
        ffi::GX_SetNumTexGens(1);
        ffi::GX_SetNumTevStages(1);
        ffi::GX_SetChanCtrl(
            ffi::GX_COLOR0A0,
            ffi::GX_DISABLE,
            ffi::GX_SRC_REG,
            ffi::GX_SRC_VTX,
            ffi::GX_LIGHTNULL,
            ffi::GX_DF_NONE,
            ffi::GX_AF_NONE,
        );
        ffi::GX_SetTexCoordGen(
            u16::from(ffi::GX_TEXCOORD0),
            u32::from(ffi::GX_TG_MTX2x4),
            u32::from(ffi::GX_TG_TEX0),
            u32::from(ffi::GX_IDENTITY),
        );
        ffi::GX_SetTevOrder(
            ffi::GX_TEVSTAGE0,
            ffi::GX_TEXCOORD0,
            ffi::GX_TEXMAP0,
            ffi::GX_COLOR0A0,
        );
        ffi::GX_SetTevOp(ffi::GX_TEVSTAGE0, ffi::GX_MODULATE);
    }
}

/// `GX_SetZMode`.
pub fn set_z_mode(enable: bool, func: u8, update: bool) {
    unsafe {
        ffi::GX_SetZMode(enable as u8, func, update as u8);
    }
}

/// `GX_SetCullMode`.
pub fn set_cull_mode(mode: u8) {
    unsafe { ffi::GX_SetCullMode(mode) };
}

// ---------------------------------------------------------------------------
// immediate mode (the write-gather pipe)
// ---------------------------------------------------------------------------

/// `GX_Begin`: start streaming `count` vertices of `primitive`.
#[inline]
pub fn begin(primitive: Primitive, vtxfmt: u8, count: u16) {
    unsafe { ffi::GX_Begin(primitive as u8, vtxfmt, count) };
}

/// `GX_Position3f32`.
#[inline(always)]
pub fn position3f32(x: f32, y: f32, z: f32) {
    unsafe {
        (ffi::WG_PIPE as *mut f32).write_volatile(x);
        (ffi::WG_PIPE as *mut f32).write_volatile(y);
        (ffi::WG_PIPE as *mut f32).write_volatile(z);
    }
}

/// `GX_Normal3f32`.
#[inline(always)]
pub fn normal3f32(x: f32, y: f32, z: f32) {
    position3f32(x, y, z);
}

/// `GX_Color4u8`.
#[inline(always)]
pub fn color4u8(r: u8, g: u8, b: u8, a: u8) {
    unsafe {
        (ffi::WG_PIPE as *mut u8).write_volatile(r);
        (ffi::WG_PIPE as *mut u8).write_volatile(g);
        (ffi::WG_PIPE as *mut u8).write_volatile(b);
        (ffi::WG_PIPE as *mut u8).write_volatile(a);
    }
}

/// Convenience wrapper around [`color4u8`] for a [`GXColor`].
#[inline(always)]
pub fn color(c: GXColor) {
    color4u8(c.r, c.g, c.b, c.a);
}

/// `GX_TexCoord2f32`.
#[inline(always)]
pub fn texcoord2f32(s: f32, t: f32) {
    unsafe {
        (ffi::WG_PIPE as *mut f32).write_volatile(s);
        (ffi::WG_PIPE as *mut f32).write_volatile(t);
    }
}

// ---------------------------------------------------------------------------
// textures
// ---------------------------------------------------------------------------

/// A resident GPU texture (RGB565) plus its owning pixel buffer.
pub struct Texture {
    obj: GXTexObjOpaque,
    data: *mut u8,
    len: usize,
    width: u16,
    height: u16,
}

impl Texture {
    /// Upload a RGB565 texture. `pixels` is `width*height` pixels in
    /// row-major order; this function swizzles them into the 4x4 tiled
    /// layout the GPU expects, flushes the data cache and registers the
    /// texture object.
    ///
    /// `width` and `height` must be multiples of 4.
    pub fn from_rgb565(width: u16, height: u16, pixels: &[u16], wrap: WrapMode) -> Texture {
        assert_eq!(
            pixels.len(),
            usize::from(width) * usize::from(height),
            "pixel buffer size mismatch"
        );
        assert!(
            width % 4 == 0 && height % 4 == 0,
            "dimensions must be multiples of 4"
        );

        // Swizzle into 4x4 tiles (GameCube native texture layout) inside a
        // 32-byte aligned buffer (GX alignment requirement).
        let len = pixels.len() * 2;
        let layout = core::alloc::Layout::from_size_align(len, 32).unwrap();
        let data = unsafe { alloc::alloc::alloc(layout) };
        assert!(!data.is_null(), "out of memory allocating texture");
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

        let mut obj = GXTexObjOpaque::new();
        unsafe {
            ffi::DCFlushRange(data.cast(), len as u32);
            ffi::GX_InitTexObj(
                &mut obj,
                data.cast(),
                width,
                height,
                ffi::GX_TF_RGB565,
                wrap as u8,
                wrap as u8,
                ffi::GX_FALSE,
            );
        }
        Texture {
            obj,
            data,
            len,
            width,
            height,
        }
    }

    /// `GX_LoadTexObj(obj, GX_TEXMAP0)`.
    pub fn bind(&self) {
        unsafe { ffi::GX_LoadTexObj(&self.obj, ffi::GX_TEXMAP0) };
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        let layout = core::alloc::Layout::from_size_align(self.len, 32).unwrap();
        unsafe { alloc::alloc::dealloc(self.data, layout) };
    }
}

/// `DCFlushRange`: make CPU-written data in `slice` visible to the GPU.
pub fn flush_data_cache<T>(slice: &[T]) {
    unsafe {
        let byte_len = core::mem::size_of_val(slice) as u32;
        ffi::DCFlushRange(slice.as_ptr().cast(), byte_len);
    }
}
