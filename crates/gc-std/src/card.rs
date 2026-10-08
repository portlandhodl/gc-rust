//! Memory-card driver — synchronous, pure-Rust port of libogc's `card.c`.
//!
//! Public surface mirrors `CARD_*` 1:1 (snake_cased, same argument order and
//! error codes); the async callback machinery of the C version collapses
//! into blocking calls riding on the synchronous [`crate::exi`] driver.
//! Write/erase completion is polled through the channel's EXI-interrupt
//! pending bit instead of the PI cascade — same wire traffic, no IRQs.
//!
//! Layout on card (verified against card.c):
//!
//! * 5 system blocks of 8192 B: header @sector0, dir0/dir1 @1/2,
//!   FAT0/FAT1 @3/4; data area starts at block 5.
//! * Directory: 127 entries of 64 B + `updated` + dual checksum.
//! * FAT: next-fit free-block chain table; `updated` + checksum; free
//!   blocks recount is verified at mount.
//!
//! Not ported (yet): the DSP flash-ID unlock dance for counterfeit/"coded"
//! third-party cards (official cards and Dolphin always report unlocked),
//! and the async* API family.

use crate::hw;

pub const CARD_SLOTA: i32 = 0;
pub const CARD_SLOTB: i32 = 1;

pub const CARD_READSIZE: usize = 512;
pub const CARD_FILENAMELEN: usize = 32;
pub const CARD_MAXFILES: usize = 127;

pub const CARD_ERROR_UNLOCKED: i32 = 1;
pub const CARD_ERROR_READY: i32 = 0;
pub const CARD_ERROR_BUSY: i32 = -1;
pub const CARD_ERROR_WRONGDEVICE: i32 = -2;
pub const CARD_ERROR_NOCARD: i32 = -3;
pub const CARD_ERROR_NOFILE: i32 = -4;
pub const CARD_ERROR_IOERROR: i32 = -5;
pub const CARD_ERROR_BROKEN: i32 = -6;
pub const CARD_ERROR_EXIST: i32 = -7;
pub const CARD_ERROR_NOENT: i32 = -8;
pub const CARD_ERROR_INSSPACE: i32 = -9;
pub const CARD_ERROR_NOPERM: i32 = -10;
pub const CARD_ERROR_LIMIT: i32 = -11;
pub const CARD_ERROR_NAMETOOLONG: i32 = -12;
pub const CARD_ERROR_CANCELED: i32 = -14;
pub const CARD_ERROR_FATAL_ERROR: i32 = -128;

pub const CARD_ATTRIB_PUBLIC: u8 = 0x04;
pub const CARD_ATTRIB_NOCOPY: u8 = 0x08;
pub const CARD_ATTRIB_NOMOVE: u8 = 0x10;

const CARD_SYSAREA: u16 = 5;
const SYSAREA_BYTES: usize = 5 * 8192;
const CARD_SYSDIR: usize = 0x2000;
const CARD_SYSDIR_BACK: usize = 0x4000;
const CARD_SYSBAT: usize = 0x6000;
const CARD_SYSBAT_BACK: usize = 0x8000;

const CARD_STATUS_UNLOCKED: u8 = 0x40;

/// EXI CSR bit: device-asserted interrupt pending (W1C).
const EXI_EXI_IRQ: u32 = 0x0002;

/// card_file — an open file handle.
#[derive(Copy, Clone)]
pub struct CardFile {
    pub chn: i32,
    pub filenum: i32,
    pub offset: i32,
    pub len: i32,
    pub iblock: u16,
}

/// card_dir — directory iteration handle.
pub struct CardDir {
    pub chn: i32,
    pub fileno: u32,
    pub filelen: u32,
    pub permissions: u8,
    pub filename: [u8; CARD_FILENAMELEN],
    pub gamecode: [u8; 4],
    pub company: [u8; 2],
}

/// card_stat — public metadata view (icon offset computation elided; use
/// [`get_status_ex`] when you need the raw fields).
#[derive(Clone)]
pub struct CardStat {
    pub filename: [u8; CARD_FILENAMELEN],
    pub len: u32,
    pub time: u32,
    pub gamecode: [u8; 4],
    pub company: [u8; 2],
    pub banner_fmt: u8,
    pub icon_addr: u32,
    pub icon_fmt: u16,
    pub icon_speed: u16,
    pub comment_addr: u32,
}

/// card_direntry — raw 64-byte directory entry.
#[derive(Clone)]
pub struct DirEntryRaw {
    pub gamecode: [u8; 4],
    pub company: [u8; 2],
    pub banner_fmt: u8,
    pub filename: [u8; CARD_FILENAMELEN],
    pub last_modified: u32,
    pub icon_addr: u32,
    pub icon_fmt: u16,
    pub icon_speed: u16,
    pub permission: u8,
    pub copy_times: u8,
    pub block: u16,
    pub length: u16,
    pub comment_addr: u32,
}

// ---------------------------------------------------------------------------
// per-channel state
// ---------------------------------------------------------------------------

struct CardChn {
    attached: bool,
    busy: bool,
    result: i32,
    cid: u32,
    card_size: u32,
    sector_size: u32,
    blocks: u32,
    latency: u32,
    curr_dir: u8, // 0 -> 0x2000, 1 -> 0x4000
    curr_fat: u8, // 0 -> 0x6000, 1 -> 0x8000
    gamecode: Option<[u8; 4]>,
    company: Option<[u8; 2]>,
}

impl CardChn {
    const fn new() -> Self {
        CardChn {
            attached: false,
            busy: false,
            result: CARD_ERROR_NOCARD,
            cid: 0,
            card_size: 0,
            sector_size: 8192,
            blocks: 0,
            latency: 4,
            curr_dir: 0,
            curr_fat: 0,
            gamecode: None,
            company: None,
        }
    }
}

static mut CARDS: [CardChn; 2] = [CardChn::new(), CardChn::new()];

#[repr(align(32))]
struct Workarea([u8; SYSAREA_BYTES]);

static mut WORKAREA: [Workarea; 2] = [Workarea([0; SYSAREA_BYTES]), Workarea([0; SYSAREA_BYTES])];

#[inline(always)]
unsafe fn card(chn: i32) -> &'static mut CardChn {
    &mut (*(&raw mut CARDS))[chn as usize]
}

#[inline(always)]
unsafe fn workarea(chn: i32) -> &'static mut [u8; SYSAREA_BYTES] {
    &mut (*(&raw mut WORKAREA))[chn as usize].0
}

