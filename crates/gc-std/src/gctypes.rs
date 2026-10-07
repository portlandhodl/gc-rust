//! Common GameCube hardware types (values identical to libogc's
//! `gctypes.h`/`video_types.h`; the structs are trivially portable because
//! they're plain field bags of ints).

#![allow(non_snake_case)]

/// `GXRModeObj`: video-mode descriptor, layout like libogc.
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

/// `GXColor`.
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

/// `Mtx` — libogc's 3x4 row-major transform matrix.
pub type Mtx = [[f32; 4]; 3];
/// `Mtx44` — 4x4 projection matrix.
pub type Mtx44 = [[f32; 4]; 4];
