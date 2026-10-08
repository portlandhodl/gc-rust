//! Free-list allocator over MEM1's "arena".
//!
//! ```text
//!  __bss_end .. heap .. MEM1_TOP(minus stack)
//! ```
//!
//! Small, deterministic, *best-fit* + address-ordered coalescing free list
//! with 32-byte block alignment (cache line). Best-fit (tightest fitting
//! block) is chosen over first-fit to limit splintering of large free runs
//! in long-running sessions. The global allocator is registered in
//! [`crate::crt0`].

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

const ALIGN: usize = 32;
/// Header size before each payload (size+magic).
const HDR: usize = 32;
const MAGIC: u32 = 0x4743_414C;
/// Smallest free-list fragment. One cache line: all block addresses and
/// sizes are multiples of 32, so every split sliver can go straight back
/// into the free list — no bytes are ever absorbed/lost between blocks.
const MIN_BLOCK: usize = 32;

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct Heap {
    head: *mut FreeBlock,
    start: *mut u8,
    end: *mut u8,
}

static mut HEAP: Heap = Heap {
    head: ptr::null_mut(),
    start: ptr::null_mut(),
    end: ptr::null_mut(),
};

#[inline]
const fn align_up(v: usize, a: usize) -> usize {
    (v + a - 1) & !(a - 1)
}

pub(crate) unsafe fn init(start: *mut u8, end: *mut u8) {
    let h = &mut *core::ptr::addr_of_mut!(HEAP);
    let s = align_up(start as usize, ALIGN) as *mut u8;
    let e = (end as usize & !(ALIGN - 1)) as *mut u8;
    h.start = s;
    h.end = e;
    h.head = ptr::null_mut();
    if s < e {
        let b = s as *mut FreeBlock;
        (*b).size = e as usize - s as usize;
        (*b).next = ptr::null_mut();
        h.head = b;
    }
}

/// Allocate with explicit alignment (no header semantics).
///
/// Best-fit policy: scan the whole list and split the *tightest* fitting
/// block; the list stays address-ordered so `free_block` can coalesce.
pub(crate) unsafe fn alloc_raw(size: usize, align: usize) -> *mut u8 {
    let h = &mut *core::ptr::addr_of_mut!(HEAP);
    let align = align.max(ALIGN);
    let want = align_up(size, ALIGN);

    let mut best: *mut FreeBlock = ptr::null_mut();
    let mut best_prev: *mut FreeBlock = ptr::null_mut();
    let mut best_pad = 0usize;
    let mut best_leftover = usize::MAX;

    let mut prev: *mut FreeBlock = ptr::null_mut();
    let mut cur = h.head;
    while !cur.is_null() {
        let raw = cur as usize;
        let aligned = align_up(raw, align);
        let pad = aligned - raw;
        let avail = (*cur).size;
        if avail >= pad + want {
            let leftover = avail - pad - want;
            if leftover < best_leftover {
                best = cur;
                best_prev = prev;
                best_pad = pad;
                best_leftover = leftover;
                if leftover == 0 {
                    break; // exact fit; can't do better
                }
            }
        }
        prev = cur;
        cur = (*cur).next;
    }
    if best.is_null() {
        return ptr::null_mut();
    }

    // Replace `best` with up to two free fragments: the alignment pad
    // [best, payload) and the tail [payload_end, block_end). Both stay
    // address-ordered in the list so free-time coalescing still works and
    // the heap is byte-exact (freed arenas fully re-coalesce).
    let raw = best as usize;
    let old_next = (*best).next;
    let tail_addr = raw + best_pad + want;

    let mut link = old_next;
    if best_leftover >= MIN_BLOCK {
        let tail = tail_addr as *mut FreeBlock;
        (*tail).size = best_leftover;
        (*tail).next = old_next;
        link = tail;
    }
    // best_pad is a multiple of ALIGN: < MIN_BLOCK means it is 0.
    if best_pad >= MIN_BLOCK {
        (*best).size = best_pad;
        (*best).next = link;
        link = best;
    }
    if best_prev.is_null() {
        h.head = link;
    } else {
        (*best_prev).next = link;
    }
    (raw + best_pad) as *mut u8
}

pub(crate) unsafe fn free_block(addr: *mut u8, size: usize) {
    let h = &mut *core::ptr::addr_of_mut!(HEAP);
    let b = addr as *mut FreeBlock;
    (*b).size = align_up(size, ALIGN);
    (*b).next = ptr::null_mut();

    if h.head.is_null() || (b as usize) < (h.head as usize) {
        (*b).next = h.head;
        h.head = b;
    } else {
        let mut cur = h.head;
        while !(*cur).next.is_null() && ((*cur).next as usize) < (b as usize) {
            cur = (*cur).next;
        }
        (*b).next = (*cur).next;
        (*cur).next = b;
    }
    // coalesce
    let mut cur = h.head;
    while !cur.is_null() {
        let next = (*cur).next;
        if !next.is_null() && (cur as usize) + (*cur).size == (next as usize) {
            (*cur).size += (*next).size;
            (*cur).next = (*next).next;
        } else {
            cur = (*cur).next;
        }
    }
}

pub struct GcAllocator;

// Only the target build registers the global allocator; host-side unit
// tests (tools/gc-host-tests) include this file too and must not clash
// with the host's allocator.
#[cfg_attr(target_arch = "powerpc", global_allocator)]
static ALLOCATOR: GcAllocator = GcAllocator;

unsafe impl GlobalAlloc for GcAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let total = align_up(layout.size() + HDR, ALIGN);
        let align = layout.align().max(ALIGN);
        let block = alloc_raw(total, align);
        if block.is_null() {
            return ptr::null_mut();
        }
        let hdr = block as *mut u32;
        hdr.write_volatile(total as u32);
        hdr.add(1).write_volatile(MAGIC);
        block.add(HDR)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let block = unsafe { ptr.sub(HDR) };
        let hdr = block as *mut u32;
        let total = hdr.read_volatile() as usize;
        let magic = unsafe { hdr.add(1).read_volatile() };
        debug_assert_eq!(magic, MAGIC, "heap header corrupted");
        unsafe { free_block(block, total) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        if ptr.is_null() {
            return self.alloc(Layout::from_size_align_unchecked(new_size, old_layout.align()));
        }
        let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());
        let new_ptr = self.alloc(new_layout);
        if !new_ptr.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(ptr, new_ptr, old_layout.size().min(new_size));
            }
            self.dealloc(ptr, old_layout);
        }
        new_ptr
    }
}