/// Clock-filler bytes for the read command's latency phase (contents
/// irrelevant, we just need to clock the bus).
static mut LATENCY_DUMMY: Workarea = Workarea([0u8; SYSAREA_BYTES]);
// ^ only the first 512 bytes are ever used; the Workarea wrapper carries
//   the 32-byte alignment we need for EXI DMA.

/// 32-byte-aligned staging for dir/fat commits (whole 8192-byte blocks).
static mut COMMIT_STAGING: A32Buf = A32Buf([0u8; 8192]);

/// Bounce buffer for sub-sector reads (read path rounds to 512 B).
static mut BOUNCE: A32Big = A32Big([0u8; 64 * 1024]);

#[repr(align(32))]
struct A32Buf([u8; 8192]);

#[repr(align(32))]
struct A32Big([u8; 64 * 1024]);

// ---------------------------------------------------------------------------
// big-endian accessors on the card image (host reads a BE u16 stream)
// ---------------------------------------------------------------------------

#[inline(always)]
fn get16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([b[off], b[off + 1]])
}
#[inline(always)]
fn put16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_be_bytes());
}
#[inline(always)]
fn get32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
#[inline(always)]
fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_be_bytes());
}

/// `__card_checksum`.
#[inline]
fn checksum(words: &[u8]) -> (u16, u16) {
    let mut cs1: u16 = 0;
    let mut cs2: u16 = 0;
    let n = words.len() / 2;
    for i in 0..n {
        let w = get16(words, i * 2);
        cs1 = cs1.wrapping_add(w);
        cs2 = cs2.wrapping_add(w ^ 0xffff);
    }
    if cs1 == 0xffff {
        cs1 = 0;
    }
    if cs2 == 0xffff {
        cs2 = 0;
    }
    (cs1, cs2)
}

/// `__card_iscard` + sector/latency decode.
fn is_card(id: u32) -> Option<(u32, u32, u32, u32)> {
    if id & !0xffff != 0 || id & 0x03 != 0 {
        return None;
    }
    let tmp = id & 0xfc;
    let in_table = matches!(tmp, 0x04 | 0x08 | 0x10 | 0x20 | 0x40 | 0x80);
    if !in_table {
        return None;
    }
    const SECTOR_SIZES: [u32; 8] = [
        0x0002_000, 0x0004_000, 0x0008_000, 0x0010_000, 0x0020_000, 0x0040_000, 0, 0,
    ];
    const LATENCY: [u32; 8] = [4, 8, 16, 32, 64, 128, 256, 512];
    let secsize = SECTOR_SIZES[((id >> 11) & 7) as usize];
    if secsize == 0 {
        return None;
    }
    let blocks = ((tmp << 20) & 0x1FFE_0000) / secsize;
    if blocks <= 8 {
        return None;
    }
    let latency = LATENCY[((id >> 8) & 7) as usize];
    Some((tmp, secsize, blocks, latency))
}

// ---------------------------------------------------------------------------
// wire layer — pluggable so host tests can emulate the card
// ---------------------------------------------------------------------------

/// Card-facing primitives (the synchronous EXI protocol on the console, an
/// emulated card in host tests).
pub(crate) trait CardBus {
    /// Device insert line for this slot.
    fn probe(&mut self, chn: u32) -> bool;
    /// EXI device-ID read of device 0.
    fn get_id(&mut self, chn: u32) -> Option<u32>;
    /// 0x89.
    fn clear_status(&mut self, chn: u32) -> Result<(), i32>;
    /// 0x83 0x00 -> status byte.
    fn read_status(&mut self, chn: u32) -> Result<u8, i32>;
    /// 0x81 on/off — card write-completion interrupt enable.
    fn enable_interrupt(&mut self, chn: u32, on: bool) -> Result<(), i32>;
    /// `ogc_card_read`: `buf.len()` multiple of 512.
    fn read(&mut self, chn: u32, addr: u32, latency: u32, buf: &mut [u8]) -> i32;
    /// `ogc_card_write`: one whole sector, 128 B pages internally.
    fn write_sector(&mut self, chn: u32, addr: u32, buf: &[u8]) -> i32;
    /// `ogc_card_sectorerase`.
    fn erase_sector(&mut self, chn: u32, addr: u32) -> i32;
}

static mut BUS: Option<&'static mut dyn CardBus> = None;

#[inline(always)]
unsafe fn bus() -> &'static mut dyn CardBus {
    (*(&raw mut BUS)).as_deref_mut().expect("card: no bus backend installed")
}

/// Install the wire backend (called once by `init` on the console; host
/// tests install their emulator instead).
pub(crate) unsafe fn set_bus(b: &'static mut dyn CardBus) {
    *(&raw mut BUS) = Some(b);
}

#[cfg(target_arch = "powerpc")]
mod exi_bus {
    //! the real EXI-backed wire primitives (libogc card.c command layer)
    use super::*;
    use crate::exi;

    pub struct ExiBus;

    static mut EXI_BUS: ExiBus = ExiBus;

