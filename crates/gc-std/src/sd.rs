//! SD/SDHC card over an SD Gecko adapter (SPI mode) — synchronous pure-Rust
//! port of libogc's `sdgecko_io.c` (`__card_*` command layer).
//!
//! The SD Gecko maps an SD card onto a memory-card slot's EXI lines and
//! speaks raw SPI. Command frames are `{0x40|idx, arg[4]}` + CRC7|1;
//! responses are polled 0xFF-clocks until !bit7; data blocks carry a 0xFE
//! start token and a CRC16 trailer.
//!
//! The SPI bus layer is a trait: on the console it's EXI
//! ([`SdSpiExi`]); host tests drive the same state machine against an
//! emulated card.
//!
//! Scope: single-block reads/writes (CMD17/CMD24) at 512-byte blocks;
//! multi-block (CMD18/25) is future work — the command layers are there.

use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// SPI bus trait
// ---------------------------------------------------------------------------

pub trait SdSpi {
    /// Clock the bus `n` bytes with CS *high* (power-up/init dummy clocks;
    /// the SD spec wants ≥74).
    fn idle_clocks(&mut self, n: usize) -> Result<(), i32>;
    /// Assert CS and run the bus at `fast` (false = 400 kHz init rate).
    fn select(&mut self, fast: bool) -> Result<(), i32>;
    /// Release CS.
    fn deselect(&mut self) -> Result<(), i32>;
    /// Full-duplex byte: write `byte`, return the simultaneously-read byte.
    fn transfer(&mut self, byte: u8) -> Result<u8, i32>;
    /// Write-only block.
    fn write_bytes(&mut self, data: &[u8]) -> Result<(), i32>;
    /// Read `buf.len()` bytes (driving 0xFF on the bus).
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<(), i32>;
}

// ---------------------------------------------------------------------------
// EXI backend (console): SD Gecko lives on a memcard slot, EXI device 0
// ---------------------------------------------------------------------------

/// Attach to the SD Gecko adapter in `slot` (0/1) and run card init.
#[cfg(target_arch = "powerpc")]
pub fn connect(slot: u32) -> Result<Sd, i32> {
    use crate::exi;
    if slot > 2 {
        return Err(SD_ERROR_NOCARD);
    }
    if !exi::probe(slot) {
        return Err(SD_ERROR_NOCARD);
    }
    struct ExiSpi {
        chn: u32,
    }
    impl SdSpi for ExiSpi {
        fn idle_clocks(&mut self, n: usize) -> Result<(), i32> {
            // CS high: EXI_SelectSD selects the channel without the CS line
            if !exi::select_sd(self.chn, exi::EXI_DEVICE_0, exi::EXI_SPEED1MHZ) {
                return Err(SD_ERROR_IOERROR);
            }
            let mut d = [0xffu8; 128];
            let mut left = n;
            while left > 0 {
                let chunk = left.min(128);
                if !exi::imm_ex(self.chn, &mut d[..chunk], exi::EXI_WRITE) {
                    exi::deselect(self.chn);
                    return Err(SD_ERROR_IOERROR);
                }
                left -= chunk;
            }
            exi::deselect(self.chn);
            Ok(())
        }
        fn select(&mut self, fast: bool) -> Result<(), i32> {
            let speed = if fast { exi::EXI_SPEED32MHZ } else { exi::EXI_SPEED1MHZ };
            if exi::select(self.chn, exi::EXI_DEVICE_0, speed) {
                Ok(())
            } else {
                Err(SD_ERROR_NOCARD)
            }
        }
        fn deselect(&mut self) -> Result<(), i32> {
            exi::deselect(self.chn);
            Ok(())
        }
        fn transfer(&mut self, byte: u8) -> Result<u8, i32> {
            let mut b = [byte];
            if !exi::imm(self.chn, &mut b, exi::EXI_READWRITE) {
                return Err(SD_ERROR_IOERROR);
            }
            Ok(b[0])
        }
        fn write_bytes(&mut self, data: &[u8]) -> Result<(), i32> {
            let mut tmp = alloc::vec::Vec::from(data);
            if exi::imm_ex(self.chn, &mut tmp, exi::EXI_WRITE) {
                Ok(())
            } else {
                Err(SD_ERROR_IOERROR)
            }
        }
        fn read_bytes(&mut self, buf: &mut [u8]) -> Result<(), i32> {
            if exi::imm_ex(self.chn, buf, exi::EXI_READ) {
                Ok(())
            } else {
                Err(SD_ERROR_IOERROR)
            }
        }
    }

    if !exi::lock(slot, exi::EXI_DEVICE_0) {
        return Err(SD_ERROR_BUSY);
    }
    static mut SPI: [Option<ExiSpi>; 2] = [None, None];
    let ret = unsafe {
        SPI[slot as usize] = Some(ExiSpi { chn: slot });
        let spi: &'static mut dyn SdSpi = (*(&raw mut SPI))[slot as usize].as_mut().unwrap();
        init(spi)
    };
    if ret.is_err() {
        exi::unlock(slot);
        unsafe { (*(&raw mut SPI))[slot as usize] = None };
    }
    ret
}

