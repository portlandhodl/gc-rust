//! Broadband Adapter (BBA) Ethernet MAC driver — synchronous port of
//! libogc's `libdb/uIP/bba.c` register layer (Broadcom "Network Interface
//! Controller" behind EXI channel 0, device 2).
//!
//! Wire protocol over EXI (32 MHz):
//! ```text
//! cmd frame = (reg << 8) | 0xC000_0000  (write) / 0x8000_0000 (read)
//! then EXI payload (immex)
//! ```
//!
//! Ring-buffer page scheme (256 B pages) like a NIC839x: TX writes via the
//! WRTXFIFOD port, TX count in BBA_TXFIFOCNT, start with NCRA_ST1.
//!
//! API: [`probe`], [`init`], [`send_frame`], [`poll_frame`]. No interrupts —
//! RX is drained by polling RRP/RWP.

use crate::{exi, hw};

use alloc::vec::Vec;

pub const BBA_CID: u32 = 0x0402_0200;
pub const BBA_MAX_PACKET: usize = 1536;

// register map (from bba.c)
#[allow(dead_code)]
mod reg {
    pub const NCRA: u32 = 0x00;
    pub const NCRB: u32 = 0x01;
    pub const IMR: u32 = 0x08;
    pub const IR: u32 = 0x09;
    pub const BP: u32 = 0x0a;
    pub const TLBP: u32 = 0x0c;
    pub const TWP: u32 = 0x0e;
    pub const TRP: u32 = 0x12;
    pub const RXINTT: u32 = 0x14;
    pub const RWP: u32 = 0x16;
    pub const RRP: u32 = 0x18;
    pub const RHBP: u32 = 0x1a;
    pub const NAFR_PAR0: u32 = 0x20;
    pub const GCA: u32 = 0x32;
    pub const MISCBURST: u32 = 0x3d;
    pub const TXFIFOCNT: u32 = 0x3e;
    pub const WRTXFIFOD: u32 = 0x48;
    pub const MISC2: u32 = 0x50;
}

mod bits {
    pub const NCRA_ST0: u8 = 1 << 1;
    pub const NCRA_ST1: u8 = 1 << 2;
    pub const NCRA_SR: u8 = 1 << 3;
    pub const NCRA_RESET: u8 = 1 << 0;

    pub const NCRB_CA: u8 = 1 << 1;
    pub const NCRB_AB: u8 = 1 << 4;

    pub const NWAYS_LS10: u8 = 1 << 0;
    pub const NWAYS_LS100: u8 = 1 << 1;

    pub const GCA_ARXERRB: u8 = 1 << 3;
    pub const MISC2_AUTORCVR: u8 = 1 << 7;
}

const BBA_INIT_TLBP: u32 = 0x00;
const BBA_INIT_BP: u32 = 0x01;
const BBA_INIT_RHBP: u32 = 0x0f;

// BBA sits on EXI channel 0 ("Memory Card slot A"? no — the BBA *is* the
// Secret Serial Port (ch2) mirrored... libogc reads its registers through
// CH0/DEV2: the BBA chip expose its register file at EXI channel 0 dev 2.
const CHN: u32 = exi::EXI_CHANNEL_0;
const DEV: u32 = exi::EXI_DEVICE_2;


// ------ EXI comfort helpers (Select/Imm lockstep + SYNC) --------------------

#[inline(always)]
fn bba_select() -> bool {
    exi::select(CHN, DEV, exi::EXI_SPEED32MHZ)
}

#[inline(always)]
fn bba_deselect() -> bool {
    exi::deselect(CHN)
}

// Two command families (both libogc used):
//
// * u16 "cmd" family (control regs 0x00..0x0f: CID, host irq mask, devid,
//   acstart, challenge/response):
//     u16 frame: read  = (reg << 8)
//     u16 frame: write = (reg << 8) | 0x4000
// * u32 family (the NIC register file: NCRA..NAFR / RX ring descriptors /
//   TX FIFO port):
//     u32 frame: read  = (reg << 8) | 0x8000_0000
//     u32 frame: write = (reg << 8) | 0xC000_0000

// ---------------------------------------------------------------------------
// family helpers
// ---------------------------------------------------------------------------

fn cmd_frame16(reg: u32, write: bool) -> [u8; 2] {
    // u16 frame
    ((reg << 8) as u16 | if write { 0x4000u16 } else { 0 }).to_be_bytes()
}

fn data_frame32(reg: u32, write: bool) -> [u8; 4] {
    ((reg << 8) as u32 | if write { 0xC000_0000 } else { 0x8000_0000 }).to_be_bytes()
}

#[allow(dead_code)]
fn cmd_ins(reg: u32, data: &mut [u8]) {
    if !bba_select() { return; }
    let mut f = cmd_frame16(reg, false);
    let _ = exi::imm_ex(CHN, &mut f, exi::EXI_WRITE);
    let _ = exi::imm_ex(CHN, data, exi::EXI_READ);
    bba_deselect();
}