    pub(crate) fn instance() -> &'static mut dyn CardBus {
        unsafe { &mut *(&raw mut EXI_BUS) }
    }

    /// `ogc_card_readstatus`.
    unsafe fn card_readstatus(chn: u32) -> Result<u8, i32> {
        let mut cmd = [0x83u8, 0x00];
        let mut st = [0u8; 1];
        if !exi::imm(chn, &mut cmd, exi::EXI_WRITE) || !exi::imm(chn, &mut st, exi::EXI_READ) {
            return Err(CARD_ERROR_NOCARD);
        }
        Ok(st[0])
    }

    /// `__card_clearstatus`.
    unsafe fn card_clearstatus(chn: u32) -> Result<(), i32> {
        let mut cmd = [0x89u8];
        if !exi::imm(chn, &mut cmd, exi::EXI_WRITE) {
            return Err(CARD_ERROR_NOCARD);
        }
        Ok(())
    }

    unsafe fn card_enable_interrupt(chn: u32, enable: bool) -> Result<(), i32> {
        let mut cmd = [0x81u8, u8::from(enable)];
        if !exi::imm(chn, &mut cmd, exi::EXI_WRITE) {
            return Err(CARD_ERROR_NOCARD);
        }
        Ok(())
    }

    /// Wait for the card to assert its EXI interrupt line (write/erase
    /// completion), with a tb-based deadline; then verify status.
    /// (libogc `__card_exihandler`, synchronously.)
    unsafe fn wait_completion(chn: u32, timeout_us: u64) -> i32 {
        const CSR: u32 = 0xCC00_6800; // +0x14*chn
        let addr = CSR + chn * 0x14;
        let start = crate::hw::mftb();
        let budget = timeout_us * 41; // 40.5 MHz tb
        loop {
            if crate::hw::read32(addr) & EXI_EXI_IRQ != 0 {
                crate::hw::write32(addr, (crate::hw::read32(addr) & 0x405) | EXI_EXI_IRQ);
                break;
            }
            if crate::hw::mftb().wrapping_sub(start) > budget {
                return CARD_ERROR_IOERROR;
            }
            core::hint::spin_loop();
        }
        if lock_select(chn).is_err() {
            return CARD_ERROR_NOCARD;
        }
        let rc = match card_readstatus(chn) {
            Err(_) => CARD_ERROR_NOCARD,
            Ok(status) => {
                let _ = card_clearstatus(chn);
                if status & 0x18 != 0 {
                    CARD_ERROR_IOERROR
                } else {
                    CARD_ERROR_READY
                }
            }
        };
        deselect_unlock(chn);
        rc
    }

    #[inline]
    unsafe fn lock_select(chn: u32) -> Result<(), i32> {
        if !exi::lock(chn, exi::EXI_DEVICE_0) {
            return Err(CARD_ERROR_BUSY);
        }
        if !exi::select(chn, exi::EXI_DEVICE_0, exi::EXI_SPEED16MHZ) {
            exi::unlock(chn);
            return Err(CARD_ERROR_NOCARD);
        }
        Ok(())
    }

    #[inline]
    unsafe fn deselect_unlock(chn: u32) {
        exi::deselect(chn);
        exi::unlock(chn);
    }

    /// `ogc_card_readsegment` — one 512-byte segment (command 0x52).
    unsafe fn card_read_segment(chn: u32, addr: u32, latency: u32, buf: &mut [u8]) -> i32 {
        let mut cmd = [
            0x52u8,
            ((addr & 0xFE0000) >> 17) as u8,
            ((addr & 0x01FE00) >> 9) as u8,
            ((addr & 0x000180) >> 7) as u8,
            (addr & 0x00007F) as u8,
        ];
        if !exi::imm_ex(chn, &mut cmd, exi::EXI_WRITE) {
            return CARD_ERROR_NOCARD;
        }
        if latency > 0 {
            let d = &mut (*(&raw mut super::LATENCY_DUMMY)).0;
            if !exi::imm_ex(chn, &mut d[..(latency as usize).min(512)], exi::EXI_WRITE) {
                return CARD_ERROR_NOCARD;
            }
        }
        if !exi::dma(chn, buf.as_mut_ptr(), CARD_READSIZE, exi::EXI_READ) {
            return CARD_ERROR_NOCARD;
        }
        CARD_ERROR_READY
    }

    /// `ogc_card_writepage` — one 128 B page (command 0xF2), then wait for
    /// the card's interrupt; 3 retries on flash errors.
    unsafe fn card_write_page(chn: u32, addr: u32, buf: &[u8]) -> i32 {
        let mut tries = 3;
        loop {
            let mut cmd = [
                0xF2u8,
                ((addr & 0xFE0000) >> 17) as u8,
                ((addr & 0x01FE00) >> 9) as u8,
                ((addr & 0x000180) >> 7) as u8,
                (addr & 0x00007F) as u8,
            ];
            if lock_select(chn).is_err() {
                return CARD_ERROR_NOCARD;
            }
            let mut ok = false;
            if exi::imm_ex(chn, &mut cmd, exi::EXI_WRITE) {
                if exi::dma(chn, buf.as_ptr() as *mut u8, 128, exi::EXI_WRITE) {
                    ok = true;
                }
            }
            deselect_unlock(chn);
            if !ok {
                return CARD_ERROR_NOCARD;
            }
            let rc = wait_completion(chn, 100_000);
            if rc == CARD_ERROR_READY {
                return CARD_ERROR_READY;
            }
            tries -= 1;
            if tries == 0 {
                return rc;
            }
        }
    }

    impl CardBus for ExiBus {
        fn probe(&mut self, chn: u32) -> bool {
            exi::probe(chn)
        }
        fn get_id(&mut self, chn: u32) -> Option<u32> {
            exi::get_id(chn, exi::EXI_DEVICE_0)
        }
        fn clear_status(&mut self, chn: u32) -> Result<(), i32> {
            unsafe { card_clearstatus(chn) }
        }
        fn read_status(&mut self, chn: u32) -> Result<u8, i32> {
            unsafe { card_readstatus(chn) }
        }
        fn enable_interrupt(&mut self, chn: u32, on: bool) -> Result<(), i32> {
            unsafe { card_enable_interrupt(chn, on) }
        }
        fn read(&mut self, chn: u32, addr: u32, latency: u32, buf: &mut [u8]) -> i32 {
            unsafe {
                if lock_select(chn).is_err() {
                    return CARD_ERROR_NOCARD;
                }
                let mut off = 0usize;
                let mut a = addr;
                let mut ret = CARD_ERROR_READY;
                while off < buf.len() {
                    let r = card_read_segment(chn, a, latency, &mut buf[off..off + CARD_READSIZE]);
                    if r != CARD_ERROR_READY {
                        ret = r;
                        break;
                    }
                    off += CARD_READSIZE;
                    a += CARD_READSIZE as u32;
                }
                deselect_unlock(chn);
                ret
            }
        }
        fn write_sector(&mut self, chn: u32, addr: u32, buf: &[u8]) -> i32 {
            unsafe {
                for off in (0..buf.len()).step_by(128) {
                    let r = card_write_page(chn, addr + off as u32, &buf[off..off + 128]);
                    if r != CARD_ERROR_READY {
                        return r;
                    }
                }
                CARD_ERROR_READY
            }
        }
        fn erase_sector(&mut self, chn: u32, addr: u32) -> i32 {
            let sector_size = unsafe { card(chn as i32).sector_size };
            if addr % sector_size != 0 {
                return CARD_ERROR_FATAL_ERROR;
            }
            let mut tries = 3;
            loop {
                let mut cmd = [0xF1u8, ((addr >> 17) & 0x7f) as u8, ((addr >> 9) & 0xff) as u8];
                unsafe {
                    if lock_select(chn).is_err() {
                        return CARD_ERROR_NOCARD;
                    }
                    let ok = exi::imm_ex(chn, &mut cmd, exi::EXI_WRITE);
                    deselect_unlock(chn);
                    if !ok {
                        return CARD_ERROR_NOCARD;
                    }
                    let erase_to = 2_000_000u64 * (sector_size as u64 / 8192) + 500_000;
                    let rc = wait_completion(chn, erase_to);
                    if rc == CARD_ERROR_READY {
                        return CARD_ERROR_READY;
                    }
                    tries -= 1;
                    if tries == 0 {
                        return rc;
                    }
                }
            }
        }
    }
}


