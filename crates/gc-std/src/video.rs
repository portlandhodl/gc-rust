//! Video Interface (VI) driver — pure Rust port of the relevant parts of
//! libogc's `video.c` (zlib license, (c) devkitPro).
//!
//! Supports the mode every GC demo uses: 640×480 (NTSC) / 640×576 (PAL)
//! interlaced, double-field framebuffer, 4:2:2 YUY2. Timing values come
//! from libogc's `video_timing` table. Vsync wait polls VI's current
//! display position registers, no IRQs needed.

use crate::gctypes::GXRModeObj;
use crate::hw::{self, vi_read, vi_write};

/// libogc `struct _timing`.
#[derive(Copy, Clone)]
struct VideoTiming {
    equ: u16,
    acv: u16,
    prb_odd: u16,
    prb_even: u16,
    psb_odd: u16,
    psb_even: u16,
    bs1: u16,
    bs2: u16,
    bs3: u16,
    bs4: u16,
    be1: u16,
    be2: u16,
    be3: u16,
    be4: u16,
    nhlines: u16,
    hlw: u16,
    hsy: u16,
    hcs: u16,
    hce: u16,
    hbe640: u16,
    hbs640: u16,
}

pub const VI_NTSC: u32 = 0;
pub const VI_PAL: u32 = 1;
pub const VI_MPAL: u32 = 2;
pub const VI_EURGB60: u32 = 5; // 60 Hz PAL-ish

/// Video standard.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Standard {
    Ntsc,
    Pal,
    Mpal,
    EurRgb60,
}

/// Frame mode (progressive vs interlaced).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameMode {
    Interlaced,
    Progressive,
}

// Timing-table index for (tv, progressive):
//   0 = NTSC interlaced, 1 = NTSC progressive
//   2 = PAL  interlaced, 3 = PAL  progressive
//   4 = MPAL interlaced, 5 = MPAL progressive
// (EURGB60 uses the NTSC progression in libogc's table.)
static TIMINGS: [VideoTiming; 6] = [
    // 0: NTSC interlaced
    VideoTiming {
        equ: 0x06, acv: 0x00F0,
        prb_odd: 0x0018, prb_even: 0x0019, psb_odd: 0x0003, psb_even: 0x0002,
        bs1: 0x0C, bs2: 0x0D, bs3: 0x0C, bs4: 0x0D,
        be1: 0x0208, be2: 0x0207, be3: 0x0208, be4: 0x0207,
        nhlines: 0x020D, hlw: 0x01AD,
        hsy: 0x40, hcs: 0x47, hce: 0x69, hbe640: 0xA2, hbs640: 0x0175,
    },
    // 1: NTSC progressive (entry 6 in libogc's table: 480p)
    VideoTiming {
        equ: 0x0C, acv: 0x01E0,
        prb_odd: 0x0030, prb_even: 0x0030, psb_odd: 0x0006, psb_even: 0x0006,
        bs1: 0x18, bs2: 0x18, bs3: 0x18, bs4: 0x18,
        be1: 0x040E, be2: 0x040E, be3: 0x040E, be4: 0x040E,
        nhlines: 0x041A, hlw: 0x01AD,
        hsy: 0x40, hcs: 0x47, hce: 0x69, hbe640: 0xA2, hbs640: 0x0175,
    },
    // 2: PAL interlaced
    VideoTiming {
        equ: 0x05, acv: 0x0120,
        prb_odd: 0x0021, prb_even: 0x0022, psb_odd: 0x0001, psb_even: 0x0000,
        bs1: 0x0D, bs2: 0x0C, bs3: 0x0B, bs4: 0x0A,
        be1: 0x026B, be2: 0x026A, be3: 0x0269, be4: 0x026C,
        nhlines: 0x0271, hlw: 0x01B0,
        hsy: 0x40, hcs: 0x4B, hce: 0x6A, hbe640: 0xAC, hbs640: 0x017C,
    },
    // 3: PAL progressive
    VideoTiming {
        equ: 0x0A, acv: 0x0240,
        prb_odd: 0x0044, prb_even: 0x0044, psb_odd: 0x0000, psb_even: 0x0000,
        bs1: 0x14, bs2: 0x14, bs3: 0x14, bs4: 0x14,
        be1: 0x04D8, be2: 0x04D8, be3: 0x04D8, be4: 0x04D8,
        nhlines: 0x04E2, hlw: 0x01B0,
        hsy: 0x40, hcs: 0x4B, hce: 0x6A, hbe640: 0xAC, hbs640: 0x017C,
    },
    // 4: MPAL interlaced (entry 4 in libogc's table)
    VideoTiming {
        equ: 0x06, acv: 0x00F0,
        prb_odd: 0x0018, prb_even: 0x0019, psb_odd: 0x0003, psb_even: 0x0002,
        bs1: 0x10, bs2: 0x0F, bs3: 0x0E, bs4: 0x0D,
        be1: 0x0206, be2: 0x0205, be3: 0x0204, be4: 0x0207,
        nhlines: 0x020D, hlw: 0x01AD,
        hsy: 0x40, hcs: 0x4E, hce: 0x70, hbe640: 0xA2, hbs640: 0x0175,
    },
    // 5: MPAL progressive (same shape as NTSC-prog but MPAL sync)
    VideoTiming {
        equ: 0x0C, acv: 0x01E0,
        prb_odd: 0x0030, prb_even: 0x0030, psb_odd: 0x0006, psb_even: 0x0006,
        bs1: 0x18, bs2: 0x18, bs3: 0x18, bs4: 0x18,
        be1: 0x040E, be2: 0x040E, be3: 0x040E, be4: 0x040E,
        nhlines: 0x041A, hlw: 0x01AD,
        hsy: 0x40, hcs: 0x4E, hce: 0x70, hbe640: 0xA2, hbs640: 0x0175,
    },
];