fn cmd_outs(reg: u32, data: &[u8]) {
    if !bba_select() { return; }
    let mut f = cmd_frame16(reg, true);
    if !exi::imm_ex(CHN, &mut f, exi::EXI_WRITE) { bba_deselect(); return; }
    let mut d = data.to_vec();
    let _ = exi::imm_ex(CHN, &mut d, exi::EXI_WRITE);
    bba_deselect();
}

#[allow(dead_code)]
fn cmd_in8(reg: u32) -> u8 {
    let mut b = [0u8; 1];
    cmd_ins(reg, &mut b);
    b[0]
}

fn cmd_out8(reg: u32, v: u8) {
    cmd_outs(reg, &[v]);
}

fn ring_ins(reg: u32, data: &mut [u8]) {
    if !bba_select() { return; }
    let mut f = data_frame32(reg, false);
    let _ = exi::imm_ex(CHN, &mut f, exi::EXI_WRITE);
    let _ = exi::imm_ex(CHN, data, exi::EXI_READ);
    bba_deselect();
}

fn ring_outs(reg: u32, data: &[u8]) {
    if !bba_select() { return; }
    let mut f = data_frame32(reg, true);
    if !exi::imm_ex(CHN, &mut f, exi::EXI_WRITE) { bba_deselect(); return; }
    let mut d = data.to_vec();
    let _ = exi::imm_ex(CHN, &mut d, exi::EXI_WRITE);
    bba_deselect();
}

fn in8(reg: u32) -> u8 {
    let mut b = [0u8; 1];
    ring_ins(reg, &mut b);
    b[0]
}

fn out8(reg: u32, v: u8) {
    ring_outs(reg, &[v]);
}

fn out12(reg: u32, v: u32) {
    out8(reg, (v & 0xff) as u8);
    out8(reg + 1, ((v >> 8) & 0x0f) as u8);
}

fn in12(reg: u32) -> u32 {
    (in8(reg) as u32) | (((in8(reg + 1) & 0x0f) as u32) << 8)
}

// ---------------------------------------------------------------------------
// bring-up
// ---------------------------------------------------------------------------

/// Probe: read the chip ID from the BBA (CID magic).
pub fn probe() -> bool {
    if !exi::lock(CHN, DEV) {
        return false;
    }
    let mut id = [0u8; 4];
    let mut zero = [0u16; 1][0].to_be_bytes();
    if bba_select() {
        exi::imm_ex(CHN, &mut zero, exi::EXI_WRITE);
        exi::imm_ex(CHN, &mut id, exi::EXI_READ);
        bba_deselect();
    }
    exi::unlock(CHN);
    u32::from_be_bytes(id) == BBA_CID
}

/// Hardware bring-up. `mac` = our ethernet source address (any locally
/// administered MAC is fine; pass None for the documented default).
pub fn init(mac: Option<[u8; 6]>) -> bool {
    let mac = mac.unwrap_or([0x02, 0xf7, 0xb0, 0x30, 0x26, 0xfe]);

    if !exi::lock(CHN, DEV) {
        return false;
    }
    let mut ok = true;

    // reset dance
    out8(0x60, 0x00);
    hw::gcdelay(10_000); // 10 ms
    {
        // slow read-back of 0x0F (HW rev?) per __bba_reset
        let mut v = [0u8; 1];
        // bba_cmd_in8_slow
        let mut f = [((0x0F << 8) as u16).to_be_bytes(); 1][0];
        if bba_select() {
            exi::imm_ex(CHN, &mut f, exi::EXI_WRITE);
            exi::imm_ex(CHN, &mut v, exi::EXI_READ);
            hw::gcdelay(200);
            bba_deselect();
        }
    }
    hw::gcdelay(10_000);
    out8(reg::NCRA, bits::NCRA_RESET);
    out8(reg::NCRA, 0x00);

    // gmac config (control registers — the u16 'cmd' family)
    cmd_outs(0x04, &[0xD1, 0x07]); // devid
    cmd_out8(0x05, 0x4e); // acstart
    let x5b = in8(0x5b);
    out8(0x5b, x5b & !0x80);
    out8(0x5e, 0x01);
    let x5c = in8(0x5c);
    out8(0x5c, x5c | 0x04);

    // rx init
    out8(reg::NCRB, bits::NCRB_CA | bits::NCRB_AB);
    out8(reg::MISC2, bits::MISC2_AUTORCVR);

    out12(reg::TLBP, BBA_INIT_TLBP);
    out12(reg::BP, BBA_INIT_BP);
    out12(reg::RWP, BBA_INIT_BP);
    out12(reg::RRP, BBA_INIT_BP);
    out12(reg::RHBP, BBA_INIT_RHBP);
    out8(reg::GCA, bits::GCA_ARXERRB);
    out8(reg::NCRA, bits::NCRA_SR);

    // MAC address: read-back from the chip (factory or our override)
    let mut macbuf = mac;
    ring_ins(reg::NAFR_PAR0, &mut macbuf);
    unsafe { CURRENT_MAC = macbuf; }

    // irq mask: none of the in-driver interrupts are polled via IMR path
    // (we poll RRP/RWP directly)
    cmd_out8(reg::IR, 0xFF); // IR: ack everything
    cmd_out8(reg::IMR, 0xFF & !(1u8 << 5)); // IMR: all but FIFOE

    // wait for link up
    let start = hw::mftb();
    loop {
        if in8(0x31) & (bits::NWAYS_LS10 | bits::NWAYS_LS100) != 0 {
            break;
        }
        if hw::mftb().wrapping_sub(start) > 41_000_000 * 5 {
            ok = false;
            break;
        }
        hw::gcdelay(500);
    }

    exi::unlock(CHN);
    ok
}


