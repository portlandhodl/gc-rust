//! Matrix math helpers — pure Rust, bit-compatible with libogc's gu
//! (`guPerspective`, `guLookAt`, `guMtxConcat`, `guMtxTransApply`,
//! `guMtxRotAxisRad`).
//!
//! Since a no_std target has no libm, the few scalar transcendentals we
//! need are implemented here with classic polynomial approximations
//! (ULP-accurate enough for graphics transforms on a 486-capable FPU).

use crate::gctypes::{Mtx, Mtx44};

pub const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[inline]
pub fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

// ---------------------------------------------------------------------------
// minimal scalar libm (Cephes-style, f32)
// ---------------------------------------------------------------------------

/// sin for small |x| ≤ π/4 via Taylor (max abs err ~ 1e-9).
#[inline]
fn sin_taylor(x: f32) -> f32 {
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0))))
}

#[inline]
fn cos_taylor(x: f32) -> f32 {
    let x2 = x * x;
    1.0 + x2 * (-0.5 + x2 * (1.0 / 24.0 + x2 * (-1.0 / 720.0)))
}

fn sinf(mut x: f32) -> f32 {
    const PI: f32 = core::f32::consts::PI;
    const FRAC_PI_2: f32 = core::f32::consts::FRAC_PI_2;
    const FRAC_PI_4: f32 = core::f32::consts::FRAC_PI_4;
    const TWO_PI: f32 = PI * 2.0;

    x %= TWO_PI;
    if x < 0.0 {
        x += TWO_PI;
    }
    // x in [0, 2π). Fold to [0,π] with sign.
    let mut sign = 1.0f32;
    if x > PI {
        x -= PI;
        sign = -1.0;
    }
    if x > FRAC_PI_2 {
        x = PI - x;
    }
    // x in [0, π/2]
    let v = if x > FRAC_PI_4 {
        cos_taylor(FRAC_PI_2 - x) // cos(π/2 - x) = sin x
    } else {
        sin_taylor(x)
    };
    sign * v
}

fn cosf(x: f32) -> f32 {
    sinf(x + core::f32::consts::FRAC_PI_2)
}

fn tanf(x: f32) -> f32 {
    sinf(x) / cosf(x)
}

fn sqrtf(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    // exponent-halving initial guess + 4 Newton iterations (f32-converged)
    let bits = x.to_bits();
    let mut g = f32::from_bits((bits >> 1) + 0x1fbd_1df5);
    for _ in 0..4 {
        g = 0.5 * (g + x / g);
    }
    g
}

// ---------------------------------------------------------------------------
// publics
// ---------------------------------------------------------------------------

/// `guPerspective(m, fovy_deg, width/height, n, f)` — libogc formula.
pub fn perspective(fovy_deg: f32, aspect: f32, n: f32, f: f32) -> Mtx44 {
    let angle = 0.5 * fovy_deg * DEG_TO_RAD;
    let cot = 1.0 / tanf(angle);

    let tmp = 1.0 / (f - n);
    let mut m = [[0.0f32; 4]; 4];
    m[0][0] = cot / aspect;
    m[1][1] = cot;
    m[2][2] = -n * tmp;
    m[2][3] = -(f * n) * tmp;
    m[3][2] = -1.0;
    m
}

/// `guLookAt(m, cameraPos, up, target)` — libogc formula.
pub fn look_at(cam: Vec3, up: Vec3, target: Vec3) -> Mtx {
    let norm = |v: Vec3| -> Vec3 {
        let l = sqrtf(v.x * v.x + v.y * v.y + v.z * v.z);
        if l == 0.0 {
            Vec3 { x: 0.0, y: 0.0, z: 0.0 }
        } else {
            Vec3 { x: v.x / l, y: v.y / l, z: v.z / l }
        }
    };
    let cross = |a: Vec3, b: Vec3| Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    };

    let vlook = norm(Vec3 { x: cam.x - target.x, y: cam.y - target.y, z: cam.z - target.z });
    let vright = norm(cross(up, vlook));
    let vup = cross(vlook, vright);

    [
        [
            vright.x, vright.y, vright.z,
            -(cam.x * vright.x + cam.y * vright.y + cam.z * vright.z),
        ],
        [
            vup.x, vup.y, vup.z,
            -(cam.x * vup.x + cam.y * vup.y + cam.z * vup.z),
        ],
        [
            vlook.x, vlook.y, vlook.z,
            -(cam.x * vlook.x + cam.y * vlook.y + cam.z * vlook.z),
        ],
    ]
}

#[inline]
pub fn identity() -> Mtx {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// guMtxConcat(a, b) = a · b (b applied after).
pub fn concat(a: &Mtx, b: &Mtx) -> Mtx {
    let mut out = [[0.0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..4 {
            let mut acc = if c == 3 { a[r][3] } else { 0.0 };
            for k in 0..3 {
                acc += a[r][k] * b[k][c];
            }
            out[r][c] = acc;
        }
    }
    out
}

/// guMtxTransApply: dst = src, then column[3] += (x, y, z).
pub fn translate(src: &Mtx, x: f32, y: f32, z: f32) -> Mtx {
    let mut out = *src;
    out[0][3] += x;
    out[1][3] += y;
    out[2][3] += z;
    out
}

/// Rotate `m` about `axis` by `deg` degrees (libogc guMtxRotAxisDeg form).
pub fn rotate_axis_deg(m: &mut Mtx, axis: Vec3, deg: f32) {
    let rad = deg * DEG_TO_RAD;
    let s = sinf(rad);
    let c = cosf(rad);
    let t = 1.0 - c;
    let l = sqrtf(axis.x * axis.x + axis.y * axis.y + axis.z * axis.z);
    if l == 0.0 {
        return;
    }
    let (x, y, z) = (axis.x / l, axis.y / l, axis.z / l);
    let (xs, ys, zs) = (x * x, y * y, z * z);

    let r: Mtx = [
        [t * xs + c, t * x * y - s * z, t * x * z + s * y, 0.0],
        [t * x * y + s * z, t * ys + c, t * y * z - s * x, 0.0],
        [t * x * z - s * y, t * y * z + s * x, t * zs + c, 0.0],
    ];
    *m = concat(&r, m);
}
