//! Example 23 — yarn-cat: a low-poly orange cat that lives inside your TV.
//!
//! A little room (wallpaper, window with a night sky, rug, cat bed) seen
//! through a cream retro-TV bezel. The cat chases a ball of yarn, crouches,
//! wiggles its rear, swats the ball across the room, then sits and watches
//! it roll while its tail swishes. The ball leaves a strand of yarn behind.
//!
//! Rendering is the gx-lit-cube approach scaled up: every mesh (spheres,
//! prisms, a torus) is built on the CPU in world space, flat-shaded per
//! face with a half-lambert key light for a soft "toy" look, collected
//! into one triangle batch and streamed through immediate mode.
//!
//! Controls: A tosses the ball, the stick nudges it, START exits.

#![no_std]
#![no_main]

use core::f32::consts::PI;
use core::ptr::addr_of_mut;

use gc_std::gctypes::Mtx;
use gc_std::{gu, gx, input::button};

type V3 = [f32; 3];
type Rgb = [u8; 3];

// ---------------------------------------------------------------------------
// palette
// ---------------------------------------------------------------------------

const ORANGE: Rgb = [246, 146, 44];
const ORANGE_DARK: Rgb = [214, 98, 24];
const CREAM: Rgb = [255, 236, 204];
const PINK: Rgb = [255, 150, 172];
const BLUSH: Rgb = [255, 158, 170];
const EYE: Rgb = [34, 26, 40];
const WHITE: Rgb = [255, 255, 255];
const YARN_A: Rgb = [92, 168, 238];
const YARN_B: Rgb = [140, 200, 255];
const YARN_BAND: Rgb = [60, 120, 210];

/// Hardware bisection builds: `YARN_DIAG=fifo|nodecal|nobezel cargo build`
/// toggles one suspect at a time; `YARN_DIAG=photo` starts the cat facing
/// the camera with the yarn in front of it (README screenshots). Normal
/// builds leave it unset.
fn diag(which: &str) -> bool {
    option_env!("YARN_DIAG") == Some(which)
}

// ---------------------------------------------------------------------------
// small vector / scalar helpers (no libm on this target)
// ---------------------------------------------------------------------------