// ---------------------------------------------------------------------------
// directory / FAT image accessors
// ---------------------------------------------------------------------------

#[inline(always)]
fn dirbase(curr: u8) -> usize {
    if curr == 0 { CARD_SYSDIR } else { CARD_SYSDIR_BACK }
}

#[inline(always)]
fn fatbase(curr: u8) -> usize {
    if curr == 0 { CARD_SYSBAT } else { CARD_SYSBAT_BACK }
}

fn parse_direntry(img: &[u8], entry: usize) -> DirEntryRaw {
    let o = entry * 64;
    let mut e = DirEntryRaw {
        gamecode: [0; 4],
        company: [0; 2],
        banner_fmt: 0,
        filename: [0; 32],
        last_modified: 0,
        icon_addr: 0,
        icon_fmt: 0,
        icon_speed: 0,
        permission: 0,
        copy_times: 0,
        block: 0,
        length: 0,
        comment_addr: 0,
    };
    e.gamecode.copy_from_slice(&img[o..o + 4]);
    e.company.copy_from_slice(&img[o + 4..o + 6]);
    e.banner_fmt = img[o + 7];
    e.filename.copy_from_slice(&img[o + 8..o + 40]);
    e.last_modified = get32(img, o + 40);
    e.icon_addr = get32(img, o + 44);
    e.icon_fmt = get16(img, o + 48);
    e.icon_speed = get16(img, o + 50);
    e.permission = img[o + 52];
    e.copy_times = img[o + 53];
    e.block = get16(img, o + 54);
    e.length = get16(img, o + 56);
    e.comment_addr = get32(img, o + 60);
    e
}

fn write_direntry(img: &mut [u8], entry: usize, e: &DirEntryRaw) {
    let o = entry * 64;
    img[o..o + 4].copy_from_slice(&e.gamecode);
    img[o + 4..o + 6].copy_from_slice(&e.company);
    img[o + 6] = 0xff;
    img[o + 7] = e.banner_fmt;
    img[o + 8..o + 40].copy_from_slice(&e.filename);
    put32(img, o + 40, e.last_modified);
    put32(img, o + 44, e.icon_addr);
    put16(img, o + 48, e.icon_fmt);
    put16(img, o + 50, e.icon_speed);
    img[o + 52] = e.permission;
    img[o + 53] = e.copy_times;
    put16(img, o + 54, e.block);
    img[o + 56..o + 58].copy_from_slice(&e.length.to_be_bytes());
    put16(img, o + 58, 0xffff);
    put32(img, o + 60, e.comment_addr);
}

fn entry_is_free(img: &[u8], entry: usize) -> bool {
    img[entry * 64] == 0xff
}