fn timing_index(standard: Standard, mode: FrameMode) -> usize {
    let s = match standard {
        Standard::Ntsc | Standard::EurRgb60 => 0,
        Standard::Pal => 2,
        Standard::Mpal => 4,
    };
    s + (mode == FrameMode::Progressive) as usize
}

/// VI copy-filter taps (defaults from libogc).
static TAPS: [u16; 26] = [
    0x01F0, 0x01DC, 0x01AE, 0x0174, 0x0129, 0x00DB,
    0x008E, 0x0046, 0x000C, 0x00E2, 0x00CB, 0x00C0,
    0x00C4, 0x00CF, 0x00DE, 0x00EC, 0x00FC, 0x0008,
    0x000F, 0x0013, 0x0013, 0x000F, 0x000C, 0x0008,
    0x0001, 0x0000,
];

/// Handle to the video subsystem (values are copies of global HW state).
#[derive(Copy, Clone)]
pub struct Video {
    mode: GXRModeObj,
    /// back (draw) buffer, uncached
    framebuffer: *mut core::ffi::c_void,
    /// slot currently displayed (0/1); the draw buffer is `1 - front`.
    front: u8,
}

/// Worst-case XFB slot size (640x576 YUY2) — the two flip-chain slots in
/// the reserved region (`memory.x.ld`, `__xfb_base`) are spaced by this.
const XFB_SLOT_BYTES: usize = 640 * 576 * 2;

/// Uncached pointer to flip-chain slot `idx`.
fn slot_ptr(idx: u8) -> *mut core::ffi::c_void {
    extern "C" {
        static __xfb_base: u32;
    }
    let base = &raw const __xfb_base as usize;
    hw::cached_to_uncached((base + idx as usize * XFB_SLOT_BYTES) as *mut core::ffi::c_void)
}

impl Video {
    pub fn mode(&self) -> &GXRModeObj {
        &self.mode
    }
    /// Uncached pointer to the *draw* (back) framebuffer (YUY2). When the
    /// flip chain is never used this is also the displayed buffer.
    pub fn framebuffer(&self) -> *mut core::ffi::c_void {
        self.framebuffer
    }
    /// Block until the next vertical retrace.
    pub fn wait_vsync(&self) {
        wait_vsync_inner()
    }

    /// Take the display out of the blank screen (`VIDEO_SetBlack(false)`).
    /// `enable_console`/`into_gx` call this; call it yourself only for raw
    /// framebuffer programs (no console, no GX).
    pub fn show(&self) {
        set_black(false);
    }

    /// Flip the double buffer: wait for vertical retrace, then display the
    /// buffer just drawn and return a handle whose draw buffer is the
    /// previous front buffer. Programs that never call `flip()` stay single
    /// buffered (draw buffer == displayed buffer), exactly as before.
    pub fn flip(&self) -> Video {
        flip_current();
        unsafe { *core::ptr::addr_of!(CURRENT) }
    }
}

/// Flip-core operating on the global CURRENT state (used by the GX copy
/// path as well). Waits for vsync, points VI at the back buffer, swaps
/// front/back bookkeeping.
pub(crate) fn flip_current() {
    let mode = unsafe { (*core::ptr::addr_of!(CURRENT)).mode };
    wait_vsync_inner();
    unsafe {
        let cur = core::ptr::addr_of_mut!(CURRENT);
        let next_front = 1 - (*cur).front;
        program_frame_buffers(&mode, slot_ptr(next_front));
        (*cur).front = next_front;
        (*cur).framebuffer = slot_ptr(1 - next_front);
    }
}

