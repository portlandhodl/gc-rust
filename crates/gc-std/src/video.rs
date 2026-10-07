//! Video subsystem: mode selection, framebuffer allocation, vsync.

use core::ffi::c_void;

use crate::ffi::{self, GXRModeObj};

/// Handle to the initialized video subsystem.
///
/// Created by [`init`](crate::init); owns the external framebuffer that the
/// console renderer and VI are pointed at.
pub struct Video {
    mode: *mut GXRModeObj,
    framebuffer: *mut c_void,
}

impl Video {
    /// The preferred (native) render mode for the attached TV.
    pub fn mode(&self) -> &GXRModeObj {
        // SAFETY: the pointer returned by VIDEO_GetPreferredMode points to
        // one of libogc's static mode tables and outlives the program.
        unsafe { &*self.mode }
    }

    /// Raw pointer to the external framebuffer (uncached address space).
    pub fn framebuffer(&self) -> *mut c_void {
        self.framebuffer
    }

    /// Point VI at the framebuffer and take the display out of the blank
    /// screen, mirroring the tail of libogc's `Initialise()` example code.
    ///
    /// Called automatically by [`Gc::enable_console`](crate::Gc::enable_console)
    /// and [`Gc::into_gx`](crate::Gc::into_gx); call it yourself only for
    /// raw/framebuffer-only programs.
    pub fn show(&self) {
        unsafe {
            ffi::VIDEO_Configure(self.mode);
            ffi::VIDEO_SetNextFramebuffer(self.framebuffer);
            ffi::VIDEO_SetBlack(0);
            ffi::VIDEO_Flush();
            ffi::VIDEO_WaitVSync();
            if self.mode().viTVMode & ffi::VI_NON_INTERLACE != 0 {
                ffi::VIDEO_WaitVSync();
            }
        }
    }
}

pub(crate) fn init() -> Video {
    unsafe {
        ffi::VIDEO_Init();
        let mode = ffi::VIDEO_GetPreferredMode(core::ptr::null_mut());
        assert!(!mode.is_null(), "VIDEO_GetPreferredMode returned null");
        let framebuffer = ffi::mem_k0_to_k1(ffi::SYS_AllocateFramebuffer(mode));
        assert!(
            !framebuffer.is_null(),
            "SYS_AllocateFramebuffer returned null"
        );
        Video { mode, framebuffer }
    }
}

impl Video {
    /// Internal: the video "handle" is just pointers to libogc statics, so
    /// cloning it for the GX hand-over is sound.
    pub(crate) fn clone_handle(&self) -> Video {
        Video {
            mode: self.mode,
            framebuffer: self.framebuffer,
        }
    }
}

/// Block until the next vertical blanking interval.
#[inline]
pub fn wait_vsync() {
    unsafe { ffi::VIDEO_WaitVSync() }
}