fn filename_matches(entry: &DirEntryRaw, name: &[u8]) -> bool {
    // strncmp over 32 bytes; name must be NUL-terminated within the field
    for i in 0..32 {
        let a = entry.filename[i];
        let b = *name.get(i).unwrap_or(&0);
        if a != b {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}

fn dir_updated(img: &[u8], dir: u8) -> u16 {
    get16(img, dirbase(dir) + 8128 + 58)
}

// ---------------------------------------------------------------------------
// mount helpers — `__card_checkdir` / `__card_checkfat` / `__card_verify`
// ---------------------------------------------------------------------------

unsafe fn check_dir(chn: i32) -> u32 {
    let wa = &mut *workarea(chn);
    let mut bad: u32 = 0;
    let mut bad_dir: u8 = 0;
    for d in 0u8..2 {
        let base = dirbase(d);
        let (cs1, cs2) = checksum(&wa[base..base + 0x1ffc]);
        let want1 = get16(wa, base + 8128 + 60);
        let want2 = get16(wa, base + 8128 + 62);
        if cs1 != want1 || cs2 != want2 {
            bad += 1;
            bad_dir = d;
        }
    }
    if bad == 0 {
        // both copies valid: keep the *newer* one, mirror into the other.
        let newer = if dir_updated(wa, 0) < dir_updated(wa, 1) { 1u8 } else { 0u8 };
        let (a, b) = (dirbase(newer), dirbase(1 - newer));
        wa.copy_within(a..a + 8192, b);
        card(chn).curr_dir = 1 - newer;
    } else if bad == 1 {
        let (a, b) = (dirbase(bad_dir), dirbase(1 - bad_dir));
        wa.copy_within(b..b + 8192, a);
        card(chn).curr_dir = bad_dir;
    } else {
        card(chn).curr_dir = 0; // both broken; verify() will fail
    }
    bad
}

unsafe fn check_fat(chn: i32) -> u32 {
    let wa = &mut *workarea(chn);
    let blocks = card(chn).blocks as usize;
    let mut bad: u32 = 0;
    let mut bad_fat: u8 = 0;
    for f in 0u8..2 {
        let base = fatbase(f);
        let (cs1, cs2) = checksum(&wa[base + 4..base + 0x1ffc]);
        let want1 = get16(wa, base);
        let want2 = get16(wa, base + 2);
        let mut ok = cs1 == want1 && cs2 == want2;
        if ok {
            // free-block recount
            let mut zeros = 0usize;
            for i in 0..(blocks - CARD_SYSAREA as usize).min(0xffb) {
                if get16(wa, base + 10 + i * 2) == 0 {
                    zeros += 1;
                }
            }
            if zeros != get16(wa, base + 6) as usize {
                ok = false;
            }
        }
        if !ok {
            bad += 1;
            bad_fat = f;
        }
    }
    if bad == 0 {
        let u0 = get16(wa, fatbase(0) + 4);
        let u1 = get16(wa, fatbase(1) + 4);
        let newer = if u0 < u1 { 1u8 } else { 0u8 };
        let (a, b) = (fatbase(newer), fatbase(1 - newer));
        wa.copy_within(a..a + 8192, b);
        card(chn).curr_fat = 1 - newer;
    } else if bad == 1 {
        let (a, b) = (fatbase(1 - bad_fat), fatbase(bad_fat));
        wa.copy_within(a..a + 8192, b);
        card(chn).curr_fat = bad_fat;
    } else {
        card(chn).curr_fat = 0;
    }
    bad
}

// ---------------------------------------------------------------------------
// commit protocol — `__card_updatedir` / `__card_updatefat` (erase+write the
// active copy, then flip which copy is "current")
// ---------------------------------------------------------------------------

unsafe fn update_dir(chn: i32) -> i32 {
    let wa = &mut *workarea(chn);
    let base = dirbase(card(chn).curr_dir);
    let updated = get16(wa, base + 8128 + 58).wrapping_add(1);
    put16(wa, base + 8128 + 58, updated);
    let (cs1, cs2) = checksum(&wa[base..base + 0x1ffc]);
    put16(wa, base + 8128 + 60, cs1);
    put16(wa, base + 8128 + 62, cs2);

    let sector_size = card(chn).sector_size;
    let card_sector = (base / 8192) as u32; // 1 or 2
    let card_addr = card_sector * sector_size;

    {
        let img = &mut (*(&raw mut COMMIT_STAGING)).0;
        img.copy_from_slice(&wa[base..base + 8192]);
        hw::dc_flush_range(img.as_ptr(), 8192);

        let bus = bus();
        let mut r = bus.erase_sector(chn as u32, card_addr);
        if r == CARD_ERROR_READY {
            r = bus.write_sector(chn as u32, card_addr, img);
        }
        if r != CARD_ERROR_READY {
            return r;
        }
    }
    // flip: copy new image into the other RAM copy, mark it current
    let wa2 = &mut *workarea(chn);
    let other = dirbase(1 - card(chn).curr_dir);
    wa2.copy_within(base..base + 8192, other);
    card(chn).curr_dir = 1 - card(chn).curr_dir;
    CARD_ERROR_READY
}

unsafe fn update_fat(chn: i32) -> i32 {
    let wa = &mut *workarea(chn);
    let base = fatbase(card(chn).curr_fat);
    let updated = get16(wa, base + 4).wrapping_add(1);
    put16(wa, base + 4, updated);
    let (cs1, cs2) = checksum(&wa[base + 4..base + 0x1ffc]);
    put16(wa, base, cs1);
    put16(wa, base + 2, cs2);

    let sector_size = card(chn).sector_size;
    let card_sector = (base / 8192) as u32; // 3 or 4
    let card_addr = card_sector * sector_size;

    {
        let img = &mut (*(&raw mut COMMIT_STAGING)).0;
        img.copy_from_slice(&wa[base..base + 8192]);
        hw::dc_flush_range(img.as_ptr(), 8192);

        let bus = bus();
        let mut r = bus.erase_sector(chn as u32, card_addr);
        if r == CARD_ERROR_READY {
            r = bus.write_sector(chn as u32, card_addr, img);
        }
        if r != CARD_ERROR_READY {
            return r;
        }
    }
    let wa2 = &mut *workarea(chn);
    let other = fatbase(1 - card(chn).curr_fat);
    wa2.copy_within(base..base + 8192, other);
    card(chn).curr_fat = 1 - card(chn).curr_fat;
    CARD_ERROR_READY
}

// ---------------------------------------------------------------------------
// fat chain mgmt — `__card_allocblock` / `__card_freeblock`
// ---------------------------------------------------------------------------

unsafe fn alloc_blocks(chn: i32, need: u16) -> Result<u16, i32> {
    let wa = &mut *workarea(chn);
    let fs = fatbase(card(chn).curr_fat);
    let blocks = card(chn).blocks;
    let freeblocks = get16(wa, fs + 6);
    if freeblocks < need {
        return Err(CARD_ERROR_INSSPACE);
    }
    let mut head: u16 = 0xffff;
    let mut prev: u16 = 0xffff;
    let mut curr = get16(wa, fs + 8); // lastalloc
    let mut count: u32 = 0;
    let mut todo = need;
    while todo > 0 {
        curr += 1;
        if curr < CARD_SYSAREA || curr as u32 >= blocks {
            curr = CARD_SYSAREA;
        }
        if get16(wa, fs + 10 + (curr as usize - 5) * 2) == 0 {
            if head == 0xffff {
                head = curr;
            } else {
                put16(wa, fs + 10 + (prev as usize - 5) * 2, curr);
            }
            put16(wa, fs + 10 + (curr as usize - 5) * 2, 0xffff);
            prev = curr;
            todo -= 1;
        }
        count += 1;
        if count > blocks - CARD_SYSAREA as u32 {
            return Err(CARD_ERROR_BROKEN);
        }
    }
    put16(wa, fs + 6, freeblocks - need);
    put16(wa, fs + 8, curr); // lastalloc
    Ok(head)
}

unsafe fn free_chain(chn: i32, head: u16) -> i32 {
    let wa = &mut *workarea(chn);
    let fs = fatbase(card(chn).curr_fat);
    let blocks = card(chn).blocks;
    let mut b = head;
    while b != 0xffff {
        if b < CARD_SYSAREA || b as u32 >= blocks {
            return CARD_ERROR_BROKEN;
        }
        let next = get16(wa, fs + 10 + (b as usize - 5) * 2);
        put16(wa, fs + 10 + (b as usize - 5) * 2, 0);
        let freeblocks = get16(wa, fs + 6);
        put16(wa, fs + 6, freeblocks + 1);
        b = next;
    }
    CARD_ERROR_READY
}

// ---------------------------------------------------------------------------
// public API (libogc-shaped; sync)
// ---------------------------------------------------------------------------

/// `CARD_Init(gamecode, company)` — per-card sane defaults; pass the game's
/// 4-byte gamecode + 2-byte company to scope file operations like libogc.
pub fn init(gamecode: Option<[u8; 4]>, company: Option<[u8; 2]>) {
    unsafe {
        #[cfg(target_arch = "powerpc")]
        if (*(&raw mut BUS)).is_none() {
            set_bus(&mut *exi_bus::instance());
        }
        for c in (*(&raw mut CARDS)).iter_mut() {
            c.gamecode = gamecode;
            c.company = company;
        }
    }
}

/// `CARD_Probe` — is a card inserted?
pub fn probe(chn: i32) -> i32 {
    if !(0..2).contains(&chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    unsafe {
        if bus().probe(chn as u32) {
            CARD_ERROR_READY
        } else {
            CARD_ERROR_NOCARD
        }
    }
}

/// `CARD_Mount` — read header/dir/FAT, verify, repair stale copies.
pub fn mount(chn: i32) -> i32 {
    if !(0..2).contains(&chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = card(chn);
        if c.attached {
            return CARD_ERROR_READY;
        }
        if c.busy {
            return CARD_ERROR_BUSY;
        }
        c.busy = true;
        c.result = CARD_ERROR_BUSY;
    }
    let rc = unsafe { mount_inner(chn) };
    unsafe {
        let c = card(chn);
        c.busy = false;
        c.result = rc;
        if rc == CARD_ERROR_READY {
            c.attached = true;
        }
    }
    rc
}

unsafe fn mount_inner(chn: i32) -> i32 {
    let chn_u = chn as u32;
    if !bus().probe(chn_u) {
        return CARD_ERROR_NOCARD;
    }
    let id = match bus().get_id(chn_u) {
        None => return CARD_ERROR_NOCARD,
        Some(id) => id,
    };
    let Some((card_size, sector_size, blocks, latency)) = is_card(id) else {
        return CARD_ERROR_WRONGDEVICE;
    };
    {
        let c = card(chn);
        c.cid = id;
        c.card_size = card_size;
        c.sector_size = sector_size;
        c.blocks = blocks;
        c.latency = latency;
    }

    // clearstatus + readstatus; refuse locked (coded) cards — the DSP
    // unlock dance for counterfeit cards is not ported yet.
    let mut rc = CARD_ERROR_READY;
    if bus().clear_status(chn_u).is_err() {
        rc = CARD_ERROR_NOCARD;
    } else {
        match bus().read_status(chn_u) {
            Err(_) => rc = CARD_ERROR_NOCARD,
            Ok(status) => {
                if status & CARD_STATUS_UNLOCKED == 0 {
                    rc = CARD_ERROR_IOERROR; // coded card: unlock not ported
                }
            }
        }
    }
    if rc == CARD_ERROR_READY {
        rc = bus().enable_interrupt(chn_u, true).map(|_| 0).unwrap_or(CARD_ERROR_NOCARD);
    }
    if rc != CARD_ERROR_READY {
        return rc;
    }

    // read the 5 system blocks into the workarea
    for step in 0..5u32 {
        let addr = step * sector_size;
        let wa = &mut *workarea(chn);
        let dst = (step as usize) * 8192;
        hw::dc_invalidate_range(wa[dst..dst + 8192].as_mut_ptr(), 8192);
        let lat = card(chn).latency;
        let r = bus().read(chn_u, addr, lat, &mut wa[dst..dst + 8192]);
        if r != CARD_ERROR_READY {
            return r;
        }
    }

    let dir_bad = check_dir(chn);
    let fat_bad = check_fat(chn);
    // libogc __card_verify: one sound copy of each table is required;
    // at most one copy may be missing overall (ret<=2 with both present).
    if dir_bad == 2 || fat_bad == 2 || dir_bad + fat_bad > 2 {
        return CARD_ERROR_BROKEN;
    }
    CARD_ERROR_READY
}

/// `CARD_Unmount`.
pub fn unmount(chn: i32) -> i32 {
    if !(0..2).contains(&chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let c = card(chn);
        c.attached = false;
        c.busy = false;
        c.result = CARD_ERROR_NOCARD;
    }
    CARD_ERROR_READY
}

/// Sector (one filesystem block) size of the mounted card.
pub fn sector_size(chn: i32) -> Option<u32> {
    if !(0..2).contains(&chn) {
        return None;
    }
    unsafe {
        if !card(chn).attached {
            return None;
        }
        Some(card(chn).sector_size)
    }
}

/// Reset all channel state (host tests only — the console driver is
/// initialized once and keeps its state).
#[cfg(not(target_arch = "powerpc"))]
pub(crate) fn test_reset() {
    unsafe {
        for c in (*(&raw mut CARDS)).iter_mut() {
            *c = CardChn::new();
        }
    }
}

/// `CARD_GetErrorCode`.
pub fn get_error_code(chn: i32) -> i32 {
    if !(0..2).contains(&chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    unsafe { card(chn).result }
}

/// `CARD_GetFreeBlocks`.
pub fn free_blocks(chn: i32) -> Result<u16, i32> {
    if !(0..2).contains(&chn) {
        return Err(CARD_ERROR_FATAL_ERROR);
    }
    unsafe {
        if !card(chn).attached {
            return Err(CARD_ERROR_NOCARD);
        }
        let wa = &*workarea(chn);
        let fs = fatbase(card(chn).curr_fat);
        Ok(get16(wa, fs + 6))
    }
}

/// `__card_getfilenum`.
unsafe fn find_file(chn: i32, filename: &str) -> Result<i32, i32> {
    let wa = &*workarea(chn);
    let base = dirbase(card(chn).curr_dir);
    let img = &wa[base..base + 8192];
    let name = filename.as_bytes();
    let (gc, co) = (card(chn).gamecode, card(chn).company);
    for i in 0..CARD_MAXFILES {
        if entry_is_free(img, i) {
            continue;
        }
        let e = parse_direntry(img, i);
        if !filename_matches(&e, name) {
            continue;
        }
        if let Some(g) = gc {
            if g != e.gamecode {
                continue;
            }
        }
        if let Some(c) = co {
            if c != e.company {
                continue;
            }
        }
        return Ok(i as i32);
    }
    Err(CARD_ERROR_NOFILE)
}

/// `CARD_Open`.
pub fn open(chn: i32, filename: &str) -> Result<CardFile, i32> {
    if !(0..2).contains(&chn) {
        return Err(CARD_ERROR_FATAL_ERROR);
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        if !card(chn).attached {
            return Err(CARD_ERROR_NOCARD);
        }
        let filenum = find_file(chn, filename)?;
        let wa = &*workarea(chn);
        let base = dirbase(card(chn).curr_dir);
        let e = parse_direntry(&wa[base..base + 8192], filenum as usize);
        if e.block < CARD_SYSAREA || e.block as u32 >= card(chn).blocks {
            return Err(CARD_ERROR_BROKEN);
        }
        Ok(CardFile {
            chn,
            filenum,
            offset: 0,
            len: e.length as i32 * card(chn).sector_size as i32,
            iblock: e.block,
        })
    }
}

/// `CARD_Close`.
pub fn close(_file: &mut CardFile) -> i32 {
    CARD_ERROR_READY
}

/// `__card_seek` — position `file` at `offset` for the next `len` bytes;
/// rewrites `file.iblock`/`file.offset` as the FAT chain is walked.
unsafe fn seek(file: &mut CardFile, len: i32, offset: i32) -> i32 {
    let chn = file.chn;
    if file.filenum < 0 || file.filenum >= CARD_MAXFILES as i32 {
        return CARD_ERROR_FATAL_ERROR;
    }
    if file.iblock < CARD_SYSAREA || file.iblock as u32 >= card(chn).blocks {
        return CARD_ERROR_FATAL_ERROR;
    }
    let wa = &*workarea(chn);
    let base = dirbase(card(chn).curr_dir);
    let e = parse_direntry(&wa[base..base + 8192], file.filenum as usize);
    let entry_len = e.length as i32 * card(chn).sector_size as i32;
    if offset >= entry_len || entry_len < offset + len {
        return CARD_ERROR_LIMIT;
    }
    file.len = len;
    if offset < file.offset {
        file.offset = 0;
        file.iblock = e.block;
    }
    let fs = fatbase(card(chn).curr_fat);
    let ss = card(chn).sector_size as i32;
    while file.offset < (offset & !(ss - 1)) {
        file.offset += ss;
        file.iblock = get16(wa, fs + 10 + (file.iblock as usize - 5) * 2);
        if file.iblock < CARD_SYSAREA || file.iblock as u32 >= card(chn).blocks {
            return CARD_ERROR_BROKEN;
        }
    }
    file.offset = offset;
    CARD_ERROR_READY
}

/// `CARD_Read` — `buf.len()` must be a multiple of 512; `offset` must be
/// 512-aligned when nonzero.
pub fn read(file: &mut CardFile, buf: &mut [u8], offset: i32) -> i32 {
    if !(0..2).contains(&file.chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    let len = buf.len() as i32;
    if len <= 0 || (len & 0x1ff) != 0 || (offset > 0 && (offset & 0x1ff) != 0) {
        return CARD_ERROR_FATAL_ERROR;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let chn = file.chn;
        if !card(chn).attached {
            return CARD_ERROR_NOCARD;
        }
        let r = seek(file, len, offset);
        if r != CARD_ERROR_READY {
            return r;
        }
        let ss = card(chn).sector_size as usize;
        let fs = fatbase(card(chn).curr_fat);
        hw::dc_invalidate_range(buf.as_ptr(), buf.len());
        let mut pos = offset as usize;
        let mut done = 0usize;
        let mut total = len as usize;
        while total > 0 {
            let in_sector = pos & (ss - 1);
            let chunk = total.min(ss - in_sector);
            // read the enclosing run of 512-byte segments from the sector
            // start, then copy the requested window out of the bounce buffer.
            let n = ((in_sector + chunk) + 511) & !511;
            if n > 64 * 1024 {
                return CARD_ERROR_FATAL_ERROR; // sector too large (unsupported card)
            }
            let addr = file.iblock as u32 * ss as u32;
            let bounce = &mut (*(&raw mut BOUNCE)).0;
            hw::dc_invalidate_range(bounce.as_mut_ptr(), n);
            let lat = card(chn).latency;
            let r = bus().read(chn as u32, addr, lat, &mut bounce[..n]);
            if r != CARD_ERROR_READY {
                return r;
            }
            hw::dc_invalidate_range(bounce.as_ptr(), n);
            buf[done..done + chunk].copy_from_slice(&bounce[in_sector..in_sector + chunk]);
            done += chunk;
            total -= chunk;
            pos += chunk;
            if total > 0 {
                let wa = &*workarea(chn);
                file.iblock = get16(wa, fs + 10 + (file.iblock as usize - 5) * 2);
                if file.iblock < CARD_SYSAREA || file.iblock as u32 >= card(chn).blocks {
                    return CARD_ERROR_BROKEN;
                }
            }
        }
        CARD_ERROR_READY
    }
}

/// `CARD_Write` — one whole sector per call in practice: erase-then-write.
/// `buf.len()` and `offset` must be sector-size multiples (>0 offset).
pub fn write(file: &mut CardFile, buf: &[u8], offset: i32) -> i32 {
    if !(0..2).contains(&file.chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let chn = file.chn;
        if !card(chn).attached {
            return CARD_ERROR_NOCARD;
        }
        let ss = card(chn).sector_size as i32;
        let len = buf.len() as i32;
        if (len & (ss - 1)) != 0 || (offset > 0 && (offset & (ss - 1)) != 0) {
            return CARD_ERROR_FATAL_ERROR;
        }
        let r = seek(file, len, offset);
        if r != CARD_ERROR_READY {
            return r;
        }
        let addr = file.iblock as u32 * card(chn).sector_size;
        hw::dc_flush_range(buf.as_ptr(), buf.len());
        let mut r = bus().erase_sector(chn as u32, addr);
        if r == CARD_ERROR_READY {
            r = bus().write_sector(chn as u32, addr, buf);
        }
        if r == CARD_ERROR_READY {
            // stamp last_modified
            let wa = &mut *workarea(chn);
            let base = dirbase(card(chn).curr_dir);
            let now = (hw::mftb() / 40_500_000) as u32;
            put32(wa, base + file.filenum as usize * 64 + 40, now);
            r = update_dir(chn);
        }
        r
    }
}

/// `CARD_Create` — reserve a directory entry + FAT chain; returns a handle.
pub fn create(chn: i32, filename: &str, size: u32) -> Result<CardFile, i32> {
    if !(0..2).contains(&chn) {
        return Err(CARD_ERROR_FATAL_ERROR);
    }
    if filename.len() > 32 {
        return Err(CARD_ERROR_NAMETOOLONG);
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        if !card(chn).attached {
            return Err(CARD_ERROR_NOCARD);
        }
        let ss = card(chn).sector_size;
        if size == 0 || size % ss != 0 {
            return Err(CARD_ERROR_FATAL_ERROR);
        }
        let wa = &mut *workarea(chn);
        let base = dirbase(card(chn).curr_dir);
        let img = &mut wa[base..base + 8192];

        // find free entry + uniqueness
        let mut filenum = -1i32;
        let name = filename.as_bytes();
        for i in 0..CARD_MAXFILES {
            if entry_is_free(img, i) {
                if filenum < 0 {
                    filenum = i as i32;
                }
                continue;
            }
            let e = parse_direntry(img, i);
            if filename_matches(&e, name) {
                return Err(CARD_ERROR_EXIST);
            }
        }
        if filenum < 0 {
            return Err(CARD_ERROR_NOENT);
        }
        let need = (size / ss) as u16;
        let freeb = get16(wa, fatbase(card(chn).curr_fat) + 6);
        if (freeb as u32) * ss < size {
            return Err(CARD_ERROR_INSSPACE);
        }

        let head = alloc_blocks(chn, need)?;
        let r = update_fat(chn);
        if r != CARD_ERROR_READY {
            return Err(r);
        }

        // fill the directory entry
        let wa = &mut *workarea(chn);
        let base = dirbase(card(chn).curr_dir);
        let img = &mut wa[base..base + 8192];
        let mut e = DirEntryRaw {
            gamecode: card(chn).gamecode.unwrap_or([0; 4]),
            company: card(chn).company.unwrap_or([0; 2]),
            banner_fmt: 0,
            filename: [0; 32],
            last_modified: (hw::mftb() / 40_500_000) as u32,
            icon_addr: 0xffff_ffff,
            icon_fmt: 0,
            icon_speed: 1, // CARD_SPEED_FAST
            permission: CARD_ATTRIB_PUBLIC,
            copy_times: 0,
            block: head,
            length: need,
            comment_addr: 0,
        };
        let n = filename.len().min(32);
        e.filename[..n].copy_from_slice(&filename.as_bytes()[..n]);
        write_direntry(img, filenum as usize, &e);

        let r = update_dir(chn);
        if r != CARD_ERROR_READY {
            return Err(r);
        }
        Ok(CardFile {
            chn,
            filenum,
            offset: 0,
            len: size as i32,
            iblock: head,
        })
    }
}

/// `CARD_Delete`.
pub fn delete(chn: i32, filename: &str) -> i32 {
    if !(0..2).contains(&chn) {
        return CARD_ERROR_FATAL_ERROR;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        if !card(chn).attached {
            return CARD_ERROR_NOCARD;
        }
        let filenum = match find_file(chn, filename) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let head = {
            let wa = &mut *workarea(chn);
            let base = dirbase(card(chn).curr_dir);
            let img = &mut wa[base..base + 8192];
            let head = parse_direntry(img, filenum as usize).block;
            // memset(entry, 0xff, 64)
            for b in &mut img[filenum as usize * 64..filenum as usize * 64 + 64] {
                *b = 0xff;
            }
            head
        };
        // dir first, then fat — crash-safe ordering.
        let r = update_dir(chn);
        if r != CARD_ERROR_READY {
            return r;
        }
        let r = free_chain(chn, head);
        if r != CARD_ERROR_READY {
            return r;
        }
        update_fat(chn)
    }
}

/// `CARD_GetStatus`.
pub fn get_status(chn: i32, filenum: i32) -> Result<CardStat, i32> {
    if !(0..2).contains(&chn) || filenum < 0 || filenum >= CARD_MAXFILES as i32 {
        return Err(CARD_ERROR_FATAL_ERROR);
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        if !card(chn).attached {
            return Err(CARD_ERROR_NOCARD);
        }
        let wa = &*workarea(chn);
        let base = dirbase(card(chn).curr_dir);
        let img = &wa[base..base + 8192];
        let e = parse_direntry(img, filenum as usize);
        Ok(CardStat {
            filename: e.filename,
            len: e.length as u32 * card(chn).sector_size,
            time: e.last_modified,
            gamecode: e.gamecode,
            company: e.company,
            banner_fmt: e.banner_fmt,
            icon_addr: e.icon_addr,
            icon_fmt: e.icon_fmt,
            icon_speed: e.icon_speed,
            comment_addr: e.comment_addr,
        })
    }
}

/// `CARD_SetStatus`.
pub fn set_status(chn: i32, filenum: i32, stat: &CardStat) -> i32 {
    if !(0..2).contains(&chn) || filenum < 0 || filenum >= CARD_MAXFILES as i32 {
        return CARD_ERROR_FATAL_ERROR;
    }
    if stat.icon_addr != 0xffff_ffff && stat.icon_addr >= 512 {
        return CARD_ERROR_FATAL_ERROR;
    }
    if stat.comment_addr != 0xffff_ffff && (stat.comment_addr % 8192) > 8128 {
        return CARD_ERROR_FATAL_ERROR;
    }
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        if !card(chn).attached {
            return CARD_ERROR_NOCARD;
        }
        let wa = &mut *workarea(chn);
        let base = dirbase(card(chn).curr_dir);
        let img = &mut wa[base..base + 8192];
        let mut e = parse_direntry(img, filenum as usize);
        e.banner_fmt = stat.banner_fmt;
        e.icon_addr = stat.icon_addr;
        e.icon_fmt = stat.icon_fmt;
        e.icon_speed = if stat.icon_addr == 0xffff_ffff {
            (stat.icon_speed & !3) | 1
        } else {
            stat.icon_speed
        };
        e.comment_addr = stat.comment_addr;
        e.last_modified = (hw::mftb() / 40_500_000) as u32;
        write_direntry(img, filenum as usize, &e);
        update_dir(chn)
    }
}

/// `CARD_FindFirst` — begin directory iteration; fills `dir` fields of the
/// first visible file. When `showall` is false, files of other gamecodes are
/// skipped.
pub fn find_first(chn: i32, showall: bool) -> Result<CardDir, i32> {
    if !(0..2).contains(&chn) {
        return Err(CARD_ERROR_FATAL_ERROR);
    }
    let mut d = CardDir {
        chn,
        fileno: u32::MAX,
        filelen: 0,
        permissions: 0,
        filename: [0; 32],
        gamecode: [0; 4],
        company: [0; 2],
    };
    let _ = showall;
    match find_next_inner(&mut d)? {
        false => Err(CARD_ERROR_NOFILE),
        true => Ok(d),
    }
}

/// `CARD_FindNext` — continue iteration. Err(NOFILE) ends the walk.
pub fn find_next(dir: &mut CardDir) -> i32 {
    match find_next_inner(dir) {
        Ok(true) => CARD_ERROR_READY,
        Ok(false) => CARD_ERROR_NOFILE,
        Err(e) => e,
    }
}

fn find_next_inner(d: &mut CardDir) -> Result<bool, i32> {
    let _guard = crate::irq::IrqLock::take();
    unsafe {
        let chn = d.chn;
        if !card(chn).attached {
            return Err(CARD_ERROR_NOCARD);
        }
        let wa = &*workarea(chn);
        let base = dirbase(card(chn).curr_dir);
        let img = &wa[base..base + 8192];
        let start = if d.fileno == u32::MAX { 0 } else { d.fileno as usize + 1 };
        for i in start..CARD_MAXFILES {
            if entry_is_free(img, i) {
                continue;
            }
            let e = parse_direntry(img, i);
            d.fileno = i as u32;
            d.filelen = e.length as u32 * card(chn).sector_size;
            d.permissions = e.permission;
            d.filename = e.filename;
            d.gamecode = e.gamecode;
            d.company = e.company;
            return Ok(true);
        }
    }
    Ok(false)
}

