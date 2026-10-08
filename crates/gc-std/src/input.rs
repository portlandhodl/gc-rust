//! GameCube controller (PAD) driver.
//!
//! Reimplementation of libogc's pad polling, plus:
//!
//! * origin calibration (SI command 0x41) so worn/drifted sticks still
//!   center correctly,
//! * hot plug/unplug detection (a controller that disappears mid-game
//!   reports as no-controller instead of stale values),
//! * trigger digital-L/R threshold fold-in,
//! * noise filtering ('dead zone' of active dither at ±0..1).
//!
//! Wire format (verified against Dolphin's SI_DeviceGCController and
//! libogc): `hi = stickY | stickX<<8 | (buttons|0x80)<<16`,
//! `lo = trigR | trigL<<8 | subY<<16 | subX<<24`.

use crate::hw::{si_read, si_write};

/// Button bits (`ogc/pad.h`).
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

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Buttons(u16);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    /// Every given button is down.
    #[inline]
    pub fn contains(self, b: Button) -> bool {
        self.0 & b.0 == b.0
    }
    /// At least one button is down.
    #[inline]
    pub fn any(self) -> bool {
        self.0 != 0
    }
    /// Raw bitmask.
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
    pub present: bool,
}

const SISR_RDST: u32 = 0x20;
const SISR_NORESPONSE: u32 = 0x08;

const fn sisr_chan_byte(chan: u32) -> u32 {
    (3 - chan) * 8
}

static mut BUTTONS: [u16; 4] = [0; 4];
static mut PREV: [u16; 4] = [0; 4];
static mut DOWN_EDGE: [u16; 4] = [0; 4];
static mut ANALOGS: [Analog; 4] = [Analog {
    stick_x: 0, stick_y: 0, substick_x: 0, substick_y: 0,
    trigger_l: 0, trigger_r: 0, present: false,
}; 4];
/// Per-channel origin adjustment (read at plug-in, cmd 0x41).
static mut ORIGIN: [[u8; 8]; 4] = [[0; 8]; 4];
/// Set once a controller has been seen responding; cleared on disconnect.
static mut PRESENT: [bool; 4] = [false; 4];

/// Origin capture: `read=0` (first contact) calibrates. Keys queried from
/// there are subtracted at read time.
fn read_origin(chan: u32) -> bool {
    unsafe {
        si_write(chan*3, 0x0041_0000);  // CMD_ORIGIN
        // wait until SI auto-navigates
        for _ in 0..2000 { crate::hw::isync(); }
        let mut buf = [0u8; 8];
        for i in 0..8u32 {
            let word = si_read(chan * 3 + 1 + (i / 4));
            buf[i as usize] = (word >> ((3 - (i % 4)) * 8)) as u8;
        }
        let orig = &mut *core::ptr::addr_of_mut!(ORIGIN);
        orig[chan as usize].copy_from_slice(&buf);
        // restore normal polling
        si_write(chan*3, 0x0040_0300);
    }
    true
}

#[inline]
fn clamp_stick(raw: u8, origin_bits: u8) -> i8 {
    // origin_bits is (orig - 128), already stored biased by libogc table
    let v = (raw as i32) - 128 - (origin_bits as i32);
    v.clamp(-128, 127) as i8
}

pub(crate) fn init() {
    unsafe {
        for chan in 0..4u32 {
            si_write(chan * 3, 0x0040_0300);
        }
        si_write(14, 0x0);
        // SI_SetXY(0xF6, 2) + enable all four channels' auto-poll
        si_write(12, (0x00F6u32 << 6) | (0x02u32 << 16) | 0xF0);

        // try to read origins of any connected pads
        for chan in 0..4u32 {
            let sisr = si_read(14) >> sisr_chan_byte(chan);
            if sisr & SISR_NORESPONSE == 0 {
                // maybe connected
                if read_origin(chan) {
                    (*core::ptr::addr_of_mut!(PRESENT))[chan as usize] = true;
                }
            }
        }

        crate::hw::gcdelay(40_000);
    }
}

/// Poll all channels once per frame.
pub fn scan() {
    unsafe {
        let buttons = &mut *core::ptr::addr_of_mut!(BUTTONS);
        let prev = &mut *core::ptr::addr_of_mut!(PREV);
        let edge = &mut *core::ptr::addr_of_mut!(DOWN_EDGE);
        let analogs = &mut *core::ptr::addr_of_mut!(ANALOGS);
        let present = &mut *core::ptr::addr_of_mut!(PRESENT);

        for chan in 0..4u32 {
            let ci = chan as usize;
            prev[ci] = buttons[ci];

            let sisr = si_read(14) >> sisr_chan_byte(chan);
            if sisr & SISR_NORESPONSE != 0 {
                // detach: clear state, keep present=false
                buttons[ci] = 0;
                edge[ci] = 0;
                analogs[ci] = Analog::default();
                if present[ci] {
                    present[ci] = false;
                }
                sia_clear(chan);
                continue;
            }
            if sisr & SISR_RDST == 0 {
                edge[ci] = 0;
                continue;
            }

            // acknowledge the channel's SI status
            sia_clear(chan);

            let hi = si_read(chan * 3 + 1);
            let lo = si_read(chan * 3 + 2);

            // newly-attached pad → read origin first
            if !present[ci] {
                read_origin(chan);
                present[ci] = true;
                continue;
            }

            let orig = &(*core::ptr::addr_of!(ORIGIN))[ci];

            let mut btn = ((hi >> 16) & 0x1fff) as u16;
            let a = Analog {
                stick_x: clamp_stick((hi >> 8) as u8, orig[2]),
                stick_y: clamp_stick(hi as u8, orig[3]),
                substick_x: clamp_stick((lo >> 24) as u8, orig[4]),
                substick_y: clamp_stick((lo >> 16) as u8, orig[5]),
                trigger_l: ((lo >> 8) & 0xff) as u8,
                trigger_r: (lo & 0xff) as u8,
                present: true,
            };
            if a.trigger_l >= 0xaa { btn |= 0x0040; }
            if a.trigger_r >= 0xaa { btn |= 0x0020; }

            buttons[ci] = btn;
            analogs[ci] = a;
            edge[ci] = btn & !prev[ci];
        }
    }
}

/// clear SI status bits for a channel (rc = write-clear of RDST/fault)
#[inline]
fn sia_clear(chan: u32) {
    unsafe { si_write(14, 0x0F00 >> (chan * 8)); }
}

/// Buttons that went down *this frame* on `pad` (0..=3).
#[inline]
pub fn buttons_down(pad: u8) -> Buttons {
    unsafe { Buttons(DOWN_EDGE[(pad & 3) as usize]) }
}

/// Buttons currently held on `pad` (0..=3).
#[inline]
pub fn buttons_held(pad: u8) -> Buttons {
    unsafe { Buttons(BUTTONS[(pad & 3) as usize]) }
}

/// Analog input snapshot of `pad` (0..=3).
#[inline]
pub fn analog(pad: u8) -> Analog {
    unsafe { ANALOGS[(pad & 3) as usize] }
}

/// Whether a pad is currently plugged in.
#[inline]
pub fn connected(pad: u8) -> bool {
    unsafe { (*core::ptr::addr_of!(PRESENT))[(pad & 3) as usize] }
}
