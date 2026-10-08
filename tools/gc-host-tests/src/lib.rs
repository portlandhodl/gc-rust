//! Host-side tests against pure-Rust modules from `gc-std`.
//!
//! We include `gu.rs` and `gctypes.rs` verbatim via `#[path]` so the exact
//! shipping code is unit-tested on the host.

#[path = "../../../crates/gc-std/src/gctypes.rs"]
pub mod gctypes;

// gu.rs needs `crate::gctypes` — resolve by aliasing to a concrete module
// name `crate::gctypes` via re-export in a root module named `gctypes`.
#[path = "../../../crates/gc-std/src/gu.rs"]
#[allow(dead_code)]
pub mod gu;

#[cfg(test)]
mod tests {
    use super::gctypes::{Mtx, Mtx44};
    use super::gu;

    const EPS: f32 = 1e-4;

    fn assert_close(a: f32, b: f32, ctx: &str) {
        assert!(
            (a - b).abs() <= EPS,
            "{ctx}: expected {a} ~ {b} (|diff| {} > {EPS})",
            (a - b).abs()
        );
    }

    #[test]
    fn perspective_matches_reference() {
        // gluPerspective(fov=60°, aspect=4:3, near=1, far=100) reference:
        let cot = 1.0f32 / (30.0 * gu::DEG_TO_RAD).tan();
        let m: Mtx44 = gu::perspective(60.0, 4.0 / 3.0, 1.0, 100.0);
        assert_close(m[0][0], cot / (4.0 / 3.0), "m00");
        assert_close(m[1][1], cot, "m11");
        assert_close(m[2][2], -1.0 / 99.0, "m22");
        assert_close(m[2][3], -(100.0 / 99.0), "m23");
        assert_close(m[3][2], -1.0, "m32");
    }

    #[test]
    fn identity_matrix() {
        let m = gu::identity();
        for r in 0..3 {
            for c in 0..4 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert_eq!(m[r][c], want);
            }
        }
    }

    #[test]
    fn concat_identity_neutral() {
        let m: Mtx = [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
        ];
        let out = gu::concat(&gu::identity(), &m);
        for r in 0..3 {
            for c in 0..4 {
                assert_eq!(out[r][c], m[r][c], "(r{r},c{c})");
            }
        }
        // and identity on the right too
        let out2 = gu::concat(&m, &gu::identity());
        for r in 0..3 {
            for c in 0..4 {
                assert_eq!(out2[r][c], m[r][c], "right (r{r},c{c})");
            }
        }
    }

    #[test]
    fn translate_adds_vector() {
        let m = gu::identity();
        let out = gu::translate(&m, 3.0, -2.0, 0.5);
        assert_eq!(out[0][3], 3.0);
        assert_eq!(out[1][3], -2.0);
        assert_eq!(out[2][3], 0.5);
        // rotation part untouched
        assert_eq!(out[0][0], 1.0);
        assert_eq!(out[1][1], 1.0);
        assert_eq!(out[2][2], 1.0);
    }

    #[test]
    fn rotate_z_90_swaps_xy() {
        let mut m = gu::identity();
        gu::rotate_axis_deg(&mut m, gu::vec3(0.0, 0.0, 1.0), 90.0);
        // +90° about Z: X+ -> Y+ ; Y+ -> -X+
        assert_close(m[0][0], 0.0, "m00");
        assert_close(m[0][1], -1.0, "m01");
        assert_close(m[1][0], 1.0, "m10");
        assert_close(m[1][1], 0.0, "m11");
    }

    #[test]
    fn rotate_360_came_home() {
        let mut m = gu::identity();
        gu::rotate_axis_deg(&mut m, gu::vec3(0.31, 0.71, -0.21), 360.0);
        assert_close(m[0][0], 1.0, "m00");
        assert_close(m[1][1], 1.0, "m11");
        assert_close(m[2][2], 1.0, "m22");
        assert_close(m[0][1], 0.0, "m01");
        assert_close(m[1][0], 0.0, "m10");
    }

    #[test]
    fn look_at_down_neg_z() {
        // cam at origin, look down -Z: identity view
        let m = gu::look_at(
            gu::vec3(0.0, 0.0, 0.0),
            gu::vec3(0.0, 1.0, 0.0),
            gu::vec3(0.0, 0.0, -1.0),
        );
        assert_close(m[0][0], 1.0, "m00");
        assert_close(m[1][1], 1.0, "m11");
        assert_close(m[2][2], 1.0, "m22");
        for (r, c) in [(0usize, 1usize), (1, 0), (2, 0), (2, 1), (0, 3), (1, 3), (2, 3)] {
            assert_close(m[r][c], 0.0, &format!("m{r}{c}"));
        }
    }

    #[test]
    fn scalar_libm_correctness() {
        // gu's private libm goes through pub wrappers; test via trig results:
        // cos(180°) == -1 (through gu rotation): rotate X by 180 deg
        let mut m = gu::identity();
        gu::rotate_axis_deg(&mut m, gu::vec3(1.0, 0.0, 0.0), 180.0);
        assert_close(m[1][1], -1.0, "cos180 m11");
        assert_close(m[2][2], -1.0, "cos180 m22");
        assert_close(m[0][0], 1.0, "axis row m00");
    }
}
