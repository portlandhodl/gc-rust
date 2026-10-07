//! Exit / loader control.

/// Return to the loader (Swiss/Homebrew-channel" style) by jumping to the
/// 0x80001800 "reload stub" address — the standard virtual-exit path.
pub fn exit(_code: i32) -> ! {
    exit_to_loader()
}

/// Jump to the loader's reload stub. Equivalent to libogc's `exit(0)`.
pub fn exit_to_loader() -> ! {
    unsafe {
        let reload: extern "C" fn() = core::mem::transmute(0x8000_1800usize);
        reload();
    }
    // unreachable on real loaders; stay here if we get back
    loop {
        unsafe { crate::hw::isync() };
    }
}