static mut CURRENT_MAC: [u8; 6] = [0; 6];

/// The chip's MAC (valid after [`init`]).
pub fn mac() -> [u8; 6] {
    unsafe { CURRENT_MAC }
}

/// Link up?
pub fn link_up() -> bool {
    let run = |_: ()| -> bool {
        let r = in8(0x31);
        r & (bits::NWAYS_LS10 | bits::NWAYS_LS100) != 0
    };
    let _ = exi::lock(CHN, DEV);
    let r = run(());
    let _ = exi::unlock(CHN);
    r
}

// ---------------------------------------------------------------------------
// TX
// ---------------------------------------------------------------------------

/// Send a raw ethernet frame (`frame` includes dst/src/ethertype, no FCS —
/// hardware adds it).
pub fn send_frame(frame: &[u8]) -> bool {
    if frame.len() > 1518 {
        return false;
    }
    if !exi::lock(CHN, DEV) {
        return false;
    }
    if !link_up() {
        exi::unlock(CHN);
        return false;
    }

    // wait for a parked tx
    while in8(reg::NCRA) & (bits::NCRA_ST0 | bits::NCRA_ST1) != 0 {}

    out12(reg::TXFIFOCNT, frame.len() as u32);

    if bba_select() {
        // write to data port
        let mut f = data_frame32(reg::WRTXFIFOD, true);
        if exi::imm_ex(CHN, &mut f, exi::EXI_WRITE) {
            let mut d = frame.to_vec();
            if !exi::imm_ex(CHN, &mut d, exi::EXI_WRITE) {
                bba_deselect();
                exi::unlock(CHN);
                return false;
            }
            if frame.len() < 60 {
                let padlen = 60 - frame.len();
                let mut pad = Vec::new();
                pad.resize(padlen, 0);
                let _ = exi::imm_ex(CHN, &mut pad, exi::EXI_WRITE);
            }
        }
        bba_deselect();
    }

    // kick
    let cur = in8(reg::NCRA);
    out8(reg::NCRA, (cur & !bits::NCRA_ST0) | bits::NCRA_ST1);
    exi::unlock(CHN);
    true
}

// ---------------------------------------------------------------------------
// RX (bg poll: pull one frame if present)
// ---------------------------------------------------------------------------

/// bba_descr: 32-bit field packing (next_packet_ptr:12 | packet_len:12) and status.
#[derive(Copy, Clone, Default)]
struct BbaDescr {
    next_packet_ptr: u16,
    packet_len: u16,
    status: u8,
}

// X(X(next_packet_ptr:12, packet_len:12), status:8) — one LE u32 in memory:
fn descr_from(raw: u32) -> BbaDescr {
    let status = ((raw >> 24) & 0xff) as u8;
    let packet_len = ((raw >> 12) & 0xfff) as u16;
    let next_packet_ptr = (raw & 0x0fff) as u16;
    BbaDescr { next_packet_ptr, packet_len, status }
}

/// Poll for one RX frame into `buf`; returns Some(len) if anything was read.
pub fn poll_frame(buf: &mut [u8]) -> Option<usize> {
    let mut out = None;
    if !exi::lock(CHN, DEV) {
        return None;
    }
    let rwp = in12(reg::RWP);
    let mut rrp = in12(reg::RRP);
    if rrp != rwp {
        // read descriptor at ring[rrp*256]
        let mut d4 = [0u8; 4];
        ring_ins(rrp << 8, &mut d4);
        let d = descr_from(u32::from_le_bytes(d4));
        let size = (d.packet_len as usize).saturating_sub(4);

        if size <= BBA_MAX_PACKET && d.status & 0x83 == 0 {
            // payload follows the 4-byte header
            let pos = (rrp as usize * 256) + 4;
            let top = (BBA_INIT_RHBP as usize + 1) * 256;
            if pos + size <= top && size <= buf.len() {
                let tmp = pos as u32;
                ring_ins(tmp, &mut buf[..size]);
            } else {
                // wrapped — two reads
                let chunk = (top - pos.min(top)).min(size).min(buf.len());
                ring_ins(pos as u32, &mut buf[..chunk]);
                if size > chunk && size - chunk <= buf.len().saturating_sub(chunk) {
                    ring_ins((BBA_INIT_BP as usize * 256) as u32, &mut buf[chunk..chunk + (size - chunk)]);
                }
            }
            if size <= buf.len() {
                out = Some(size);
            }
        }
        rrp = d.next_packet_ptr as u32;
        out12(reg::RRP, rrp);
    }
    exi::unlock(CHN);
    out
}
