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

// heap.rs is pure pointer arithmetic on a caller-provided arena — testable
// on the host as-is. The `#[global_allocator]` registration is gated on
// target_arch = "powerpc" inside heap.rs, so including it here is safe.
#[path = "../../../crates/gc-std/src/heap.rs"]
#[allow(dead_code)]
pub mod heap;

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

#[cfg(test)]
mod heap_tests {
    use crate::heap;
    use core::alloc::{GlobalAlloc, Layout};
    use std::vec::Vec;

    const ARENA: usize = 1024 * 1024;

    #[repr(align(32))]
    struct Arena([u8; ARENA]);

    // The allocator under test holds its free list in one global static —
    // serialize the heap tests (the default test runner is multithreaded).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TestHeap {
        _arena: Box<Arena>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHeap {
        fn new() -> TestHeap {
            let guard = LOCK.lock().unwrap();
            let mut t = TestHeap {
                _arena: Box::new(Arena([0; ARENA])),
                _guard: guard,
            };
            let start = t.base();
            unsafe { heap::init(start, start.add(ARENA)) };
            t
        }
        fn base(&mut self) -> *mut u8 {
            self._arena.0.as_mut_ptr() as *mut u8
        }
    }

    #[test]
    fn alloc_free_reuse() {
        let mut t = TestHeap::new();
        unsafe {
            let a = heap::alloc_raw(100, 32);
            let b = heap::alloc_raw(200, 64);
            assert!(!a.is_null() && !b.is_null());
            assert_eq!(a as usize % 32, 0);
            assert_eq!(b as usize % 64, 0);
            assert!(b as usize - a as usize >= 96); // no overlap (a rounded to 96)
            heap::free_block(a, 100);
            heap::free_block(b, 200);
            // after coalescing the whole arena is one block again
            let whole = heap::alloc_raw(ARENA, 32);
            assert_eq!(whole, t.base());
        }
    }

    #[test]
    fn best_fit_consumes_tightest_hole() {
        let mut t = TestHeap::new();
        let k = 1024usize;
        unsafe {
            let a = heap::alloc_raw(64 * k, 32);
            let hole_big = heap::alloc_raw(96 * k, 32);
            let b = heap::alloc_raw(64 * k, 32);
            let hole_exact = heap::alloc_raw(64 * k, 32);
            let pad = heap::alloc_raw(ARENA - 288 * k, 32);
            assert!(!a.is_null() && !hole_big.is_null() && !b.is_null()
                && !hole_exact.is_null() && !pad.is_null());
            heap::free_block(hole_big, 96 * k);
            heap::free_block(hole_exact, 64 * k);
            // 64 KiB request: best-fit must land in the exact 64 KiB hole,
            // first-fit would splinter the earlier 96 KiB hole.
            let x = heap::alloc_raw(64 * k, 32);
            assert_eq!(x, hole_exact, "64K must reuse the exact-size hole");
            let _ = t;
        }
    }

    #[test]
    fn stress_then_full_coalesce() {
        let mut t = TestHeap::new();
        let mut live: Vec<(*mut u8, usize)> = Vec::new();
        let mut rng: u32 = 0x1234_5678;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng
        };
        unsafe {
            for _ in 0..4096 {
                if (next() % 5) < 3 {
                    // alloc
                    let size = ((next() as usize) % (8 * 1024)) / 32 * 32 + 32;
                    let p = heap::alloc_raw(size, 32);
                    if !p.is_null() {
                        // stamp + verify unallocated memory doesn't alias
                        for i in 0..size {
                            *p.add(i) = (p as usize & 0xff) as u8;
                        }
                        live.push((p, size));
                    }
                } else if let Some(idx) = live.len().checked_sub(1).map(|n| (next() as usize) % (n + 1)) {
                    let (p, size) = live.swap_remove(idx);
                    for i in 0..size {
                        assert_eq!(*p.add(i), (p as usize & 0xff) as u8, "block corrupted (overlap)");
                    }
                    heap::free_block(p, size);
                }
            }
            for (p, s) in live {
                heap::free_block(p, s);
            }
            let whole = heap::alloc_raw(ARENA, 32);
            assert_eq!(whole, t.base(), "arena must coalesce back to one block");
        }
    }

    #[test]
    fn global_alloc_header_roundtrip() {
        let mut t = TestHeap::new();
        let a = heap::GcAllocator;
        unsafe {
            let l = Layout::from_size_align(1000, 16).unwrap();
            let p1 = a.alloc(l);
            assert!(!p1.is_null());
            *p1 = 0xAA;
            let p2 = a.alloc(l);
            a.dealloc(p1, l);
            // reuse after free
            let p3 = a.alloc(l);
            assert!(!p3.is_null());
            a.dealloc(p2, l);
            a.dealloc(p3, l);
            let whole = heap::alloc_raw(ARENA, 32);
            assert_eq!(whole, t.base());
        }
    }
}
