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

// --- stubs the gc-std modules need when compiled into this crate ---------
pub mod irq {
    pub struct IrqLock;
    impl IrqLock {
        pub fn take() -> IrqLock {
            IrqLock
        }
    }
    impl Drop for IrqLock {
        fn drop(&mut self) {}
    }
}
pub mod hw {
    pub unsafe fn dc_flush_range<T>(_p: *const T, _len: usize) {}
    pub unsafe fn dc_invalidate_range<T>(_p: *const T, _len: usize) {}
    pub fn mftb() -> u64 {
        static mut T: u64 = 0;
        unsafe {
            T += 41_000;
            *(&raw const T)
        }
    }
}

#[path = "../../../crates/gc-std/src/card.rs"]
#[allow(dead_code)]
pub mod card;

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
mod card_tests {
    //! Emulated 2 MB memory card (Dolphin/libogc-compatible image format)
    //! driving the real card.rs state machine end to end.
    #![allow(non_snake_case)]

    use crate::card;
    use crate::card::CardBus;

    const BLOCKS: u32 = 2048; // 16 Mbit card
    const SS: u32 = 8192;

    pub struct MockCard {
        pub image: Vec<u8>,
    }

    fn checksum(img: &[u8]) -> (u16, u16) {
        let mut cs1: u16 = 0;
        let mut cs2: u16 = 0;
        for i in 0..img.len() / 2 {
            let w = u16::from_be_bytes([img[i * 2], img[i * 2 + 1]]);
            cs1 = cs1.wrapping_add(w);
            cs2 = cs2.wrapping_add(w ^ 0xffff);
        }
        if cs1 == 0xffff {
            cs1 = 0;
        }
        if cs2 == 0xffff {
            cs2 = 0;
        }
        (cs1, cs2)
    }

    impl MockCard {
        /// Fresh card formatted exactly like `GCMemcard::Format` /
        /// libogc `__card_formatregion`.
        fn new_formatted() -> MockCard {
            let mut img = vec![0xFFu8; (BLOCKS * SS) as usize];
            // dir copies (sectors 1, 2): all 0xFF, dircntrl.updated = 0
            for d in 1..=2usize {
                let base = d * 8192;
                img[base + 8186] = 0;
                img[base + 8187] = 0;
                let (c1, c2) = checksum(&img[base..base + 0x1ffc]);
                img[base + 8188..base + 8190].copy_from_slice(&c1.to_be_bytes());
                img[base + 8190..base + 8192].copy_from_slice(&c2.to_be_bytes());
            }
            // fat copies (sectors 3, 4): zeroed; freeblocks; lastalloc=4
            for f in 3..=4usize {
                let base = f * 8192;
                for b in &mut img[base..base + 8192] {
                    *b = 0;
                }
                let free = (BLOCKS - 5) as u16;
                img[base + 6..base + 8].copy_from_slice(&free.to_be_bytes());
                img[base + 8..base + 10].copy_from_slice(&4u16.to_be_bytes());
                let (c1, c2) = checksum(&img[base + 4..base + 0x1ffc]);
                img[base..base + 2].copy_from_slice(&c1.to_be_bytes());
                img[base + 2..base + 4].copy_from_slice(&c2.to_be_bytes());
            }
            MockCard { image: img }
        }
    }

    impl CardBus for MockCard {
        fn probe(&mut self, _chn: u32) -> bool {
            true
        }
        fn get_id(&mut self, _chn: u32) -> Option<u32> {
            Some(0x10) // memcard251: 16 Mbit, 8 KiB sectors, latency 4
        }
        fn clear_status(&mut self, _chn: u32) -> Result<(), i32> {
            Ok(())
        }
        fn read_status(&mut self, _chn: u32) -> Result<u8, i32> {
            Ok(0x40) // unlocked, ready
        }
        fn enable_interrupt(&mut self, _chn: u32, _on: bool) -> Result<(), i32> {
            Ok(())
        }
        fn read(&mut self, _chn: u32, addr: u32, _latency: u32, buf: &mut [u8]) -> i32 {
            let a = addr as usize;
            if a + buf.len() > self.image.len() {
                return card::CARD_ERROR_FATAL_ERROR;
            }
            buf.copy_from_slice(&self.image[a..a + buf.len()]);
            card::CARD_ERROR_READY
        }
        fn write_sector(&mut self, _chn: u32, addr: u32, buf: &[u8]) -> i32 {
            let a = addr as usize;
            if a + buf.len() > self.image.len() {
                return card::CARD_ERROR_FATAL_ERROR;
            }
            self.image[a..a + buf.len()].copy_from_slice(buf);
            card::CARD_ERROR_READY
        }
        fn erase_sector(&mut self, _chn: u32, addr: u32) -> i32 {
            let a = addr as usize;
            for b in &mut self.image[a..a + SS as usize] {
                *b = 0xFF;
            }
            card::CARD_ERROR_READY
        }
    }

