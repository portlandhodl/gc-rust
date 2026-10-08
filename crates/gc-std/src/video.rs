//! Video Interface (VI) driver — pure Rust port of the relevant parts of
//! libogc's `video.c` (zlib license, (c) devkitPro).
//!
//! Supports the mode every GC demo uses: 640×480 (NTSC) / 640×576 (PAL)
//! interlaced, double-field framebuffer, 4:2:2 YUY2. Timing values come
//! from libogc's `video_timing` table. Vsync wait polls VI's current
//! display position registers, no IRQs needed.

use crate::gctypes::GXRModeObj;
use crate::hw::{self, vi_read, vi_write};

pub const VI_NTSC: u32 = 0;
pub const VI_PAL: u32 = 1;
pub const VI_MPAL: u32 = 2;

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

// vi mode = tv<<2 | interlace. Index 0/1 = NTSC int/nint, 2/3 = PAL int/nint.
static TIMINGS: [VideoTiming; 4] = [
    // NTSC interlaced (VI_TVMODE_NTSC_INT)
    VideoTiming {
        equ: 0x06, acv: 0x00F0,
        prb_odd: 0x0018, prb_even: 0x0019, psb_odd: 0x0003, psb_even: 0x0002,
        bs1: 0x0C, bs2: 0x0D, bs3: 0x0C, bs4: 0x0D,
        be1: 0x0208, be2: 0x0207, be3: 0x0208, be4: 0x0207,
        nhlines: 0x020D, hlw: 0x01AD,
        hsy: 0x40, hcs: 0x47, hce: 0x69, hbe640: 0xA2, hbs640: 0x0175,
    },
    // NTSC non-interlaced
    VideoTiming {
        equ: 0x06, acv: 0x00F0,
        prb_odd: 0x0018, prb_even: 0x0018, psb_odd: 0x0004, psb_even: 0x0004,
        bs1: 0x0C, bs2: 0x0C, bs3: 0x0C, bs4: 0x0C,
        be1: 0x0208, be2: 0x0208, be3: 0x0208, be4: 0x0208,
        nhlines: 0x020E, hlw: 0x01AD,
        hsy: 0x40, hcs: 0x47, hce: 0x69, hbe640: 0xA2, hbs640: 0x0175,
    },
    // PAL interlaced (VI_TVMODE_PAL_INT)
    VideoTiming {
        equ: 0x05, acv: 0x0120,
        prb_odd: 0x0021, prb_even: 0x0022, psb_odd: 0x0001, psb_even: 0x0000,
        bs1: 0x0D, bs2: 0x0C, bs3: 0x0B, bs4: 0x0A,
        be1: 0x026B, be2: 0x026A, be3: 0x0269, be4: 0x026C,
        nhlines: 0x0271, hlw: 0x01B0,
        hsy: 0x40, hcs: 0x4B, hce: 0x6A, hbe640: 0xAC, hbs640: 0x017C,
    },
    // PAL non-interlaced
    VideoTiming {
        equ: 0x05, acv: 0x0120,
        prb_odd: 0x0021, prb_even: 0x0021, psb_odd: 0x0000, psb_even: 0x0000,
        bs1: 0x0D, bs2: 0x0B, bs3: 0x0D, bs4: 0x0B,
        be1: 0x026B, be2: 0x026D, be3: 0x026B, be4: 0x026D,
        nhlines: 0x0270, hlw: 0x01B0,
        hsy: 0x40, hcs: 0x4B, hce: 0x6A, hbe640: 0xAC, hbs640: 0x017C,
    },
];

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
    framebuffer: *mut core::ffi::c_void,
}

impl Video {
    pub fn mode(&self) -> &GXRModeObj {
        &self.mode
    }
    /// Uncached pointer to the external framebuffer (YUY2).
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
};

fn int_df_mode(tv: u32) -> GXRModeObj {
    let height = if tv == VI_PAL { 576u16 } else { 480u16 };
    GXRModeObj {
        viTVMode: tv << 2, // interlaced
        fbWidth: 640,
        efbHeight: height,
        xfbHeight: height,
        viXOrigin: 40,
        viYOrigin: 0,
        viWidth: 640,
        viHeight: height,
        xfbMode: 1, // DF
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
        let tvmode = hw::shiftr(u32::from(dcr), 8, 2);

        let mode = int_df_mode(tvmode);
        // The XFB lives at a *fixed* cached address from the linker script;
        // the heap ends just below it. This keeps the framebuffer at a
        // known location for tests and Dual-homing GX copies.
        extern "C" {
            static __xfb_base: u32;
        }
        let fb_cached = &raw const __xfb_base as *mut u8;
        let size = usize::from(mode.fbWidth) * usize::from(mode.xfbHeight) * 2;
        let fb = hw::cached_to_uncached(fb_cached as *mut core::ffi::c_void);
        core::ptr::write_bytes(fb, 0x10, size); // black (uncached view)

        configure(&mode, fb);
        hw::dc_flush_range(fb, size);

        CURRENT = Video { mode, framebuffer: fb };
        CURRENT
    }
}

/// `__VIInit` port for a timing slot.
unsafe fn vi_reset_with(idx: u32) {
    let cur = &TIMINGS[(idx & 3) as usize];
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

    let tvmode = (idx >> 2) as u16;
    let interlace = (idx & 1) as u16;
    vi_write(1, (tvmode << 8) | (interlace << 2) | 0x0001);
    vi_write(54, 0x0000);
}

/// VIDEO_Configure port (centered 640-wide, dispPosY=0, DF XFB).
fn configure(mode: &GXRModeObj, fb: *mut core::ffi::c_void) {
    let t = &TIMINGS[(mode.viTVMode & 3) as usize];
    let tvmode = hw::shiftr(mode.viTVMode, 2, 3);
    let nonint = mode.viTVMode & 1;

    unsafe {
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
        vi_write(5, (((val1 << 7) as u16) | t.hsy));

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

        flush();
        set_black(false);
    }
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
