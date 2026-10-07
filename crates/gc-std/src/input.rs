//! GameCube controller (PAD) input.

use crate::ffi;

/// A single GameCube controller button (bitmask from `ogc/pad.h`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Button(u16);

/// Namespaced button constants, mirroring `ogc/pad.h`.
pub mod button {
    use super::Button;

    pub const LEFT: Button = Button(0x0001);
    pub const RIGHT: Button = Button(0x0002);
    pub const DOWN: Button = Button(0x0004);
    pub const UP: Button = Button(0x0008);
    pub const Z: Button = Button(0x0010);
    pub const R: Button = Button(0x0020);
    pub const L: Button = Button(0x0040);
    pub const A: Button = Button(0x0100);
    pub const B: Button = Button(0x0200);
    pub const X: Button = Button(0x0400);
    pub const Y: Button = Button(0x0800);
    pub const START: Button = Button(0x1000);
    /// Alias of [`START`](self::START).
    pub const MENU: Button = START;
}

/// Set of buttons pushed this frame. Returned by [`buttons_down`].
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Buttons(u16);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);

    /// `true` if every button in `button` is down.
    #[inline]
    pub fn contains(self, button: Button) -> bool {
        self.0 & button.0 == button.0
    }

    /// `true` if at least one button is down.
    #[inline]
    pub fn any(self) -> bool {
        self.0 != 0
    }

    /// Raw `ogc/pad.h` bitmask.
    #[inline]
    pub fn bits(self) -> u16 {
        self.0
    }
}

pub(crate) fn init() {
    unsafe {
        ffi::PAD_Init();
    }
}

/// Poll all four controller ports. Call once per frame before
/// [`buttons_down`].
#[inline]
pub fn scan() {
    unsafe {
        ffi::PAD_ScanPads();
    }
}

/// Buttons that transitioned to "down" since the last [`scan`], for
/// controller `pad` (0..=3).
#[inline]
pub fn buttons_down(pad: u8) -> Buttons {
    Buttons(unsafe { ffi::PAD_ButtonsDown(i32::from(pad)) })
}

/// Buttons currently held down on controller `pad` (0..=3).
#[inline]
pub fn buttons_held(pad: u8) -> Buttons {
    Buttons(unsafe { ffi::PAD_ButtonsHeld(i32::from(pad)) })
}

/// Snapshot of the analog inputs of controller `pad` (0..=3).
#[derive(Copy, Clone, Debug, Default)]
pub struct Analog {
    /// Main stick X axis (-100..=100 roughly).
    pub stick_x: i8,
    /// Main stick Y axis.
    pub stick_y: i8,
    /// C-stick X axis.
    pub substick_x: i8,
    /// C-stick Y axis.
    pub substick_y: i8,
    /// Analog L trigger (0..=255).
    pub trigger_l: u8,
    /// Analog R trigger (0..=255).
    pub trigger_r: u8,
}

/// Read all analog inputs of controller `pad` (0..=3).
#[inline]
pub fn analog(pad: u8) -> Analog {
    let p = i32::from(pad);
    unsafe {
        Analog {
            stick_x: ffi::PAD_StickX(p),
            stick_y: ffi::PAD_StickY(p),
            substick_x: ffi::PAD_SubStickX(p),
            substick_y: ffi::PAD_SubStickY(p),
            trigger_l: ffi::PAD_TriggerL(p),
            trigger_r: ffi::PAD_TriggerR(p),
        }
    }
}