/// Block until the next vertical retrace (module-function form).
pub fn wait_vsync() {
    wait_vsync_inner()
}

/// The active external framebuffer (uncached), for the GX copy path.
pub(crate) fn current_xfb() -> *mut core::ffi::c_void {
    unsafe { (*core::ptr::addr_of_mut!(CURRENT)).framebuffer() }
}

static mut CURRENT: Video = Video {
    mode: GXRModeObj {
        viTVMode: 0,
        fbWidth: 640,
        efbHeight: 480,
        xfbHeight: 480,
        viXOrigin: 40,
        viYOrigin: 0,
        viWidth: 640,
        viHeight: 480,
        xfbMode: 1,
        field_rendering: 0,
        aa: 0,
        sample_pattern: [[6; 2]; 12],
        vfilter: [8, 8, 10, 12, 10, 8, 8],
    },
    framebuffer: core::ptr::null_mut(),
    front: 0,
};

/// Build a `GXRModeObj` matching libogc's IntDf/Prog modes for the given
/// standard and frame mode — this is the shape of TVNtsc480IntDf etc.
fn mode_for(standard: Standard, fm: FrameMode) -> GXRModeObj {
    let interlaced = fm == FrameMode::Interlaced;
    let progressive = fm == FrameMode::Progressive;
    let (height_576, fb_h) = match standard {
        Standard::Pal => (true, 576u16),
        Standard::Ntsc | Standard::Mpal | Standard::EurRgb60 => (false, 480u16),
    };
    let tv_bits = match standard {
        Standard::Ntsc => 0u32,
        Standard::Pal => 1u32,
        Standard::Mpal => 2u32,
        Standard::EurRgb60 => 5u32,
    };
    let interlace_bits = if progressive { 2u32 } else if interlaced { 0u32 } else { 1u32 };
    GXRModeObj {
        viTVMode: (tv_bits << 2) | interlace_bits,
        fbWidth: 640,
        efbHeight: fb_h,
        xfbHeight: if progressive { fb_h } else { fb_h },
        viXOrigin: 40,
        viYOrigin: 0,
        viWidth: 640,
        viHeight: if height_576 { 528 } else { 480 },
        xfbMode: if progressive { 0 } else { 1 }, // DF when interlaced
        field_rendering: 0,
        aa: 0,
        sample_pattern: [[6; 2]; 12],
        vfilter: [8, 8, 10, 12, 10, 8, 8],
    }
}

pub(crate) fn init() -> Video {
    unsafe {
        if vi_read(1) & 0x0001 == 0 {
            vi_reset_with(0); // NTSC interlaced
        }

        // AA/vfilter taps
        vi_write(38, (TAPS[1] >> 6) | (TAPS[2] << 4));
        vi_write(39, TAPS[0] | ((TAPS[1] & 0x3f) << 10));
        vi_write(40, (TAPS[4] >> 6) | (TAPS[5] << 4));
        vi_write(41, TAPS[3] | ((TAPS[4] & 0x3f) << 10));
        vi_write(42, (TAPS[7] >> 6) | (TAPS[8] << 4));
        vi_write(43, TAPS[6] | ((TAPS[7] & 0x3f) << 10));
        vi_write(44, TAPS[11] | (TAPS[12] << 8));
        vi_write(45, TAPS[9] | (TAPS[10] << 8));
        vi_write(46, TAPS[15] | (TAPS[16] << 8));
        vi_write(47, TAPS[13] | (TAPS[14] << 8));
        vi_write(48, TAPS[19] | (TAPS[20] << 8));
        vi_write(49, TAPS[17] | (TAPS[18] << 8));
        vi_write(50, TAPS[23] | (TAPS[24] << 8));
        vi_write(51, TAPS[21] | (TAPS[22] << 8));
        vi_write(56, 640);

        // interlaced DIs
        vi_write(26, 0x1001);
        vi_write(27, 0x0001);
        vi_write(36, 0x2828);

        let dcr = vi_read(1);
        let tvmode_bits = hw::shiftr(u32::from(dcr), 8, 2);
        let nonint = hw::shiftr(u32::from(dcr), 2, 1);
        let _ = nonint; // currently unused; VI_DCR's DFP bit would select progressive

        // decode TV standard; anything unusual defaults to what the loader left
        let standard = match tvmode_bits {
            0 => Standard::Ntsc,
            1 => Standard::Pal,
            2 => Standard::Mpal,
            5 => Standard::EurRgb60,
            _ => Standard::Ntsc,
        };
        let mode = mode_for(standard, FrameMode::Interlaced);
        // The XFBs live at a *fixed* cached address from the linker script;
        // the heap ends just below them. Two slots of XFB_SLOT_BYTES each.
        let size = usize::from(mode.fbWidth) * usize::from(mode.xfbHeight) * 2;
        for slot in 0..2u8 {
            let p = slot_ptr(slot) as *mut u8;
            core::ptr::write_bytes(p, 0x10, size); // black (uncached view)
        }

        let fb = slot_ptr(0);
        configure(&mode, fb);
        hw::dc_flush_range(slot_ptr(0) as *mut u8, size);
        hw::dc_flush_range(slot_ptr(1) as *mut u8, size);

        CURRENT = Video { mode, framebuffer: fb, front: 0 };
        CURRENT
    }
}