fn sin(x: f32) -> f32 {
    gu::sinf(x)
}
fn cos(x: f32) -> f32 {
    gu::cosf(x)
}
fn sqrt(x: f32) -> f32 {
    gu::sqrtf(x)
}
fn absf(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}
fn clampf(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}
fn max0(x: f32) -> f32 {
    if x > 0.0 {
        x
    } else {
        0.0
    }
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// atan2 polynomial approximation (max error ~1e-4 rad).
fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (absf(x), absf(y));
    if ax < 1e-9 && ay < 1e-9 {
        return 0.0;
    }
    let (a, swap) = if ax > ay { (ay / ax, false) } else { (ax / ay, true) };
    let s = a * a;
    let mut r = ((-0.046_496_475 * s + 0.159_314_22) * s - 0.327_622_76) * s * a + a;
    if swap {
        r = PI / 2.0 - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 {
        r = -r;
    }
    r
}

/// Wrap an angle in degrees to (-180, 180].
fn wrap180(mut d: f32) -> f32 {
    while d > 180.0 {
        d -= 360.0;
    }
    while d <= -180.0 {
        d += 360.0;
    }
    d
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: V3) -> V3 {
    let l = sqrt(dot(v, v));
    if l < 1e-12 {
        [0.0, 1.0, 0.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

// ---------------------------------------------------------------------------
// matrices (3x4, libogc layout; child = parent · local)
// ---------------------------------------------------------------------------

fn t(x: f32, y: f32, z: f32) -> Mtx {
    gu::translate(&gu::identity(), x, y, z)
}
fn rot(axis: gu::Vec3, deg: f32) -> Mtx {
    let mut m = gu::identity();
    gu::rotate_axis_deg(&mut m, axis, deg);
    m
}
fn rx(deg: f32) -> Mtx {
    rot(gu::vec3(1.0, 0.0, 0.0), deg)
}
fn ry(deg: f32) -> Mtx {
    rot(gu::vec3(0.0, 1.0, 0.0), deg)
}
fn rz(deg: f32) -> Mtx {
    rot(gu::vec3(0.0, 0.0, 1.0), deg)
}
fn s(x: f32, y: f32, z: f32) -> Mtx {
    [[x, 0.0, 0.0, 0.0], [0.0, y, 0.0, 0.0], [0.0, 0.0, z, 0.0]]
}
fn chain(ms: &[Mtx]) -> Mtx {
    let mut out = gu::identity();
    for m in ms {
        out = gu::concat(&out, m);
    }
    out
}
fn xf(m: &Mtx, p: V3) -> V3 {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

/// Re-orthonormalize the rotation part (the rolling ball accumulates a
/// rotation every frame; f32 drift would otherwise shear it).
fn orthonormalize(m: &mut Mtx) {
    let r0 = normalize([m[0][0], m[0][1], m[0][2]]);
    let r1 = [m[1][0], m[1][1], m[1][2]];
    let d = dot(r0, r1);
    let r1 = normalize([r1[0] - r0[0] * d, r1[1] - r0[1] * d, r1[2] - r0[2] * d]);
    let r2 = cross(r0, r1);
    for (row, r) in [r0, r1, r2].iter().enumerate() {
        m[row][0] = r[0];
        m[row][1] = r[1];
        m[row][2] = r[2];
    }
}

// ---------------------------------------------------------------------------
// triangle batch + shading
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct Tri {
    p: [V3; 3],
    c: [Rgb; 3],
}

const MAX_TRIS: usize = 4096;
static mut TRIS: [Tri; MAX_TRIS] = [Tri { p: [[0.0; 3]; 3], c: [[0; 3]; 3] }; MAX_TRIS];

struct Batch {
    tris: &'static mut [Tri; MAX_TRIS],
    n: usize,
    key: V3,
}

impl Batch {
    fn raw(&mut self, p: [V3; 3], c: [Rgb; 3]) {
        if self.n < MAX_TRIS {
            self.tris[self.n] = Tri { p, c };
            self.n += 1;
        }
    }

    fn flat(&mut self, a: V3, b: V3, c: V3, col: Rgb) {
        self.raw([a, b, c], [col, col, col]);
    }

    /// Flat-shaded triangle. The face normal is oriented away from `refp`
    /// (or towards it when `inward`), so winding order never matters.
    fn lit(&mut self, a: V3, b: V3, c: V3, col: Rgb, refp: V3, inward: bool) {
        let mut n = normalize(cross(sub(b, a), sub(c, a)));
        let cen = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let mut d = dot(n, sub(cen, refp));
        if inward {
            d = -d;
        }
        if d < 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        let sc = self.shade(n, col);
        self.flat(a, b, c, sc);
    }

    fn lit_quad(&mut self, a: V3, b: V3, c: V3, d: V3, col: Rgb, refp: V3, inward: bool) {
        self.lit(a, b, c, col, refp, inward);
        self.lit(a, c, d, col, refp, inward);
    }

    /// Half-lambert warm key + cool ambient: soft and toy-like.
    fn shade(&self, n: V3, col: Rgb) -> Rgb {
        let hl = dot(n, self.key) * 0.5 + 0.5;
        let hl = hl * hl;
        let ch = |c: u8, amb: f32, key: f32| -> u8 {
            clampf(f32::from(c) * (amb + key * hl), 0.0, 255.0) as u8
        };
        [ch(col[0], 0.38, 0.80), ch(col[1], 0.38, 0.74), ch(col[2], 0.46, 0.64)]
    }

    fn submit(&mut self) {
        let fifo_safe = diag("fifo");
        let mut start = 0;
        while start < self.n {
            let k = core::cmp::min(self.n - start, if fifo_safe { 128 } else { 4000 });
            gx::begin(gx::Primitive::Triangles, gx::VTXFMT0, (k * 3) as u16);
            for tri in &self.tris[start..start + k] {
                for v in 0..3 {
                    let p = tri.p[v];
                    let c = tri.c[v];
                    gx::position3f32(p[0], p[1], p[2]);
                    gx::color4u8(c[0], c[1], c[2], 255);
                }
            }
            start += k;
            if fifo_safe {
                gx::wait_gp_idle();
            }
        }
        self.n = 0;
    }
}

// ---------------------------------------------------------------------------
// mesh builders
// ---------------------------------------------------------------------------

const MAX_RING: usize = 17;

/// Low-poly UV sphere of radius 1 under `m`. `col(stack, slice)` picks the
/// per-face color; `glow` skips lighting (eye highlights).
fn sphere(b: &mut Batch, m: &Mtx, slices: usize, stacks: usize, glow: bool, col: &dyn Fn(usize, usize) -> Rgb) {
    let mut st = [(0.0f32, 0.0f32); MAX_RING];
    let mut sl = [(0.0f32, 0.0f32); MAX_RING];
    for i in 0..=stacks {
        let a = PI * i as f32 / stacks as f32;
        st[i] = (sin(a), cos(a));
    }
    for j in 0..=slices {
        let a = 2.0 * PI * j as f32 / slices as f32;
        sl[j] = (sin(a), cos(a));
    }
    let p = |i: usize, j: usize| -> V3 {
        let (sp, cp) = st[i];
        let (sth, cth) = sl[j];
        xf(m, [sp * cth, cp, sp * sth])
    };
    let center = xf(m, [0.0; 3]);
    for i in 0..stacks {
        for j in 0..slices {
            let (a, bb, c, d) = (p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j));
            let k = col(i, j);
            if glow {
                if i > 0 {
                    b.flat(a, bb, c, k);
                }
                if i + 1 < stacks {
                    b.flat(a, c, d, k);
                }
            } else {
                if i > 0 {
                    b.lit(a, bb, c, k, center, false);
                }
                if i + 1 < stacks {
                    b.lit(a, c, d, k, center, false);
                }
            }
        }
    }
}

fn sphere1(b: &mut Batch, m: &Mtx, slices: usize, stacks: usize, col: Rgb) {
    sphere(b, m, slices, stacks, false, &|_, _| col);
}

/// Tapered n-gon prism along +Y from 0 to `h` (r1 = 0 makes a pyramid).
fn prism(b: &mut Batch, m: &Mtx, sides: usize, r0: f32, r1: f32, h: f32, col: Rgb) {
    let mut ring = [(0.0f32, 0.0f32); MAX_RING];
    for j in 0..=sides {
        let a = 2.0 * PI * j as f32 / sides as f32;
        ring[j] = (cos(a), sin(a));
    }
    let center = xf(m, [0.0, h * 0.5, 0.0]);
    let bot_c = xf(m, [0.0, 0.0, 0.0]);
    let top_c = xf(m, [0.0, h, 0.0]);
    for j in 0..sides {
        let (c0, s0) = ring[j];
        let (c1, s1) = ring[j + 1];
        let a = xf(m, [r0 * c0, 0.0, r0 * s0]);
        let bb = xf(m, [r0 * c1, 0.0, r0 * s1]);
        if r1 > 0.0 {
            let c = xf(m, [r1 * c1, h, r1 * s1]);
            let d = xf(m, [r1 * c0, h, r1 * s0]);
            b.lit_quad(a, bb, c, d, col, center, false);
            b.lit(d, c, top_c, col, center, false);
        } else {
            b.lit(a, bb, top_c, col, center, false);
        }
        b.lit(a, bb, bot_c, col, center, false);
    }
}

/// Torus in the XZ plane (the cat bed's cushion ring).
fn torus(b: &mut Batch, m: &Mtx, seg: usize, side: usize, big_r: f32, small_r: f32, col: &dyn Fn(usize) -> Rgb) {
    let p = |u: f32, v: f32| -> V3 {
        let rr = big_r + small_r * cos(v);
        xf(m, [rr * cos(u), small_r * sin(v), rr * sin(u)])
    };
    for i in 0..seg {
        let u0 = 2.0 * PI * i as f32 / seg as f32;
        let u1 = 2.0 * PI * (i + 1) as f32 / seg as f32;
        let um = (u0 + u1) * 0.5;
        let tube = xf(m, [big_r * cos(um), 0.0, big_r * sin(um)]);
        for j in 0..side {
            let v0 = 2.0 * PI * j as f32 / side as f32;
            let v1 = 2.0 * PI * (j + 1) as f32 / side as f32;
            b.lit_quad(p(u0, v0), p(u1, v0), p(u1, v1), p(u0, v1), col(i), tube, false);
        }
    }
}

/// Flat ellipse lying on the floor (rugs, shadows), under `m`.
fn floor_disc(b: &mut Batch, m: &Mtx, sides: usize, rx_: f32, rz_: f32, y: f32, col: Rgb) {
    let c = xf(m, [0.0, y, 0.0]);
    for j in 0..sides {
        let a0 = 2.0 * PI * j as f32 / sides as f32;
        let a1 = 2.0 * PI * (j + 1) as f32 / sides as f32;
        let p0 = xf(m, [rx_ * cos(a0), y, rz_ * sin(a0)]);
        let p1 = xf(m, [rx_ * cos(a1), y, rz_ * sin(a1)]);
        b.flat(c, p0, p1, col);
    }
}

/// Flat disc in the XY plane (view-space bezel knobs, hearts, moon).
fn xy_disc(b: &mut Batch, cx: f32, cy: f32, z: f32, r: f32, sides: usize, col: Rgb) {
    for j in 0..sides {
        let a0 = 2.0 * PI * j as f32 / sides as f32;
        let a1 = 2.0 * PI * (j + 1) as f32 / sides as f32;
        b.flat(
            [cx, cy, z],
            [cx + r * cos(a0), cy + r * sin(a0), z],
            [cx + r * cos(a1), cy + r * sin(a1), z],
            col,
        );
    }
}

fn xy_rect(b: &mut Batch, x0: f32, y0: f32, x1: f32, y1: f32, z: f32, top: Rgb, bot: Rgb) {
    b.raw([[x0, y0, z], [x1, y0, z], [x1, y1, z]], [bot, bot, top]);
    b.raw([[x0, y0, z], [x1, y1, z], [x0, y1, z]], [bot, top, top]);
}

/// Flat ribbon on the floor from p to q (a strand of yarn).
fn ribbon(b: &mut Batch, p: V3, q: V3, w: f32, col: Rgb) {
    let dx = q[0] - p[0];
    let dz = q[2] - p[2];
    let l = sqrt(dx * dx + dz * dz);
    if l < 1e-5 {
        return;
    }
    let (nx, nz) = (-dz / l * w, dx / l * w);
    let a = [p[0] + nx, p[1], p[2] + nz];
    let bb = [p[0] - nx, p[1], p[2] - nz];
    let c = [q[0] - nx, q[1], q[2] - nz];
    let d = [q[0] + nx, q[1], q[2] + nz];
    b.flat(a, bb, c, col);
    b.flat(a, c, d, col);
}

// ---------------------------------------------------------------------------
// world
// ---------------------------------------------------------------------------

const ROOM_X: f32 = 2.4;
const ROOM_BACK: f32 = -2.2;
const ROOM_FRONT: f32 = 1.2;
/// The floor runs past the play area towards the viewer so its front
/// edge never shows inside the bezel — but stays >= ~1.5 units in front of
/// the camera so nothing straddles the near plane (real GX clipping of
/// near-crossing triangles is far less forgiving than Dolphin's).
const FLOOR_FRONT: f32 = 3.2;
const WALL_H: f32 = 3.4;
const BALL_R: f32 = 0.2;
const BED_R: f32 = 0.62;
/// Tucked into the back-left corner so its collision circle (grown by the
/// ball radius) meets both wall bounds.
const BED: (f32, f32) = (
    -(ROOM_X - BALL_R - 0.05) + BED_R + BALL_R,
    (ROOM_BACK + BALL_R + 0.05) + BED_R + BALL_R,
);

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// uniform in [0, 1)
    fn f(&mut self) -> f32 {
        (self.next() >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
}

#[derive(Copy, Clone, PartialEq)]
enum State {
    Chase,
    Crouch,
    Swat,
    Watch,
}

/// A damped spring for secondary motion: chases `target` with a little
/// overshoot, so nothing snaps. `k` = stiffness, `d` = damping (per frame).
#[derive(Copy, Clone, Default)]
struct Spring {
    x: f32,
    v: f32,
}

impl Spring {
    fn go(&mut self, target: f32, k: f32, d: f32) {
        self.v += (target - self.x) * k;
        self.v *= 1.0 - d;
        self.x += self.v;
    }
}

const TAIL_SEGS: usize = 7;
/// Spine joints from shoulders (0) to hips (SPINE - 1).
const SPINE: usize = 4;
/// Distance between spine joints along the back.
const SPINE_SEG: f32 = 0.15;
/// How far neighbouring vertebrae may bend relative to each other (deg).
const SPINE_MAX_BEND: f32 = 30.0;

struct Cat {
    x: f32,
    z: f32,
    heading: f32, // degrees, 0 = facing +Z (towards the camera)
    state: State,
    timer: u32,
    watch_len: u32,
    /// forward speed (units/frame), eased towards the state's target
    speed: f32,
    /// turn rate (degrees/frame), eased so turns start and stop smoothly
    turn: f32,
    walk_phase: f32,
    walk_amt: f32,
    crouch: Spring,
    sit: Spring,
    /// swat paw: <0 wind-up, 1 = full strike
    paw: f32,
    /// forward lunge of the chest during a swat
    lunge: f32,
    /// pre-pounce butt-wiggle amplitude and phase; the shake starts at the
    /// hips and travels up the spine, fading towards the shoulders
    wiggle: f32,
    wiggle_t: f32,
    head_yaw: Spring,
    head_pitch: Spring,
    head_tilt: Spring,
    /// World yaw (deg) of each spine joint, shoulders first. Joint 0 is
    /// the heading; every joint behind chases the one in front of it, so
    /// turns ripple down the back and the body curls into a C.
    spine: [f32; SPINE],
    spine_v: [f32; SPINE],
    /// body roll (deg): leans into turns
    lean: Spring,
    /// per-segment tail sway that lags the body (follow-through)
    tail: [Spring; TAIL_SEGS],
    ear_perk: Spring,
    blink_in: u32,
    blink: u32,
    ear_twitch: f32,
}

impl Cat {
    fn new(x: f32, z: f32, heading: f32) -> Cat {
        Cat {
            x,
            z,
            heading,
            state: State::Chase,
            timer: 0,
            watch_len: 0,
            speed: 0.0,
            turn: 0.0,
            walk_phase: 0.0,
            walk_amt: 0.0,
            crouch: Spring::default(),
            sit: Spring::default(),
            paw: 0.0,
            lunge: 0.0,
            wiggle: 0.0,
            wiggle_t: 0.0,
            head_yaw: Spring::default(),
            head_pitch: Spring::default(),
            head_tilt: Spring::default(),
            spine: [heading; SPINE],
            spine_v: [0.0; SPINE],
            lean: Spring::default(),
            tail: [Spring::default(); TAIL_SEGS],
            ear_perk: Spring::default(),
            blink_in: 90,
            blink: 0,
            ear_twitch: 0.0,
        }
    }
}

struct Ball {
    x: f32,
    z: f32,
    vx: f32,
    vz: f32,
    rot: Mtx,
    /// frames left during which the cat's body doesn't shove the ball
    /// (so a swat isn't immediately cancelled by the swatting cat)
    free: u32,
}

const TRAIL: usize = 48;

struct Trail {
    pts: [(f32, f32); TRAIL],
    head: usize,
    len: usize,
}

impl Trail {
    fn last(&self) -> Option<(f32, f32)> {
        if self.len == 0 {
            None
        } else {
            Some(self.pts[(self.head + TRAIL - 1) % TRAIL])
        }
    }
    fn push(&mut self, p: (f32, f32)) {
        self.pts[self.head] = p;
        self.head = (self.head + 1) % TRAIL;
        if self.len < TRAIL {
            self.len += 1;
        }
    }
    /// i = 0 is the oldest point.
    fn get(&self, i: usize) -> (f32, f32) {
        self.pts[(self.head + TRAIL - self.len + i) % TRAIL]
    }
}

#[derive(Copy, Clone)]
struct Heart {
    x: f32,
    y: f32,
    z: f32,
    age: u32,
}

// ---------------------------------------------------------------------------
// simulation
// ---------------------------------------------------------------------------

fn bounce_ball(ball: &mut Ball) {
    let (xmax, zmin, zmax) = (ROOM_X - BALL_R - 0.05, ROOM_BACK + BALL_R + 0.05, ROOM_FRONT - BALL_R);
    if ball.x > xmax {
        ball.x = xmax;
        ball.vx = -absf(ball.vx) * 0.7;
    }
    if ball.x < -xmax {
        ball.x = -xmax;
        ball.vx = absf(ball.vx) * 0.7;
    }
    if ball.z > zmax {
        ball.z = zmax;
        ball.vz = -absf(ball.vz) * 0.7;
    }
    if ball.z < zmin {
        ball.z = zmin;
        ball.vz = absf(ball.vz) * 0.7;
    }
    // the cat bed rim; the corner quadrant behind its center counts as bed
    // too, otherwise the yarn can wedge itself out of the cat's reach
    if ball.x < BED.0 && ball.z < BED.1 {
        if BED.0 - ball.x < BED.1 - ball.z {
            ball.x = BED.0;
            ball.vx = absf(ball.vx);
        } else {
            ball.z = BED.1;
            ball.vz = absf(ball.vz);
        }
    }
    let (dx, dz) = (ball.x - BED.0, ball.z - BED.1);
    let d = sqrt(dx * dx + dz * dz);
    let rr = BED_R + BALL_R;
    if d < rr && d > 1e-4 {
        let (nx, nz) = (dx / d, dz / d);
        ball.x = BED.0 + nx * rr;
        ball.z = BED.1 + nz * rr;
        let vn = ball.vx * nx + ball.vz * nz;
        if vn < 0.0 {
            ball.vx -= 1.7 * vn * nx;
            ball.vz -= 1.7 * vn * nz;
        }
    }
}

fn step_ball(ball: &mut Ball, trail: &mut Trail) {
    ball.x += ball.vx;
    ball.z += ball.vz;
    bounce_ball(ball);
    ball.vx *= 0.978;
    ball.vz *= 0.978;

    // roll: rotate about the horizontal axis perpendicular to travel
    let dist = sqrt(ball.vx * ball.vx + ball.vz * ball.vz);
    if dist > 1e-5 {
        let axis = gu::vec3(ball.vz / dist, 0.0, -ball.vx / dist);
        let deg = dist / BALL_R * (180.0 / PI);
        gu::rotate_axis_deg(&mut ball.rot, axis, deg);
        orthonormalize(&mut ball.rot);
    }

    match trail.last() {
        Some((lx, lz)) => {
            let (dx, dz) = (ball.x - lx, ball.z - lz);
            if dx * dx + dz * dz > 0.09 * 0.09 {
                trail.push((ball.x, ball.z));
            }
        }
        None => trail.push((ball.x, ball.z)),
    }
}

fn kick(ball: &mut Ball, dir_deg: f32, speed: f32) {
    let r = dir_deg * (PI / 180.0);
    ball.vx = sin(r) * speed;
    ball.vz = cos(r) * speed;
    ball.free = 30;
}

fn ease_out(t: f32) -> f32 {
    let t = clampf(t, 0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

fn ease_in_out(t: f32) -> f32 {
    let t = clampf(t, 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Spine dynamics. The shoulders (joint 0) take the heading; each joint
/// behind springs towards the one in front of it (plus a small travelling
/// wave while trotting), walking forward drags the rear into line like a
/// follow-the-leader chain, and neighbouring joints can only bend so far.
fn step_spine(cat: &mut Cat) {
    cat.spine[0] = cat.heading;
    for k in 1..SPINE {
        let rel = wrap180(cat.spine[k - 1] - cat.spine[k]);
        // a trotting cat's back sways side to side, phase-shifted per joint
        let wave = sin(cat.walk_phase - k as f32 * 0.9) * 4.0 * cat.walk_amt;
        cat.spine_v[k] += (rel - wave) * 0.16;
        cat.spine_v[k] *= 1.0 - 0.36;
        // moving forward pulls the hips round behind the shoulders
        cat.spine_v[k] += rel * clampf(cat.speed * 5.0, 0.0, 0.25) * 0.5;
        cat.spine[k] = wrap180(cat.spine[k] + cat.spine_v[k]);
        let rel = wrap180(cat.spine[k - 1] - cat.spine[k]);
        if absf(rel) > SPINE_MAX_BEND {
            let lim = if rel > 0.0 { SPINE_MAX_BEND } else { -SPINE_MAX_BEND };
            cat.spine[k] = wrap180(cat.spine[k - 1] - lim);
            cat.spine_v[k] *= 0.5;
        }
    }
}

/// World positions of the spine joints: joint 0 sits over the shoulders,
/// each following joint one segment behind along the mean of the two
/// joints' directions (so the back forms a smooth arc).
fn spine_nodes(cat: &Cat) -> [(f32, f32); SPINE] {
    let dir = |deg: f32| {
        let r = deg * (PI / 180.0);
        (sin(r), cos(r))
    };
    let (fx, fz) = dir(cat.spine[0]);
    let lead = 0.24 + cat.lunge;
    let mut nodes = [(cat.x + fx * lead, cat.z + fz * lead); SPINE];
    for k in 1..SPINE {
        let mid = cat.spine[k] + wrap180(cat.spine[k - 1] - cat.spine[k]) * 0.5;
        let (dx, dz) = dir(mid);
        nodes[k] = (nodes[k - 1].0 - dx * SPINE_SEG, nodes[k - 1].1 - dz * SPINE_SEG);
    }
    nodes
}

fn step_cat(cat: &mut Cat, ball: &mut Ball, rng: &mut Rng, hearts: &mut [Heart; 4]) {
    let (dx, dz) = (ball.x - cat.x, ball.z - cat.z);
    let dist = sqrt(dx * dx + dz * dz);
    let want = atan2(dx, dz) * (180.0 / PI);
    let diff = wrap180(want - cat.heading);
    let ball_speed = sqrt(ball.vx * ball.vx + ball.vz * ball.vz);

    let mut target_speed = 0.0f32;
    // how fast (deg/frame) this state lets the body turn towards the ball
    let mut max_turn = 0.0f32;
    let mut target_crouch = 0.0f32;
    let mut target_sit = 0.0f32;
    let mut target_yaw = 0.0f32;
    let mut target_tilt = 0.0f32;
    let mut target_pitch = 0.0f32;
    let target_perk: f32;
    cat.timer += 1;

    match cat.state {
        State::Chase => {
            max_turn = 9.0;
            const REACH: f32 = 0.78;
            if dist > REACH {
                let cap = if dist > 2.0 { 0.045 } else { 0.03 };
                target_speed = clampf((dist - REACH) * 0.06, 0.0, cap);
                if absf(diff) > 70.0 {
                    target_speed *= 0.3;
                }
            }
            // the head leads: it looks at the ball before the body turns
            target_yaw = clampf(diff * 0.8, -45.0, 45.0);
            target_pitch = clampf((1.6 - dist) * 12.0, 0.0, 12.0);
            target_perk = clampf(1.5 - dist, 0.0, 1.0);
            if cat.timer > 300 {
                // can't get at it for a while: fish it back out into the room
                let dir = atan2(0.2 - ball.x, -0.5 - ball.z) * (180.0 / PI);
                kick(ball, dir + rng.range(-25.0, 25.0), 0.06);
                cat.timer = 0;
            }
            // blocked (wall, bed) but close enough to swat from here
            let stuck = cat.speed < 0.004 && dist < 1.0;
            if (dist < REACH + 0.06 || stuck) && absf(diff) < 12.0 && ball_speed < 0.02 {
                cat.state = State::Crouch;
                cat.timer = 0;
            }
        }
        State::Crouch => {
            max_turn = 2.5;
            target_crouch = 1.0;
            target_yaw = clampf(diff, -20.0, 20.0);
            target_pitch = 14.0;
            target_perk = 1.0;
            // the butt wiggle builds up before the pounce
            if cat.timer > 18 {
                cat.wiggle = clampf((cat.timer as f32 - 18.0) / 20.0, 0.0, 1.0) * 0.07;
            }
            cat.wiggle_t += 0.8;
            if dist > 1.1 {
                cat.state = State::Chase;
                cat.timer = 0;
            } else if cat.timer > 52 {
                cat.state = State::Swat;
                cat.timer = 0;
            }
        }
        State::Swat => {
            target_crouch = 0.35;
            target_pitch = 10.0;
            target_perk = 1.0;
            // wind-up, fast strike, hold, slow recovery
            let tm = cat.timer as f32;
            cat.paw = if tm < 6.0 {
                -0.18 * ease_in_out(tm / 6.0)
            } else if tm < 10.0 {
                -0.18 + 1.18 * ease_out((tm - 6.0) / 4.0)
            } else if tm < 14.0 {
                1.0
            } else {
                1.0 - ease_in_out((tm - 14.0) / 12.0)
            };
            cat.lunge = 0.07 * clampf(cat.paw, 0.0, 1.0);
            if cat.timer == 9 && dist < 1.1 {
                // cats bat sideways: pick the side facing the open room
                // (straight ahead is the wall or, after a bounce, the cat)
                let (cx, cz) = (0.2 - ball.x, -0.5 - ball.z);
                let side = |d: f32| {
                    let r = d * (PI / 180.0);
                    sin(r) * cx + cos(r) * cz
                };
                // facing a wall, a sideways bat just slides it along the
                // skirting; allow back-diagonals then (the swing passes
                // beside the body)
                let near_wall = absf(ball.x) > ROOM_X - 0.7
                    || ball.z < ROOM_BACK + 0.7
                    || ball.z > ROOM_FRONT - 0.5;
                let spread: &[f32] = if near_wall { &[75.0, -75.0, 130.0, -130.0] } else { &[75.0, -75.0] };
                let mut aim = cat.heading + spread[0];
                for &o in spread {
                    if side(cat.heading + o) > side(aim) {
                        aim = cat.heading + o;
                    }
                }
                let dir = aim + rng.range(-20.0, 20.0);
                kick(ball, dir, rng.range(0.06, 0.12));
                let h = cat.heading * (PI / 180.0);
                if let Some(hh) = hearts.iter_mut().find(|hh| hh.age == 0) {
                    *hh = Heart { x: cat.x + sin(h) * 0.4, y: 1.55, z: cat.z + cos(h) * 0.4, age: 1 };
                }
            }
            if cat.timer > 26 {
                cat.paw = 0.0;
                cat.state = State::Watch;
                cat.timer = 0;
                cat.watch_len = 70 + (rng.next() % 110);
            }
        }
        State::Watch => {
            max_turn = 2.0;
            target_sit = 1.0;
            target_yaw = clampf(diff, -60.0, 60.0);
            target_tilt = sin(cat.timer as f32 * 0.035) * 14.0;
            target_perk = clampf(ball_speed * 25.0, 0.0, 1.0);
            if cat.timer > cat.watch_len && ball_speed < 0.03 {
                cat.state = State::Chase;
                cat.timer = 0;
            }
        }
    }
    if cat.state != State::Crouch {
        cat.wiggle *= 0.8;
    }
    if cat.state != State::Swat {
        cat.lunge *= 0.85;
    }

    // turning: ease the turn rate in and out instead of snapping a fixed
    // angle per frame; slow down while turning hard
    let want_turn = clampf(diff * 0.22, -max_turn, max_turn);
    cat.turn += (want_turn - cat.turn) * 0.3;
    cat.heading = wrap180(cat.heading + cat.turn);
    let turn_slow = 1.0 - clampf(absf(cat.turn) / 12.0, 0.0, 0.55);
    cat.speed += (target_speed * turn_slow - cat.speed) * 0.12;

    let h = cat.heading * (PI / 180.0);
    let (fx, fz) = (sin(h), cos(h));
    cat.x += fx * cat.speed;
    cat.z += fz * cat.speed;
    cat.x = clampf(cat.x, -ROOM_X + 0.45, ROOM_X - 0.45);
    cat.z = clampf(cat.z, ROOM_BACK + 0.6, ROOM_FRONT - 0.4);
    // keep the head/chest out of the walls too (otherwise the body circles
    // reach through the skirting and pin the yarn against it)
    let (chx, chz) = (cat.x + fx * 0.5, cat.z + fz * 0.5);
    cat.x += clampf(chx, -ROOM_X + 0.3, ROOM_X - 0.3) - chx;
    cat.z += clampf(chz, ROOM_BACK + 0.3, ROOM_FRONT - 0.1) - chz;
    // walk around the bed rather than through it
    let (bx, bz) = (cat.x - BED.0, cat.z - BED.1);
    let bd = sqrt(bx * bx + bz * bz);
    if bd < BED_R + 0.35 && bd > 1e-4 {
        cat.x = BED.0 + bx / bd * (BED_R + 0.35);
        cat.z = BED.1 + bz / bd * (BED_R + 0.35);
    }

    // gait: paws keep stepping while turning on the spot too
    cat.walk_phase += cat.speed * 9.0 + absf(cat.turn) * 0.06;
    let moving = cat.speed > 0.003 || absf(cat.turn) > 0.8;
    cat.walk_amt = lerp(cat.walk_amt, if moving { 1.0 } else { 0.0 }, 0.12);

    // springs: a little overshoot everywhere reads as weight and life
    cat.crouch.go(target_crouch, 0.06, 0.3);
    cat.sit.go(target_sit, 0.04, 0.28);
    cat.head_yaw.go(target_yaw, 0.16, 0.38);
    cat.head_pitch.go(target_pitch, 0.07, 0.3);
    cat.head_tilt.go(target_tilt, 0.05, 0.3);
    cat.ear_perk.go(target_perk, 0.1, 0.3);
    cat.lean.go(clampf(-cat.turn * 1.2, -9.0, 9.0), 0.1, 0.32);
    step_spine(cat);
    // tail follow-through: the base swings opposite to the spine's curl and
    // every segment chases the one before it, so the motion reaches the tip
    let curl = wrap180(cat.spine[0] - cat.spine[SPINE - 1]);
    let base = -curl * 0.7 - cat.turn * 2.0;
    for i in 0..TAIL_SEGS {
        let target = if i == 0 { base } else { cat.tail[i - 1].x * 0.9 };
        cat.tail[i].go(target, 0.14, 0.24);
    }

    // the body pushes the ball out of the way (no walking through yarn)
    if ball.free > 0 {
        ball.free -= 1;
    }
    // (kept inside swat reach, or the cat dribbles the ball into the walls
    // on every approach instead of batting it)
    for off in [-0.35f32, 0.0, 0.3] {
        if ball.free > 0 {
            break;
        }
        let (px, pz) = (cat.x + fx * off, cat.z + fz * off);
        let (ex, ez) = (ball.x - px, ball.z - pz);
        let d = sqrt(ex * ex + ez * ez);
        let rr = 0.3 + BALL_R * 0.5;
        if d < rr && d > 1e-4 {
            ball.x = px + ex / d * rr;
            ball.z = pz + ez / d * rr;
            ball.vx += ex / d * 0.004;
            ball.vz += ez / d * 0.004;
        }
    }

    // blinking + ear twitches
    if cat.blink > 0 {
        cat.blink -= 1;
    } else if cat.blink_in == 0 {
        cat.blink = 7;
        cat.blink_in = 120 + rng.next() % 200;
    } else {
        cat.blink_in -= 1;
    }
    if rng.next() % 240 == 0 {
        cat.ear_twitch = 1.0;
    }
    cat.ear_twitch *= 0.85;
}

// ---------------------------------------------------------------------------
// drawing
// ---------------------------------------------------------------------------

fn draw_room(b: &mut Batch, frame: u32) {
    let center = [0.0, 1.5, -0.5];

    // checkered wooden floor
    let mut z = ROOM_BACK;
    let mut row = 0;
    let tile = 0.8;
    while z < FLOOR_FRONT - 0.01 {
        let mut x = -ROOM_X;
        let mut col = 0;
        while x < ROOM_X - 0.01 {
            let c = if (row + col) % 2 == 0 { [198, 142, 94] } else { [178, 124, 80] };
            b.lit_quad([x, 0.0, z], [x + tile, 0.0, z], [x + tile, 0.0, z + tile], [x, 0.0, z + tile], c, center, true);
            x += tile;
            col += 1;
        }
        z += tile;
        row += 1;
    }

    // striped mint wallpaper with a white baseboard
    let stripes = 10;
    for i in 0..stripes {
        let x0 = -ROOM_X + 2.0 * ROOM_X * i as f32 / stripes as f32;
        let x1 = -ROOM_X + 2.0 * ROOM_X * (i + 1) as f32 / stripes as f32;
        let c = if i % 2 == 0 { [176, 216, 204] } else { [160, 202, 190] };
        let zb = ROOM_BACK;
        b.lit_quad([x0, 0.18, zb], [x1, 0.18, zb], [x1, WALL_H, zb], [x0, WALL_H, zb], c, center, true);
        b.lit_quad([x0, 0.0, zb], [x1, 0.0, zb], [x1, 0.18, zb], [x0, 0.18, zb], [240, 236, 228], center, true);
    }
    for side in [-1.0f32, 1.0] {
        let x = side * ROOM_X;
        let panels = 8;
        for i in 0..panels {
            let z0 = ROOM_BACK + (FLOOR_FRONT - ROOM_BACK) * i as f32 / panels as f32;
            let z1 = ROOM_BACK + (FLOOR_FRONT - ROOM_BACK) * (i + 1) as f32 / panels as f32;
            let c = if i % 2 == 0 { [170, 210, 198] } else { [154, 196, 184] };
            b.lit_quad([x, 0.18, z0], [x, 0.18, z1], [x, WALL_H, z1], [x, WALL_H, z0], c, center, true);
            b.lit_quad([x, 0.0, z0], [x, 0.0, z1], [x, 0.18, z1], [x, 0.18, z0], [236, 232, 224], center, true);
        }
    }

    // window with a night sky, moon and twinkling stars
    let (wx0, wx1, wy0, wy1) = (0.45, 1.75, 1.25, 2.45);
    let zw = ROOM_BACK + 0.01;
    let frame_c = [250, 244, 230];
    b.raw(
        [[wx0 - 0.1, wy0 - 0.1, zw], [wx1 + 0.1, wy0 - 0.1, zw], [wx1 + 0.1, wy1 + 0.1, zw]],
        [frame_c; 3],
    );
    b.raw(
        [[wx0 - 0.1, wy0 - 0.1, zw], [wx1 + 0.1, wy1 + 0.1, zw], [wx0 - 0.1, wy1 + 0.1, zw]],
        [frame_c; 3],
    );
    xy_rect(b, wx0, wy0, wx1, wy1, zw + 0.02, [16, 20, 64], [52, 60, 132]);
    xy_disc(b, 1.42, 2.12, zw + 0.04, 0.15, 12, [255, 244, 190]);
    xy_disc(b, 1.49, 2.18, zw + 0.06, 0.12, 12, [26, 32, 84]); // crescent bite
    let stars = [(0.62, 2.3), (0.92, 2.0), (0.8, 1.55), (1.2, 2.35), (1.6, 1.5), (1.2, 1.42), (0.58, 1.8)];
    for (i, &(sx, sy)) in stars.iter().enumerate() {
        let tw = 0.012 + 0.012 * (0.5 + 0.5 * sin(frame as f32 * 0.07 + i as f32 * 1.7));
        let c = [255, 255, 220];
        b.flat([sx - tw, sy, zw + 0.04], [sx + tw, sy, zw + 0.04], [sx, sy + tw * 2.0, zw + 0.04], c);
        b.flat([sx - tw, sy + tw * 1.2, zw + 0.04], [sx + tw, sy + tw * 1.2, zw + 0.04], [sx, sy - tw * 0.8, zw + 0.04], c);
    }
    let mull = [244, 236, 220];
    xy_rect(b, (wx0 + wx1) * 0.5 - 0.035, wy0, (wx0 + wx1) * 0.5 + 0.035, wy1, zw + 0.08, mull, mull);
    xy_rect(b, wx0, (wy0 + wy1) * 0.5 - 0.035, wx1, (wy0 + wy1) * 0.5 + 0.035, zw + 0.08, mull, mull);

    // round rug
    let rug = t(0.25, 0.0, -0.5);
    if !diag("nodecal") {
        floor_disc(b, &rug, 16, 1.45, 0.98, 0.01, [214, 132, 150]);
        floor_disc(b, &rug, 16, 1.12, 0.74, 0.02, [236, 170, 182]);
        floor_disc(b, &rug, 16, 0.74, 0.48, 0.03, [214, 132, 150]);
    }

    // cat bed: purple ring with a fluffy cushion
    let bed = t(BED.0, 0.0, BED.1);
    if !diag("nodecal") {
        floor_disc(b, &bed, 14, 0.66, 0.66, 0.01, [120, 88, 70]);
    }
    torus(b, &chain(&[bed, t(0.0, 0.16, 0.0)]), 12, 5, 0.46, 0.16, &|i| {
        if i % 2 == 0 { [150, 110, 200] } else { [134, 96, 186] }
    });
    sphere1(b, &chain(&[bed, t(0.0, 0.07, 0.0), s(0.44, 0.07, 0.44)]), 10, 4, [250, 228, 246]);
}

fn draw_ball(b: &mut Batch, ball: &Ball, trail: &Trail) {
    // strand of yarn across the floor
    let y = 0.045;
    let decals = !diag("nodecal");
    for i in (1..trail.len).filter(|_| decals) {
        let (ax, az) = trail.get(i - 1);
        let (bx, bz) = trail.get(i);
        ribbon(b, [ax, y, az], [bx, y, bz], 0.018, YARN_A);
    }
    if let Some((lx, lz)) = trail.last().filter(|_| decals) {
        ribbon(b, [lx, y, lz], [ball.x, y, ball.z], 0.018, YARN_A);
    }

    // shadow
    if decals {
        floor_disc(b, &t(ball.x, 0.0, ball.z), 10, BALL_R * 1.05, BALL_R * 1.05, 0.04, [96, 70, 60]);
    }

    let m = chain(&[t(ball.x, BALL_R, ball.z), ball.rot, s(BALL_R, BALL_R, BALL_R)]);
    sphere(b, &m, 10, 7, false, &|i, j| {
        if i == 3 {
            YARN_BAND
        } else if (j + i / 2) % 2 == 0 {
            YARN_A
        } else {
            YARN_B
        }
    });
    // a cross-wound loop so the rolling reads clearly
    let loop_m = chain(&[t(ball.x, BALL_R, ball.z), ball.rot, rz(60.0), s(BALL_R * 1.02, BALL_R * 0.18, BALL_R * 1.02)]);
    sphere1(b, &loop_m, 10, 3, YARN_BAND);
}

fn draw_cat(b: &mut Batch, cat: &Cat, frame: u32) {
    let (cr, si) = (cat.crouch.x, cat.sit.x);
    let breathe = sin(frame as f32 * 0.06) * 0.008;
    let walk = cat.walk_amt;
    let ph = cat.walk_phase;
    // two footfalls per stride: the body bobs twice and sways once
    let bob = (absf(sin(ph)) - 0.5) * 0.035 * walk;
    let sway = sin(ph) * 3.0 * walk;
    let nodes = spine_nodes(cat);
    let last = SPINE - 1;

    // soft shadow under the middle of the back
    if !diag("nodecal") {
        let mid = chain(&[t(cat.x, 0.0, cat.z), ry(cat.spine[1])]);
        floor_disc(b, &mid, 12, 0.42, 0.66, 0.04, [96, 70, 60]);
    }

    // Per-joint pose. u runs 0 (shoulders) .. 1 (hips): crouching drops the
    // shoulders and lifts the hips, sitting does the opposite. The butt
    // wiggle is a wave that starts at the hips and travels forward, fading
    // out before the shoulders so the head stays locked on the yarn.
    let mut height = [0.0f32; SPINE];
    let mut shake = [0.0f32; SPINE];
    let mut twist = [0.0f32; SPINE];
    for k in 0..SPINE {
        let u = k as f32 / last as f32;
        height[k] = 0.5 + breathe + bob + cr * lerp(-0.16, -0.01, u) + si * lerp(0.06, -0.14, u);
        let phase = cat.wiggle_t - (last - k) as f32 * 0.75;
        let weight = u * u;
        shake[k] = cat.wiggle * sin(phase) * weight;
        twist[k] = cat.wiggle * cos(phase) * weight * 260.0;
    }
    // the frame of joint k: on the spine, turned to its yaw, leaning, and
    // pitched to follow the back's slope between its neighbours
    let joint = |k: usize| -> Mtx {
        let (fwd, back) = (k.saturating_sub(1), (k + 1).min(last));
        let run = SPINE_SEG * (back - fwd) as f32;
        let pitch = atan2(height[fwd] - height[back], run) * (180.0 / PI);
        chain(&[
            t(nodes[k].0, height[k], nodes[k].1),
            ry(cat.spine[k] + twist[k]),
            t(shake[k], 0.0, 0.0),
            rz(cat.lean.x + sway),
            rx(-pitch),
        ])
    };

    // body: one overlapping section per vertebra joint, so the back curves
    // smoothly through turns instead of hinging
    let radii: [(f32, f32); SPINE] = [(0.31, 0.29), (0.33, 0.3), (0.335, 0.305), (0.34, 0.31)];
    for k in 0..SPINE {
        let (rw, rh) = radii[k];
        // long enough to overlap the neighbours well: one smooth back
        let section = chain(&[joint(k), s(rw, rh + breathe, 0.24), rx(90.0)]);
        sphere(b, &section, 10, 6, false, &|i, j| {
            let back = j >= 5;
            let belly = (1..=3).contains(&j);
            if back && (i == 2 || i == 3) && k % 2 == 1 {
                ORANGE_DARK
            } else if belly && i > 0 && i < 5 {
                CREAM
            } else {
                ORANGE
            }
        });
    }

    // legs hang from the shoulder and hip joints (upright, only turned and
    // shaken with them), in a trot: diagonal pairs move together and a paw
    // lifts while its leg swings forward. During the wiggle the back paws
    // tread in place.
    let leg_base = |k: usize| -> Mtx {
        chain(&[t(nodes[k].0, 0.0, nodes[k].1), ry(cat.spine[k] + twist[k] * 0.5), t(shake[k], 0.0, 0.0)])
    };
    let (front_base, back_base) = (leg_base(0), leg_base(last));
    let tread = cat.wiggle * 1.6;
    let legs: [(&Mtx, f32, f32, f32, f32, f32, f32); 4] = [
        // (frame, x, z, pivot height, gait phase offset, extra swing, tread)
        // the swat: +swing rotates the paw forward and up (the wind-up's
        // negative paw pulls it back first)
        (&front_base, 0.16, 0.01, height[0] - 0.06, 0.0, cat.paw * 115.0, 0.0),
        (&front_base, -0.16, 0.01, height[0] - 0.06, PI, 0.0, 0.0),
        (&back_base, 0.16, -0.04, height[last] - 0.06, PI, si * 10.0, tread * max0(sin(cat.wiggle_t))),
        (&back_base, -0.16, -0.04, height[last] - 0.06, 0.0, si * 10.0, tread * max0(-sin(cat.wiggle_t))),
    ];
    for (n, &(base, lx, lz, py, off, extra, tread)) in legs.iter().enumerate() {
        let p = ph + off;
        let swing = sin(p) * 28.0 * walk + extra;
        let lift = max0(cos(p)) * 0.08 * walk + tread;
        let len = py - lift;
        // the swatting paw also sweeps inwards, across in front of the face
        let sweep = if n == 0 { clampf(cat.paw, 0.0, 1.0) * 22.0 } else { 0.0 };
        let leg = chain(&[*base, t(lx, py, lz), rz(sweep), rx(-swing), t(0.0, -len, 0.0)]);
        prism(b, &leg, 6, 0.085, 0.1, len, ORANGE);
        // round white sock paws (beans!), toes tipping up as they lift
        let paw = chain(&[leg, t(0.0, 0.05, 0.03), rx(-lift * 300.0), s(0.105, 0.07, 0.125)]);
        sphere1(b, &paw, 8, 4, CREAM);
    }

    // tail: tapering segments curling up from the hip joint, so the wiggle
    // and the spine's curl carry straight into it. Each segment adds its
    // follow-through angle (cat.tail) plus an idle wave; when crouched only
    // the tip twitches, fast.
    let crouched = cat.state == State::Crouch;
    let mut tm = chain(&[
        joint(last),
        t(0.0, 0.06 - si * 0.22, -0.18 + si * 0.07),
        rx(-58.0 - si * 30.0 - walk * 8.0 + cr * 18.0),
    ]);
    for i in 0..TAIL_SEGS {
        let r0 = 0.07 - 0.005 * i as f32;
        let r1 = r0 - 0.005;
        let len = 0.13;
        let wave = if crouched {
            if i >= TAIL_SEGS - 3 { sin(frame as f32 * 0.5 + i as f32) * 16.0 } else { 0.0 }
        } else {
            sin(frame as f32 * 0.07 + ph * 0.5 - i as f32 * 0.6) * (6.0 + 4.0 * walk)
        };
        tm = chain(&[tm, rz(cat.tail[i].x + wave), rx(13.0 - si * 8.0 - walk * 2.0)]);
        let c = if i == TAIL_SEGS - 1 { CREAM } else if i % 2 == 1 { ORANGE_DARK } else { ORANGE };
        prism(b, &tm, 6, r0, r1, len, c);
        tm = chain(&[tm, t(0.0, len, 0.0)]);
    }
    sphere1(b, &chain(&[tm, s(0.04, 0.04, 0.04)]), 6, 3, CREAM);

    // head: rides ahead of the shoulder joint, nods a little with each step
    // and looks where it wants (head_yaw is relative to the shoulders)
    let nod = sin(ph * 2.0) * 2.5 * walk;
    let head = chain(&[
        t(nodes[0].0, 0.0, nodes[0].1),
        ry(cat.spine[0]),
        t(shake[0], 0.98 - cr * 0.26 + si * 0.12 + breathe + bob * 0.6, 0.21 - cr * 0.02 - si * 0.04 + cat.lunge * 0.4),
        ry(cat.head_yaw.x - twist[0]),
        rz(cat.head_tilt.x + cat.lean.x * 0.5),
        rx(-si * 6.0 + cr * 6.0 + cat.head_pitch.x + nod),
    ]);
    sphere(b, &chain(&[head, s(0.44, 0.39, 0.40)]), 12, 8, false, &|i, j| {
        let front = j < 6;
        if i == 1 && j % 2 == 0 && !front {
            ORANGE_DARK
        } else if i == 1 && (j == 2 || j == 4) {
            ORANGE_DARK
        } else if i >= 6 && front {
            CREAM
        } else {
            ORANGE
        }
    });

    // ears: square pyramids with a pink inner triangle
    for side in [-1.0f32, 1.0] {
        let twitch = if side > 0.0 { cat.ear_twitch * 20.0 } else { 0.0 };
        let perk = cat.ear_perk.x;
        let ear = chain(&[head, t(side * 0.21, 0.25, -0.03), rz(-side * (24.0 - perk * 8.0 + twitch)), rx(-6.0 - perk * 14.0), ry(45.0)]);
        let (r, h) = (0.14, 0.24);
        prism(b, &ear, 4, r, 0.0, h, ORANGE);
        // the face that points forward after ry(45) is the one between the
        // corners at 45° and 135° → local +Z after un-rotating; build it in
        // the pre-ry space instead so it lines up exactly.
        let face = chain(&[head, t(side * 0.21, 0.25, -0.03), rz(-side * (24.0 - perk * 8.0 + twitch)), rx(-6.0 - perk * 14.0)]);
        let a = r * 0.7071;
        let refp = xf(&face, [0.0, h * 0.3, 0.0]);
        b.lit(
            xf(&face, [-a * 0.62, h * 0.1, a * 0.9 + 0.012]),
            xf(&face, [a * 0.62, h * 0.1, a * 0.9 + 0.012]),
            xf(&face, [0.0, h * 0.72, a * 0.28 + 0.012]),
            PINK,
            refp,
            false,
        );
    }

    // eyes: glossy dark ovals with a highlight; squash to a line to blink
    let open = if cat.blink > 0 { 0.12 } else { 1.0 };
    for side in [-1.0f32, 1.0] {
        // big, low-set eyes read as kitten-cute
        let eye = chain(&[head, t(side * 0.16, -0.02, 0.35), ry(side * 16.0), s(0.095, 0.12 * open, 0.06)]);
        sphere1(b, &eye, 10, 6, EYE);
        if cat.blink == 0 {
            let hl = chain(&[head, t(side * 0.16 + 0.03, 0.025, 0.405), s(0.032, 0.032, 0.012)]);
            sphere(b, &hl, 6, 3, true, &|_, _| WHITE);
            let hl2 = chain(&[head, t(side * 0.16 - 0.028, -0.06, 0.4), s(0.015, 0.015, 0.008)]);
            sphere(b, &hl2, 5, 3, true, &|_, _| WHITE);
        }
        // rosy cheeks
        let cheek = chain(&[head, t(side * 0.26, -0.1, 0.3), ry(side * 40.0), s(0.065, 0.035, 0.02)]);
        sphere(b, &cheek, 8, 3, true, &|_, _| BLUSH);
    }

    // muzzle, nose, whiskers
    for side in [-1.0f32, 1.0] {
        sphere1(b, &chain(&[head, t(side * 0.06, -0.135, 0.345), s(0.075, 0.06, 0.065)]), 8, 4, CREAM);
    }
    sphere1(b, &chain(&[head, t(0.0, -0.08, 0.388), s(0.038, 0.026, 0.026)]), 6, 3, PINK);
    for side in [-1.0f32, 1.0] {
        for k in 0..3 {
            let y0 = -0.14 + k as f32 * 0.025;
            let p0 = [side * 0.13, y0, 0.37];
            let p1 = [side * 0.44, y0 + (k as f32 - 1.0) * 0.05 + 0.01, 0.3];
            let w = 0.004;
            let c = [255, 250, 240];
            b.flat(xf(&head, [p0[0], p0[1] - w, p0[2]]), xf(&head, [p1[0], p1[1] - w, p1[2]]), xf(&head, [p1[0], p1[1] + w, p1[2]]), c);
            b.flat(xf(&head, [p0[0], p0[1] - w, p0[2]]), xf(&head, [p1[0], p1[1] + w, p1[2]]), xf(&head, [p0[0], p0[1] + w, p0[2]]), c);
        }
    }
}

fn draw_hearts(b: &mut Batch, hearts: &[Heart; 4]) {
    for h in hearts.iter().filter(|h| h.age > 0) {
        let k = h.age as f32 / 70.0;
        let sz = 0.09 * (1.0 - k) * clampf(h.age as f32 / 8.0, 0.0, 1.0);
        if sz <= 0.0 {
            continue;
        }
        let (x, y, z) = (h.x, h.y + k * 0.6, h.z);
        let c = [255, 92, 132];
        xy_disc(b, x - 0.5 * sz, y + 0.3 * sz, z, 0.56 * sz, 8, c);
        xy_disc(b, x + 0.5 * sz, y + 0.3 * sz, z, 0.56 * sz, 8, c);
        b.flat([x - 1.03 * sz, y + 0.15 * sz, z], [x + 1.03 * sz, y + 0.15 * sz, z], [x, y - 1.1 * sz, z], c);
    }
}

/// Rounded-rect contour point `i` of `n` (n = 4 * per_corner).
fn rrect(i: usize, per_corner: usize, cy: f32, hw: f32, hh: f32, r: f32) -> (f32, f32) {
    let corner = i / per_corner;
    let k = i % per_corner;
    let (cx_, cy_, a0) = match corner {
        0 => (hw - r, hh - r, 0.0),
        1 => (-(hw - r), hh - r, PI * 0.5),
        2 => (-(hw - r), -(hh - r), PI),
        _ => (hw - r, -(hh - r), PI * 1.5),
    };
    let a = a0 + (PI * 0.5) * k as f32 / (per_corner - 1) as f32;
    (cx_ + r * cos(a), cy + cy_ + r * sin(a))
}

/// The retro TV bezel, drawn in view space at z = -1.2.
fn draw_bezel(b: &mut Batch, half_h: f32, frame: u32) {
    let z = -1.2;
    let (cy, hw, hh, r) = (0.06, 0.70, 0.49, 0.12);
    let pc = 6;
    let n = 4 * pc;
    let bevel = 0.045;
    let shell = [232, 214, 184];
    let lip = [190, 170, 142];
    let dark = [70, 62, 58];
    for i in 0..n {
        let j = (i + 1) % n;
        let a = rrect(i, pc, cy, hw, hh, r);
        let bb = rrect(j, pc, cy, hw, hh, r);
        let c = rrect(j, pc, cy, hw + bevel, hh + bevel, r + bevel);
        let d = rrect(i, pc, cy, hw + bevel, hh + bevel, r + bevel);
        // inner bevel: dark at the glass, warming towards the shell
        b.raw([[a.0, a.1, z], [bb.0, bb.1, z], [c.0, c.1, z]], [dark, dark, lip]);
        b.raw([[a.0, a.1, z], [c.0, c.1, z], [d.0, d.1, z]], [dark, lip, lip]);
        // the shell shades from the lip into the plastic just outside it
        let o = 0.03;
        let e = rrect(j, pc, cy, hw + bevel + o, hh + bevel + o, r + bevel + o);
        let f = rrect(i, pc, cy, hw + bevel + o, hh + bevel + o, r + bevel + o);
        b.raw([[d.0, d.1, z], [c.0, c.1, z], [e.0, e.1, z]], [lip, lip, shell]);
        b.raw([[d.0, d.1, z], [e.0, e.1, z], [f.0, f.1, z]], [lip, shell, shell]);
    }

    // Solid shell out to just past the visible edge. Keep every vertex near
    // the screen: real GX rasterizes in fixed point with a limited range,
    // and vertices thousands of pixels off-screen wrap into garbage slivers
    // (Dolphin's float rasterizer doesn't care).
    let (ow, oh) = (half_h * (640.0 / 480.0) * 1.08, half_h * 1.08);
    let (bx, by) = (hw + bevel + 0.03, hh + bevel + 0.03);
    let rr = r + bevel + 0.03;
    xy_rect(b, -ow, cy + by, ow, oh, z, shell, shell); // top
    xy_rect(b, -ow, -oh, ow, cy - by, z, shell, shell); // bottom
    xy_rect(b, -ow, cy - by, -bx, cy + by, z, shell, shell); // left
    xy_rect(b, bx, cy - by, ow, cy + by, z, shell, shell); // right
    // corner fans: the bounding-box corner to the rounded arc
    for corner in 0..4 {
        let (sx, sy) = [(1.0f32, 1.0f32), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)][corner];
        let k = [sx * bx, cy + sy * by, z];
        for m in 0..pc - 1 {
            let p0 = rrect(corner * pc + m, pc, cy, bx, by, rr);
            let p1 = rrect(corner * pc + m + 1, pc, cy, bx, by, rr);
            b.flat(k, [p0.0, p0.1, z], [p1.0, p1.1, z], shell);
        }
    }

    // chin: speaker grille, power LED, two knobs
    let chin_top = cy - hh - bevel;
    let ky = (chin_top - half_h) * 0.5;
    let zz = z + 0.001;
    for k in 0..4 {
        let y = ky - 0.03 + k as f32 * 0.02;
        xy_rect(b, -0.66, y, -0.36, y + 0.008, zz, [150, 132, 110], [150, 132, 110]);
    }
    let led = if (frame / 90) % 8 == 0 { [255, 150, 60] } else { [110, 255, 140] };
    xy_disc(b, 0.22, ky, zz, 0.012, 8, led);
    for kx in [0.46f32, 0.6] {
        xy_disc(b, kx, ky, zz, 0.042, 12, [148, 92, 62]);
        xy_disc(b, kx, ky, zz + 0.0005, 0.03, 12, [178, 118, 82]);
        xy_rect(b, kx - 0.005, ky, kx + 0.005, ky + 0.034, zz + 0.001, [250, 240, 220], [250, 240, 220]);
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[no_mangle]
extern "C" fn main() -> i32 {
    let gc = gc_std::init();
    let gx = gc.into_gx();
    gx.set_clear_color(gc_std::GXColor::rgb(18, 14, 24));

    const FOV: f32 = 55.0;
    let proj = gu::perspective(FOV, 640.0 / 480.0, 1.0, 100.0);
    gx.load_perspective(&proj);
    let half_h = 1.2 * gu::sinf(FOV * 0.5 * gu::DEG_TO_RAD) / gu::cosf(FOV * 0.5 * gu::DEG_TO_RAD);

    gx::clear_vtx_desc();
    gx::set_vtx_desc_direct(gx::GX_VA_POS);
    gx::set_vtx_desc_direct(gx::GX_VA_CLR0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_POS, gx::GX_POS_XYZ, gx::GX_F32, 0);
    gx::set_vtx_attr_fmt(gx::VTXFMT0, gx::GX_VA_CLR0, gx::GX_CLR_RGBA, gx::GX_RGBA8, 0);
    gx::config_vertex_color_pipeline();

    let mut batch = Batch {
        tris: unsafe { &mut *addr_of_mut!(TRIS) },
        n: 0,
        key: normalize([-0.45, 0.8, 0.55]),
    };
    let mut rng = Rng((gc_std::timebase::now() as u32) | 1);

    let mut cat = Cat::new(-0.8, -0.6, 60.0);
    let mut ball = Ball { x: 0.9, z: 0.2, vx: -0.02, vz: 0.0, rot: gu::identity(), free: 0 };
    if diag("photo") {
        cat = Cat::new(0.0, 0.15, 0.0);
        ball.x = 0.15;
        ball.z = 0.95;
        ball.vx = 0.0;
    }
    let mut trail = Trail { pts: [(0.0, 0.0); TRAIL], head: 0, len: 0 };
    let mut hearts = [Heart { x: 0.0, y: 0.0, z: 0.0, age: 0 }; 4];
    let mut frame: u32 = 0;

    let up = gu::vec3(0.0, 1.0, 0.0);
    let bezel_view = gu::identity();

    loop {
        gc_std::input::scan();
        let down = gc_std::input::buttons_down(0);
        if down.contains(button::START) {
            gc_std::system::exit(0);
        }
        if down.contains(button::A) {
            kick(&mut ball, rng.range(-180.0, 180.0), rng.range(0.08, 0.14));
        }
        let a = gc_std::input::analog(0);
        if a.present {
            let (sx, sy) = (f32::from(a.stick_x), f32::from(a.stick_y));
            if absf(sx) > 24.0 || absf(sy) > 24.0 {
                ball.vx += sx / 127.0 * 0.004;
                ball.vz -= sy / 127.0 * 0.004;
            }
        }

        step_ball(&mut ball, &mut trail);
        step_cat(&mut cat, &mut ball, &mut rng, &mut hearts);
        for h in hearts.iter_mut().filter(|h| h.age > 0) {
            h.age += 1;
            if h.age > 70 {
                h.age = 0;
            }
        }

        gx.begin_frame();
        gx::set_cull_mode(gx::GX_CULL_NONE);
        gx::set_z_mode(true, gx::GX_LEQUAL, true);

        // a gentle camera sway so the room feels alive
        let ft = frame as f32;
        let cam = gu::vec3(sin(ft * 0.004) * 0.35, 2.55 + sin(ft * 0.003) * 0.08, 5.3);
        let target = gu::vec3(0.0, 0.45, -0.55);
        let view = gu::look_at(cam, up, target);
        gx.load_model_view(&view);

        draw_room(&mut batch, frame);
        draw_ball(&mut batch, &ball, &trail);
        draw_cat(&mut batch, &cat, frame);
        draw_hearts(&mut batch, &hearts);
        batch.submit();

        if !diag("nobezel") {
            gx.load_model_view(&bezel_view);
            gx::set_z_mode(false, gx::GX_ALWAYS, false);
            draw_bezel(&mut batch, half_h, frame);
            batch.submit();
        }

        gx.draw_done();
        gx.end_frame();
        frame = frame.wrapping_add(1);
    }
}
