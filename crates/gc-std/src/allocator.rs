//! Global allocator backed by newlib's heap (libogc provides `sbrk` on top
//! of the system arena, so `malloc` works with no setup).

use core::alloc::{GlobalAlloc, Layout};

use crate::ffi;

/// newlib guarantees 8-byte alignment from `malloc`.
const MALLOC_ALIGN: usize = 8;

struct GcAllocator;

unsafe impl GlobalAlloc for GcAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = if layout.align() <= MALLOC_ALIGN {
            ffi::malloc(layout.size())
        } else {
            ffi::memalign(layout.align(), layout.size())
        };
        ptr.cast::<u8>()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // newlib's free() also releases memalign() allocations.
        ffi::free(ptr.cast());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() <= MALLOC_ALIGN {
            ffi::realloc(ptr.cast(), new_size).cast::<u8>()
        } else {
            // newlib cannot grow over-aligned allocations in place.
            let new_layout =
                Layout::from_size_align_unchecked(new_size, layout.align());
            let new_ptr = self.alloc(new_layout);
            if !new_ptr.is_null() {
                core::ptr::copy_nonoverlapping(ptr, new_ptr, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
            new_ptr
        }
    }
}

#[global_allocator]
static ALLOCATOR: GcAllocator = GcAllocator;