/// `__VIInit` port for a timing slot. On NTSC-interlaced default the slot
/// index is 0; progressive NTSC is 1.
unsafe fn vi_reset_with(idx: u32) {
    let cur = &TIMINGS[(idx as usize) % TIMINGS.len()];
    vi_write(1, 0x02);
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    vi_write(1, 0x00);

    vi_write(2, (cur.hcs << 8) | cur.hce);
    vi_write(3, cur.hlw);
    vi_write(4, cur.hbs640 << 1);
    vi_write(5, (cur.hbe640 << 7) | cur.hsy);

    vi_write(0, cur.equ);
    vi_write(6, cur.psb_odd + 2);
    vi_write(7, cur.prb_odd + ((cur.acv << 1) - 2));
    vi_write(8, cur.psb_even + 2);
    vi_write(9, cur.prb_even + ((cur.acv << 1) - 2));
    vi_write(10, (cur.be3 << 5) | cur.bs3);
    vi_write(11, (cur.be1 << 5) | cur.bs1);
    vi_write(12, (cur.be4 << 5) | cur.bs4);
    vi_write(13, (cur.be2 << 5) | cur.bs2);

    vi_write(24, 0x1000 | ((cur.nhlines / 2) + 1));
    vi_write(25, cur.hlw + 1);
    vi_write(26, 0x1001);
    vi_write(27, 0x0001);
    vi_write(36, 0x2828);

    let tvmode: u16 = 0;
    let interlace: u16 = 0;
    vi_write(1, (tvmode << 8) | (interlace << 2) | 0x0001);
    vi_write(54, 0x0000);
}

/// Switch to a user-selected mode after init. The flip chain (if in use)
/// is left intact; both slots are reprogrammed to the new timing.
pub fn set_mode(_v: &Video, standard: Standard, fm: FrameMode) -> Video {
    unsafe {
        let mode = mode_for(standard, fm);
        let front = (*core::ptr::addr_of!(CURRENT)).front;
        let front_fb = slot_ptr(front);
        configure(&mode, front_fb);
        hw::sync();
        let nv = Video { mode, framebuffer: slot_ptr(1 - front), front };
        *core::ptr::addr_of_mut!(CURRENT) = nv;
        nv
    }
}

impl Video {
    /// Switch to progressive 480p. Only valid if the sink supports it;
    /// the GameCube can't know, so this trusts the caller.
    pub fn progressive(&self) -> Video {
        set_mode(self, standard_of(&self.mode), FrameMode::Progressive)
    }

    /// The current TV standard.
    pub fn standard(&self) -> Standard {
        standard_of(&self.mode)
    }

    /// Whether the current mode is progressive.
    pub fn is_progressive(&self) -> bool {
        (self.mode.viTVMode & 3) == 2
    }
}

fn standard_of(mode: &GXRModeObj) -> Standard {
    match hw::shiftr(mode.viTVMode, 2, 3) {
        0 => Standard::Ntsc,
        1 => Standard::Pal,
        2 => Standard::Mpal,
        5 => Standard::EurRgb60,
        _ => Standard::Ntsc,
    }
}

