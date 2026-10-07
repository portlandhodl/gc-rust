//! Console lifecycle helpers.

use crate::ffi;

/// Exit the application (returns to the loader / homebrew channel).
pub fn exit(code: i32) -> ! {
    unsafe { ffi::exit(code) }
}
