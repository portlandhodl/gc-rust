//! Matrix math helpers, wrapping libogc's `gu` routines (`ogc/gu.h`).
//!
//! Matrices follow libogc conventions: [`Mtx`] is a 3x4 row-major
//! position/normal matrix and [`Mtx44`] a 4x4 projection matrix. All
//! functions forward to the paired-single (Gekko SIMD) implementations in
//! libogc.

use crate::ffi::{self, guVector, Mtx, Mtx44};

pub use crate::ffi::guVector as Vec3;

/// Degrees -> radians (`DegToRad` in libogc).
pub const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;

#[inline]
pub fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    guVector { x, y, z }
}

/// `guPerspective`: horizontal/vertical-symmetric perspective projection.
/// `fovy` is the full vertical field of view **in degrees**, `aspect` is
/// width/height, `n`/`f` the near and far clip distances.
pub fn perspective(fovy: f32, aspect: f32, n: f32, f: f32) -> Mtx44 {
    let mut m = [[0.0; 4]; 4];
    unsafe { ffi::guPerspective(&mut m, fovy, aspect, n, f) };
    m
}

/// `guLookAt`: view matrix looking from `cam` towards `look` with `up`.
pub fn look_at(cam: Vec3, up: Vec3, look: Vec3) -> Mtx {
    let mut m = [[0.0; 4]; 3];
    unsafe { ffi::guLookAt(&mut m, &cam, &up, &look) };
    m
}

/// `guMtxIdentity`.
pub fn identity() -> Mtx {
    let mut m = [[0.0; 4]; 3];
    unsafe { ffi::ps_guMtxIdentity(&mut m) };
    m
}

/// `guMtxConcat(a, b)`: `a * b` (apply `b` first, then `a`).
pub fn concat(a: &Mtx, b: &Mtx) -> Mtx {
    let mut m = [[0.0; 4]; 3];
    unsafe { ffi::ps_guMtxConcat(a, b, &mut m) };
    m
}

/// `guMtxTransApply`: apply a translation to `m`.
pub fn translate(m: &Mtx, x: f32, y: f32, z: f32) -> Mtx {
    let mut out = [[0.0; 4]; 3];
    unsafe { ffi::ps_guMtxTransApply(m, &mut out, x, y, z) };
    out
}

/// `guMtxRotAxisDeg`: rotate `m` about `axis` by `deg` degrees.
pub fn rotate_axis_deg(m: &mut Mtx, axis: Vec3, deg: f32) {
    unsafe { ffi::ps_guMtxRotAxisRad(m, &axis, deg * DEG_TO_RAD) };
}