/// VIDEO_Configure port (centered 640-wide, dispPosY=0).
fn configure(mode: &GXRModeObj, fb: *mut core::ffi::c_void) {
    let standard = standard_of(mode);
    let fm = if (mode.viTVMode & 3) == 2 { FrameMode::Progressive } else { FrameMode::Interlaced };
    let t = &TIMINGS[timing_index(standard, fm)];
    let tvmode = hw::shiftr(mode.viTVMode, 2, 3);
    let nonint = mode.viTVMode & 1;
    let progressive = fm == FrameMode::Progressive;

    unsafe {
        let mut r54 = vi_read(54) & !0x0001;
        if progressive { r54 |= 1; }
        vi_write(54, r54);
        let mut dcr = vi_read(1) & !0x030cu16;
        dcr |= ((nonint as u16) << 2) | (((tvmode as u16) & 3) << 8);
        vi_write(1, dcr);

        let disp_pos_x: u16 = 40;
        let disp_size_x = mode.viWidth;

        vi_write(2, (t.hcs << 8) | t.hce);
        vi_write(3, t.hlw);
        let val1 = (((t.hbe640 + disp_pos_x) as u32).wrapping_sub(40)) & 0x01ff;
        let val2 = (t.hbs640 as u32 + disp_pos_x as u32 + 40)
            .wrapping_sub(720 - disp_size_x as u32) & 0x03ff;
        vi_write(4, ((val1 >> 9) | (val2 << 1)) as u16);
        vi_write(5, ((val1 << 7) as u16) | t.hsy);

        vi_write(10, (t.be3 << 5) | t.bs3);
        vi_write(11, (t.be1 << 5) | t.bs1);
        vi_write(12, (t.be4 << 5) | t.bs4);
        vi_write(13, (t.be2 << 5) | t.bs2);

        vi_write(24, 0x1000 | ((t.nhlines / 2) + 1));
        vi_write(25, t.hlw + 1);

        let (div1, div2): (u32, u32) = if t.equ >= 10 { (1, 2) } else { (2, 1) };
        let prb = div2 * 0;
        let psb = div2 * ((u32::from(t.acv) * div1) - u32::from(mode.viHeight));
        let prbodd = u32::from(t.prb_odd) + prb;
        let prbeven = u32::from(t.prb_even) + prb;
        let psbodd = u32::from(t.psb_odd) + psb;
        let psbeven = u32::from(t.psb_even) + psb;
        let tmp = u32::from(mode.viHeight) / div1;
        vi_write(0, (((tmp << 4) as u16) & !0x0f) | t.equ);
        vi_write(6, psbodd as u16);
        vi_write(7, prbodd as u16);
        vi_write(8, psbeven as u16);
        vi_write(9, prbeven as u16);

        program_frame_buffers(mode, fb);

        flush();
        set_black(false);
    }
}

/// Program the VI's framebuffer base-address registers (top/bottom field)
/// plus the word-stride registers — `VIDEO_SetNextFramebuffer`-style part
/// of `VIDEO_Configure`, also used standalone by the flip chain. VI latches
/// these at the next field boundary.
unsafe fn program_frame_buffers(mode: &GXRModeObj, fb: *mut core::ffi::c_void) {
    let wpl = (u32::from(mode.fbWidth) + 15) / 16;
    let bytes_per_line = (wpl << 5) & 0x1fe0;
    let tfbb = hw::virt_to_phys(fb);
    let mut bfbb = tfbb;
    if mode.xfbMode == 1 {
        bfbb += bytes_per_line;
    }
    vi_write(14, (tfbb >> 16) as u16);
    vi_write(15, (tfbb & 0xffff) as u16);
    vi_write(18, (bfbb >> 16) as u16);
    vi_write(19, (bfbb & 0xffff) as u16);

    let std = if mode.xfbMode == 1 { wpl << 1 } else { wpl };
    vi_write(36, ((wpl as u16) << 8) | std as u16);
}

/// VIDEO_Flush.
pub fn flush() {
    unsafe { hw::sync() };
}

/// VIDEO_SetBlack.
pub fn set_black(black: bool) {
    unsafe {
        if black {
            vi_write(1, vi_read(1) & !0x0001);
        } else {
            vi_write(1, vi_read(1) | 0x0001);
        }
    }
}

fn wait_vsync_inner() {
    unsafe {
        // The VI exposes the *current display position*: regs 22 (vpos)
        // and 23 (hpos). We wait until vpos resets (new frame).
        let mut last = vi_read(22) & 0x3ff;
        loop {
            // debounce
            let mut vold = vi_read(22) & 0x3ff;
            let mut hp = vi_read(23) & 0x7ff;
            while (vi_read(22) & 0x3ff) != vold {
                vold = vi_read(22) & 0x3ff;
                hp = vi_read(23) & 0x7ff;
            }
            let v = vold;
            let h = hp;
            if last != 0 && v < last && h < 40 {
                // frame boundary
                break;
            }
            last = v;
        }
    }
}
