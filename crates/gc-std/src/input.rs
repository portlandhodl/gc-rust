//! GameCube controller (PAD) driver.
//!
//! Pure-Rust reimplementation of libogc's pad.si sampling using the SI
//! hardware's built-in **command polling** (SICXOUTBUF + SISR RDST), the
//! same mechanism libogc uses. Wire format verified against both libogc
//! and Dolphin's `SI_DeviceGCController`:
//!
//! ```text
//! cmd 0x40 0x03 0x00  ->  hi = stickY | stickX<<8 | (buttons|0x80)<<16
//!                         lo = trigR | trigL<<8 | subY<<16 | subX<<24
//! ```
//!
//! Button bits match `ogc/pad.h` (A=0x100, B=0x200, X=0x400, Y=0x800,
//! START=0x1000, dpad=0x1-0x8, Z=0x10, R=0x20, L=0x40).

use crate::hw::{si_read, si_write, gcdelay, MEM_BASE_UNCACHED};

/// Button bits (`ogc/pad.h` values).
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
    pub const MENU: Button = START;
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Button(pub u16);

/// Set of buttons.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Buttons(u16);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    #[inline]
    pub fn contains(self, b: Button) -> bool {
        self.0 & b.0 == b.0
    }
    #[inline]
    pub fn any(self) -> bool {
        self.0 != 0
    }
    #[inline]
    pub fn bits(self) -> u16 {
        self.0
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Analog {
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_l: u8,
    pub trigger_r: u8,
}

// SISR packing: channel status lives in byte (3-chan) — chan0 = bits 24-31.
// Within the byte: 0x20=RDST (response ready), 0x08=NORESPONSE (ERRSTAT).
const fn sisr_chan_byte(chan: u32) -> u32 {
    (3 - chan) * 8
}
const SISR_RDST: u32 = 0x20;
const SISR_NORESPONSE: u32 = 0x08;



static mut BUTTONS: [u16; 4] = [0; 4];
static mut PREV: [u16; 4] = [0; 4];
static mut DOWN_EDGE: [u16; 4] = [0; 4];
static mut ANALOGS: [Analog; 4] = [Analog{
    stick_x: 0, stick_y: 0, substick_x: 0, substick_y: 0, trigger_l: 0, trigger_r: 0
}; 4];

pub(crate) fn init() {
    unsafe {
        // libogc __si_init + __pad_enable equivalents:
        // 1. command for all channels: "poll pad" 0x40 0x03 0x00
        for chan in 0..4u32 {
            si_write(chan * 3, 0x0040_0300);
        }
        // 2. clear status flags
        si_write(14, 0x0);
        // 3. SI_SetXY(0xF6, 2) + enable polling for all 4 channels.
        //    SICPOL = line<<6 | cnt<<16 | (0x80>>chan enable bits 4-7)
        si_write(12, (0x00F6 << 6) | (0x02 << 16) | 0xF0);

        // let the SI hardware settle (one poll cycle)
        gcdelay(40_000);
    }
}

/// Poll all channels synchronously. Called once per frame.
pub fn scan() {
    unsafe {
        let buttons = &mut *core::ptr::addr_of_mut!(BUTTONS);
        let prev = &mut *core::ptr::addr_of_mut!(PREV);
        let edge = &mut *core::ptr::addr_of_mut!(DOWN_EDGE);
        let analogs = &mut *core::ptr::addr_of_mut!(ANALOGS);

        for chan in 0..4u32 {
            prev[chan as usize] = buttons[chan as usize];

            let sisr = si_read(14) >> sisr_chan_byte(chan);
            if sisr & SISR_NORESPONSE != 0 {
                // no pad
                buttons[chan as usize] = 0;
                edge[chan as usize] = 0;
                analogs[chan as usize] = Analog::default();
                continue;
            }

            if sisr & SISR_RDST == 0 {
                // data not ready; keep last state
                edge[chan as usize] = 0;
                continue;
            }

            let hi = si_read(chan * 3 + 1);
            let lo = si_read(chan * 3 + 2);
            si_write(14, 0); // ack rdst

            let mut btn = ((hi >> 16) & 0x1fff) as u16; // strip USE_ORIGIN (0x80)
            let a = Analog {
                stick_x: (((hi >> 8) & 0xff) as u8).wrapping_sub(128) as i8,
                stick_y: ((hi & 0xff) as u8).wrapping_sub(128) as i8,
                substick_x: ((lo >> 24) as u8).wrapping_sub(128) as i8,
                substick_y: (((lo >> 16) & 0xff) as u8).wrapping_sub(128) as i8,
                trigger_l: ((lo >> 8) & 0xff) as u8,
                trigger_r: (lo & 0xff) as u8,
            };
            if a.trigger_l >= 0xaa { btn |= 0x0040; }
            if a.trigger_r >= 0xaa { btn |= 0x0020; }

            buttons[chan as usize] = btn;
            analogs[chan as usize] = a;
            edge[chan as usize] = btn & !prev[chan as usize];
        }
    }
    let _ = MEM_BASE_UNCACHED;
}

/// Buttons that were pushed this frame on `pad` (0..=3).
#[inline]
pub fn buttons_down(pad: u8) -> Buttons {
    unsafe { Buttons(DOWN_EDGE[(pad & 3) as usize]) }
}

/// Buttons currently held on `pad` (0..=3).
#[inline]
pub fn buttons_held(pad: u8) -> Buttons {
    unsafe { Buttons(BUTTONS[(pad & 3) as usize]) }
}

/// Analog state snapshot for `pad` (0..=3).
#[inline]
pub fn analog(pad: u8) -> Analog {
    unsafe { ANALOGS[(pad & 3) as usize] }
}
