//! USB Gecko debug adapter driver — pure-Rust port of libogc's
//! `libdb/geckousb.c` (`__usb_sendbyte`/`__usb_recvbyte`/`usb_sendbuffer`…).
//!
//! The USB Gecko is a memory-card-slot (EXI channel) device that tunnels a
//! serial byte stream to a host PC over USB; homebrew uses it as a debug
//! console. Dolphin emulates it ("Gecko" EXI device) as a TCP server on
//! port 55020 — so this doubles as the toolchain's "printf to the host"
//! channel under emulation.
//!
//! Wire protocol: 16-bit command words at 32 MHz. `0xB0xx` = send byte xx,
//! `0xA000` = receive, `0xC000`/`0xD000` = TX/RX fifos ready, `0x9000` =
//! detect (device ID echo must be zero).

use crate::exi;

const GECKO_CMD_SEND: u16 = 0xB000;
const GECKO_CMD_RECV: u16 = 0xA000;
const GECKO_CMD_CHKSEND: u16 = 0xC000;
const GECKO_CMD_CHKRECV: u16 = 0xD000;
const GECKO_CMD_IDENT: u16 = 0x9000;

/// Round-trip one 16-bit command at 32 MHz (channel must already be
/// locked for device 0 — like libogc's `__send_command`); returns the
/// device's reply.
#[inline(always)]
fn send_command_locked(chn: u32, cmd: u16) -> Option<u16> {
    if !exi::select(chn, exi::EXI_DEVICE_0, exi::EXI_SPEED32MHZ) {
        return None;
    }
    let mut buf = cmd.to_be_bytes();
    let ok = exi::imm(chn, &mut buf, exi::EXI_READWRITE);
    exi::deselect(chn);
    if !ok {
        return None;
    }
    Some(u16::from_be_bytes(buf))
}

/// Lock the channel for one command round-trip.
fn send_command(chn: u32, cmd: u16) -> Option<u16> {
    if !exi::lock(chn, exi::EXI_DEVICE_0) {
        return None;
    }
    let r = send_command_locked(chn, cmd);
    exi::unlock(chn);
    r
}

/// `__usb_sendbyte`.
pub fn send_byte(chn: u32, byte: u8) -> bool {
    let cmd = GECKO_CMD_SEND | ((byte as u16) << 4);
    matches!(send_command(chn, cmd), Some(reply) if reply & 0x0400 != 0)
}

/// `__usb_recvbyte`.
pub fn recv_byte(chn: u32) -> Option<u8> {
    match send_command(chn, GECKO_CMD_RECV) {
        Some(reply) if reply & 0x0800 != 0 => Some((reply & 0xff) as u8),
        _ => None,
    }
}

/// `__usb_checksend` — TX fifo has room.
pub fn check_send(chn: u32) -> bool {
    matches!(send_command(chn, GECKO_CMD_CHKSEND), Some(r) if r & 0x0400 != 0)
}

/// `__usb_checkrecv` — RX fifo has data.
pub fn check_recv(chn: u32) -> bool {
    matches!(send_command(chn, GECKO_CMD_CHKRECV), Some(r) if r & 0x0400 != 0)
}

/// `usb_isgeckoalive` — true when a gecko answers on `chn` (its EXI device
/// ID is 0 and the IDENT command comes back with the ack bits set).
pub fn is_gecko_alive(chn: u32) -> bool {
    match exi::get_id(chn, exi::EXI_DEVICE_0) {
        Some(0) => {}
        _ => return false,
    }
    matches!(send_command(chn, GECKO_CMD_IDENT), Some(r) if r & 0x0470 != 0)
}

/// `usb_sendbuffer` — write all bytes (blocks on the tx fifo).
pub fn send_buffer(chn: u32, data: &[u8]) -> usize {
    if !exi::lock(chn, exi::EXI_DEVICE_0) {
        return 0;
    }
    let mut n = 0;
    while n < data.len() {
        // wait for fifo space (bounded — a stalled host shouldn't hang us)
        let mut tries = 0u32;
        while !matches!(send_command_locked(chn, GECKO_CMD_CHKSEND), Some(r) if r & 0x0400 != 0) {
            tries += 1;
            if tries > 500_000 {
                exi::unlock(chn);
                return n;
            }
        }
        let cmd = GECKO_CMD_SEND | ((data[n] as u16) << 4);
        if !matches!(send_command_locked(chn, cmd), Some(r) if r & 0x0400 != 0) {
            break;
        }
        n += 1;
    }
    exi::unlock(chn);
    n
}

/// `usb_recvbuffer` — read up to `buf.len()` bytes that are available
/// (non-blocking: returns what's in the fifo).
pub fn recv_buffer(chn: u32, buf: &mut [u8]) -> usize {
    if !exi::lock(chn, exi::EXI_DEVICE_0) {
        return 0;
    }
    let mut n = 0;
    while n < buf.len()
        && matches!(send_command_locked(chn, GECKO_CMD_CHKRECV), Some(r) if r & 0x0400 != 0)
    {
        match send_command_locked(chn, GECKO_CMD_RECV) {
            Some(reply) if reply & 0x0800 != 0 => {
                buf[n] = (reply & 0xff) as u8;
                n += 1;
            }
            _ => break,
        }
    }
    exi::unlock(chn);
    n
}