// ---------------------------------------------------------------------------
// errors (mirror libogc's cardio codes)
// ---------------------------------------------------------------------------

pub const SD_ERROR_READY: i32 = 0;
pub const SD_ERROR_BUSY: i32 = -1;
pub const SD_ERROR_WRONGDEVICE: i32 = -2;
pub const SD_ERROR_NOCARD: i32 = -3;
pub const SD_ERROR_IOERROR: i32 = -4;
pub const SD_ERROR_IOTIMEOUT: i32 = -5;
pub const SD_ERROR_CRC: i32 = -6;
pub const SD_ERROR_UNUSABLE: i32 = -7;

// ---------------------------------------------------------------------------
// CRC tables (spec: CRC7 for command frames, CRC16-CCITT for data)
// ---------------------------------------------------------------------------

/// `__make_crc7`.
fn crc7(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        for i in 0..8 {
            crc <<= 1;
            let bit = if b & (0x80 >> i) != 0 { 1 } else { 0 };
            crc ^= if (crc & 0x10) != 0 { 0x09 ^ bit } else { bit };
        }
    }
    crc & 0x7f
}

/// `__make_crc16` — CCITT over the data block.
fn crc16(data: &[u8]) -> u16 {
    // matches libogc's table-driven version
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

// ---------------------------------------------------------------------------
// SD command layer (`__card_writecmd` / `__card_readresponse` / ...)
// ---------------------------------------------------------------------------

struct Cmd(u8, u32);

fn send_cmd(spi: &mut dyn SdSpi, fast: bool, cmd: Cmd) -> Result<(), i32> {
    let buf = [
        0x40 | cmd.0,
        (cmd.1 >> 24) as u8,
        (cmd.1 >> 16) as u8,
        (cmd.1 >> 8) as u8,
        cmd.1 as u8,
    ];
    let crc = crc7(&buf) | 0x01;

    spi.select(fast)?;
    // lead-in dummy clocks
    let mut d = [0xff; 10];
    spi.write_bytes(&mut d)?;
    spi.write_bytes(&buf)?;
    spi.write_bytes(&[crc])?;
    Ok(())
}

/// `__card_readresponse` — poll for !bit7 (≤64 clocks), then read len-1
/// more bytes if the command wants a wider response (R7/OCR/etc).
fn read_response_bytes(spi: &mut dyn SdSpi, len: usize) -> Result<Vec<u8>, i32> {
    let mut first = 0u8;
    let mut found = false;
    for _ in 0..64 {
        first = spi.transfer(0xff)?;
        if first & 0x80 == 0 {
            found = true;
            break;
        }
    }
    if !found {
        return Err(SD_ERROR_IOTIMEOUT);
    }
    let mut out = Vec::new(); out.push(first);
    for _ in 1..len {
        out.push(spi.transfer(0xff)?);
    }
    Ok(out)
}

/// `__card_datares` — the write-path busy/status dance. Returns the data
/// response token (the *first* byte; e.g. 0x05 = accepted).
fn write_data_response(spi: &mut dyn SdSpi) -> Result<u8, i32> {
    let resp = spi.transfer(0xff)?;
    // bit4 set → mid-transfer stall (shouldn't happen post-frame)
    let mut b = resp;
    let mut spins = 0u32;
    while b & 0x10 != 0 {
        b = spi.transfer(0xff)?;
        spins += 1;
        if spins > 8_000_000 {
            return Err(SD_ERROR_IOTIMEOUT);
        }
    }
    // program busy: 0x00 until flash is written
    let mut b2 = spi.transfer(0xff)?;
    spins = 0;
    while b2 == 0 {
        b2 = spi.transfer(0xff)?;
        spins += 1;
        if spins > 8_000_000 {
            return Err(SD_ERROR_IOTIMEOUT);
        }
    }
    Ok(resp)
}

// command indices
const CMD0: u8 = 0;
const CMD8: u8 = 8;
#[allow(dead_code)]
const CMD9: u8 = 9;
#[allow(dead_code)]
const CMD12: u8 = 12;
const CMD16: u8 = 16;
const CMD17: u8 = 17;
const CMD24: u8 = 24;
const CMD41: u8 = 41;
const CMD55: u8 = 55;
const CMD58: u8 = 58;

// ---------------------------------------------------------------------------
// public driver
// ---------------------------------------------------------------------------

/// Card handle.
pub struct Sd {
    sdhc: bool,
    blocks: u32,
    spi: &'static mut dyn SdSpi,
}

/// Is anything inserted (EXI EXT line)?
pub fn probe(_slot: u32) -> bool {
    #[cfg(target_arch = "powerpc")]
    {
        crate::exi::probe(_slot)
    }
    #[cfg(not(target_arch = "powerpc"))]
    {
        true
    }
}

/// `sdio_Startup`-ish: run the full SD init sequence (CMD0 → CMD8 → ACMD41
/// → CMD58 → CMD16) against `spi`. Grain of select/deselect framing is
/// handled per command — each public op owns the bus.
pub fn init(spi: &'static mut dyn SdSpi) -> Result<Sd, i32> {
    // powerup clocks with CS high, then CMD0
    spi.idle_clocks(74)?;
    send_cmd(spi, false, Cmd(CMD0, 0))?;
    let r1 = read_response_bytes(spi, 1)?[0];
    spi.deselect()?;
    if r1 & !0x01 != 0 {
        return Err(SD_ERROR_UNUSABLE);
    }

    // CMD8: if the card echoes 0xAA it's SDv2+
    let mut sdhc = false;
    send_cmd(spi, false, Cmd(CMD8, 0x1AA))?;
    match read_response_bytes(spi, 5) {
        Ok(r) => {
            if r.len() == 5 && r[4] == 0xAA && r[3] == 0x01 {
                sdhc = true;
            }
        }
        Err(_) => {} // v1 card: no CMD8
    }
    spi.deselect()?;

    // init loop: ACMD41 (sdhc) or CMD1
    let deadline = crate::hw::mftb() + 41_000_000; // ~1 s
    loop {
        if sdhc {
            send_cmd(spi, false, Cmd(CMD55, 0))?;
            read_response_bytes(spi, 1)?;
            spi.deselect()?;
            send_cmd(spi, false, Cmd(CMD41, 0x4000_0000))?;
        } else {
            send_cmd(spi, false, Cmd(1, 0))?;
        }
        let r1 = read_response_bytes(spi, 1)?[0];
        spi.deselect()?;
        if r1 == 0x00 {
            break; // ready
        }
        if crate::hw::mftb() > deadline {
            return Err(SD_ERROR_IOTIMEOUT);
        }
    }

    // OCR: block addressing?
    send_cmd(spi, true, Cmd(CMD58, 0))?;
    let ocr = read_response_bytes(spi, 5)?; // R3: r1 + 4-byte OCR
    spi.deselect()?;
    let ccs = if sdhc && ocr.len() >= 5 { ocr[1] & 0x40 != 0 } else { false };

    // 512-byte blocks for legacy cards
    if !ccs {
        send_cmd(spi, true, Cmd(CMD16, 512))?;
        let r1 = read_response_bytes(spi, 1)?[0];
        spi.deselect()?;
        if r1 != 0 {
            return Err(SD_ERROR_UNUSABLE);
        }
    }

    Ok(Sd { sdhc: ccs, blocks: 0, spi })
}

/// Number of 512-byte blocks reported by the card (0 when unknown).
pub fn blocks(sd: &Sd) -> u32 {
    sd.blocks
}

/// Read one 512-byte block (CMD17).
pub fn read_block(sd: &mut Sd, lba: u32, buf: &mut [u8; 512]) -> i32 {
    let addr = if sd.sdhc { lba } else { lba.wrapping_mul(512) };
    if send_cmd(sd.spi, true, Cmd(CMD17, addr)).is_err() {
        return SD_ERROR_IOERROR;
    }
    let r1 = read_response_bytes(sd.spi, 1);
    let r1 = match r1 {
        Ok(v) => v[0],
        Err(e) => {
            let _ = sd.spi.deselect();
            return e;
        }
    };
    if r1 != 0 {
        let _ = sd.spi.deselect();
        return SD_ERROR_IOERROR;
    }

    // wait for the 0xFE data token
    let mut spins = 0u32;
    loop {
        let b = match sd.spi.transfer(0xff) {
            Ok(b) => b,
            Err(_) => {
                let _ = sd.spi.deselect();
                return SD_ERROR_IOERROR;
            }
        };
        if b == 0xFE {
            break;
        }
        spins += 1;
        if spins > 8_000_000 {
            let _ = sd.spi.deselect();
            return SD_ERROR_IOTIMEOUT;
        }
    }

    let mut ok = 0;
    if sd.spi.read_bytes(&mut buf[..]).is_err() {
        ok = SD_ERROR_IOERROR;
    }
    let mut trailer = [0u8; 2];
    let _ = sd.spi.read_bytes(&mut trailer);
    let _ = sd.spi.deselect();
    if ok != 0 {
        return ok;
    }

    let crc_rx = ((trailer[0] as u16) << 8) | trailer[1] as u16;
    if crc_rx != crc16(buf) {
        return SD_ERROR_CRC;
    }
    SD_ERROR_READY
}

/// Write one 512-byte block (CMD24). Caller must have erased nothing —
/// flash cards self-manage; the card signals completion through the busy
/// byte dance (`write_data_response`).
pub fn write_block(sd: &mut Sd, lba: u32, buf: &[u8; 512]) -> i32 {
    let addr = if sd.sdhc { lba } else { lba.wrapping_mul(512) };
    if send_cmd(sd.spi, true, Cmd(CMD24, addr)).is_err() {
        return SD_ERROR_IOERROR;
    }
    let r1 = match read_response_bytes(sd.spi, 1) {
        Ok(v) => v[0],
        Err(e) => {
            let _ = sd.spi.deselect();
            return e;
        }
    };
    if r1 != 0 {
        let _ = sd.spi.deselect();
        return SD_ERROR_IOERROR;
    }

    let mut frame = Vec::with_capacity(516);
    frame.push(0xFEu8);
    frame.extend_from_slice(buf);
    frame.extend_from_slice(&crc16(buf).to_be_bytes());
    if sd.spi.write_bytes(&frame).is_err() {
        let _ = sd.spi.deselect();
        return SD_ERROR_IOERROR;
    }
    let resp = match write_data_response(sd.spi) {
        Ok(r) => r,
        Err(e) => {
            let _ = sd.spi.deselect();
            return e;
        }
    };
    let _ = sd.spi.deselect();
    match (resp >> 1) & 0x07 {
        0b010 => SD_ERROR_READY, // data accepted
        0b101 => SD_ERROR_CRC,
        _ => SD_ERROR_IOERROR,
    }
}
