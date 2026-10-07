//! Free-list allocator over MEM1's "arena".
//!
//! ```text
//!  __bss_end .. heap .. MEM1_TOP(minus stack)
//! ```
//!
//! Small, deterministic, first-fit + coalescing free list with 32-byte
//! block alignment (cache line). The global allocator is registered in
//! [`crate::crt0`].

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

const ALIGN: usize = 32;
/// Header size before each payload (size+magic).
const HDR: usize = 32;
const MAGIC: u32 = 0x4743_414C;
const MIN_BLOCK: usize = 64;

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
pub(crate) unsafe fn alloc_raw(size: usize, align: usize) -> *mut u8 {
    let h = &mut *core::ptr::addr_of_mut!(HEAP);
    let align = align.max(ALIGN);
    let want = align_up(size, ALIGN);

    let mut prev: *mut FreeBlock = ptr::null_mut();
    let mut cur = h.head;
    while !cur.is_null() {
        let raw = cur as usize;
        let aligned = align_up(raw, align);
        let pad = aligned - raw;
        let avail = (*cur).size;
        if avail >= pad + want {
            let leftover = avail - pad - want;
            let next = (*cur).next;
            if leftover >= MIN_BLOCK {
                let nb = (raw + pad + want) as *mut FreeBlock;
                (*nb).size = leftover;
                (*nb).next = next;
                if prev.is_null() {
                    h.head = nb;
                } else {
                    (*prev).next = nb;
                }
            } else if prev.is_null() {
                h.head = next;
            } else {
                (*prev).next = next;
            }
            return aligned as *mut u8;
        }
        prev = cur;
        cur = (*cur).next;
    }
    ptr::null_mut()
}

unsafe fn free_block(addr: *mut u8, size: usize) {
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

#[global_allocator]
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