    /// Test fixture: 'static mock wired into the driver; tests are
    /// serialized (the driver keeps global state).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Fixture {
        card: core::ptr::NonNull<MockCard>,
        _g: std::sync::MutexGuard<'static, ()>,
    }

    impl Fixture {
        fn img(&mut self) -> &mut Vec<u8> {
            unsafe { &mut self.card.as_mut().image }
        }
    }

    fn fixture() -> Fixture {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let m: &'static mut MockCard = Box::leak(Box::new(MockCard::new_formatted()));
        let ptr = unsafe { core::ptr::NonNull::new_unchecked(m as *mut MockCard) };
        unsafe { card::set_bus(m) };
        card::test_reset();
        card::init(None, None);
        Fixture { card: ptr, _g: g }
    }

    #[test]
    fn mount_fresh_card() {
        struct FixtureGuard; // placeholder to keep style uniform
        {
            let mut _m = fixture();
            assert_eq!(card::mount(0), card::CARD_ERROR_READY);
            assert_eq!(card::free_blocks(0), Ok(BLOCKS as u16 - 5));
            assert!(card::find_first(0, true).is_err());
        }
    }

    #[test]
    fn create_write_read_verify() {
        let mut _m = fixture();
        assert_eq!(card::mount(0), card::CARD_ERROR_READY);
        let mut f = card::create(0, "test.sav", SS).expect("create");
        let data: Vec<u8> = (0..SS).map(|i| (i * 7 + 3) as u8).collect();
        assert_eq!(card::write(&mut f, &data, 0), card::CARD_ERROR_READY);

        // re-open and read back
        let mut f2 = card::open(0, "test.sav").expect("open");
        let mut back = vec![0u8; SS as usize];
        assert_eq!(card::read(&mut f2, &mut back, 0), card::CARD_ERROR_READY);
        assert_eq!(back, data, "read-back must match what was written");

        let mut dir = card::find_first(0, true).expect("find_first");
        let n = dir.filename.iter().position(|&c| c == 0).unwrap_or(32);
        assert_eq!(&dir.filename[..n], b"test.sav");
        assert_eq!(dir.filelen, SS);
        assert_eq!(card::find_next(&mut dir), card::CARD_ERROR_NOFILE);
        assert_eq!(card::free_blocks(0), Ok(BLOCKS as u16 - 6));
    }

    #[test]
    fn duplicate_name_rejected_and_delete_recovers() {
        let mut _m = fixture();
        assert_eq!(card::mount(0), card::CARD_ERROR_READY);
        let mut f = card::create(0, "dup.sav", SS).expect("create");
        assert_eq!(card::create(0, "dup.sav", SS).err(), Some(card::CARD_ERROR_EXIST));
        let payload = vec![0xABu8; SS as usize];
        card::write(&mut f, &payload, 0);
        assert_eq!(card::delete(0, "dup.sav"), card::CARD_ERROR_READY);
        assert!(card::open(0, "dup.sav").is_err());
        assert_eq!(card::free_blocks(0), Ok(BLOCKS as u16 - 5));
    }

    #[test]
    fn multi_sector_chain_roundtrip() {
        let mut _m = fixture();
        assert_eq!(card::mount(0), card::CARD_ERROR_READY);
        let sectors = 3u32;
        let mut f = card::create(0, "big.sav", SS * sectors).expect("create");
        let data: Vec<u8> = (0..SS * sectors).map(|i| (i / 7 % 251) as u8).collect();
        for s in 0..sectors {
            let off = (s * SS) as i32;
            let w = &data[(s * SS) as usize..((s + 1) * SS) as usize];
            assert_eq!(card::write(&mut f, w, off), card::CARD_ERROR_READY);
        }
        let mut back = vec![0u8; (SS * sectors) as usize];
        for s in 0..sectors {
            let off = (s * SS) as i32;
            assert_eq!(
                card::read(&mut f, &mut back[off as usize..(off as usize + SS as usize)], off),
                card::CARD_ERROR_READY
            );
        }
        assert_eq!(back, data);
    }

    #[test]
    fn fills_card_then_insspace() {
        let mut _m = fixture();
        assert_eq!(card::mount(0), card::CARD_ERROR_READY);
        let free = card::free_blocks(0).unwrap() as u32;
        card::create(0, "all.sav", free * SS).expect("huge create must fit");
        assert_eq!(card::free_blocks(0), Ok(0));
        assert_eq!(card::create(0, "nope.sav", SS).err(), Some(card::CARD_ERROR_INSSPACE));
        // deleting the huge file returns every block
        assert_eq!(card::delete(0, "all.sav"), card::CARD_ERROR_READY);
        assert_eq!(card::free_blocks(0).unwrap(), free as u16);
    }

    #[test]
    fn mount_repairs_one_corrupt_dir_copy() {
        let mut _m = fixture();
        assert_eq!(card::mount(0), card::CARD_ERROR_READY);
        let mut f = card::create(0, "keep.sav", SS).unwrap();
        card::write(&mut f, &[7u8; SS as usize], 0);
        // flip one byte inside the active dir copy in the image
        // (create committed dir copy 0; corrupt sector 1 entirely)
        let mi = _m.img();
        for b in &mut mi[8192..8192 + 64] {
            *b ^= 0x5A;
        }
        card::unmount(0);
        assert_eq!(card::mount(0), card::CARD_ERROR_READY, "mount must repair from the good copy");
        let mut f2 = card::open(0, "keep.sav").expect("file must survive repair");
        let mut back = vec![0u8; SS as usize];
        assert_eq!(card::read(&mut f2, &mut back, 0), card::CARD_ERROR_READY);
        assert!(back.iter().all(|&b| b == 7));
    }

    #[test]
    fn both_dir_copies_corrupt_is_broken() {
        let mut _m = fixture();
        let mi = _m.img();
        for d in 1..=2usize {
            mi[d * 8192 + 8186] = 1; // invalidate each dir copy's checksums
        }
        assert_eq!(card::mount(0), card::CARD_ERROR_BROKEN);
    }

    #[test]
    fn data_actually_lands_in_card_image() {
        let mut _m = fixture();
        assert_eq!(card::mount(0), card::CARD_ERROR_READY);
        let mut f = card::create(0, "vis.sav", SS).expect("create");
        let data: Vec<u8> = (0..SS as u32).map(|i| (0x80u8).wrapping_add((i & 0x7f) as u8)).collect();
        assert_eq!(card::write(&mut f, &data, 0), card::CARD_ERROR_READY);
        // walk the FAT chain on the raw image and expect our bytes there.
        let fat_base = 3 * 8192; // active fat at commit time is sector 3 or 4
        let blk = {
            // read directory to find first block of vis.sav
            let img = _m.img();
            let mut found = None;
            for d in [1usize, 4usize] {
                for i in 0..127 {
                    let o = d * 8192 + i * 64;
                    if &img[o + 8..o + 16] == b"vis.sav\0" {
                        found = Some(u16::from_be_bytes([img[o + 54], img[o + 55]]));
                        break;
                    }
                }
            }
            found.expect("dir entry on card")
        } as usize;
        assert!(blk >= 5 && blk < BLOCKS as usize);
        let img = _m.img();
        assert_eq!(&img[blk * 8192..blk * 8192 + 16], &data[..16]);
        let _ = fat_base;
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
