//! ELF32 → GameCube `.dol` packing library.
//!
//! The `.dol` header (0x100 bytes):
//!
//! ```text
//! off   bytes  field
//! 0x00  28     textOffsets[7]
//! 0x1C  44     dataOffsets[11]
//! 0x48  28     textAddresses[7]
//! 0x64  44     dataAddresses[11]
//! 0x90  28     textSizes[7]
//! 0xAC  44     dataSizes[11]
//! 0xD8   4     bssAddress
//! 0xDC   4     bssSize
//! 0xE0   4     entryPoint
//! 0xE4  28     padding
//! 0x100  .     section payloads: all texts, then all datas
//! ```

#[derive(Debug, Clone)]
pub struct DolSection {
    pub file_offset: u32,
    pub addr: u32,
    pub size: u32,
}

#[derive(Debug)]
pub struct DolHeader {
    pub text: Vec<DolSection>,
    pub data: Vec<DolSection>,
    pub bss_addr: u32,
    pub bss_size: u32,
    pub entry: u32,
}

pub const MEM1_START: u32 = 0x8000_0000;
pub const MEM1_END: u32 = 0x8180_0000;
pub const DOL_ENTRY: u32 = 0x8000_3100;
pub const MAX_TEXT: usize = 7;
pub const MAX_DATA: usize = 11;

/// Parse a `.dol` header (without payload) for verification.
pub fn parse_dol_header(buf: &[u8]) -> Result<DolHeader, String> {
    if buf.len() < 0x100 {
        return Err("too small for a .dol header".into());
    }
    let be32 = |off: usize| -> u32 {
        u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    };
    let mut text = Vec::new();
    let mut data = Vec::new();
    for i in 0..7 {
        let o = be32(i * 4);
        let a = be32(0x48 + i * 4);
        let s = be32(0x90 + i * 4);
        if s != 0 {
            text.push(DolSection { file_offset: o, addr: a, size: s });
        }
    }
    for i in 0..11 {
        let o = be32(0x1C + i * 4);
        let a = be32(0x64 + i * 4);
        let s = be32(0xAC + i * 4);
        if s != 0 {
            data.push(DolSection { file_offset: o, addr: a, size: s });
        }
    }
    Ok(DolHeader {
        text,
        data,
        bss_addr: be32(0xD8),
        bss_size: be32(0xDC),
        entry: be32(0xE0),
    })
}

/// Verify a .dol image against hard requirements.
pub fn validate_dol(buf: &[u8]) -> Vec<String> {
    let mut errs = Vec::new();
    let hdr = match parse_dol_header(buf) {
        Ok(h) => h,
        Err(e) => return vec![e],
    };
    if hdr.entry != DOL_ENTRY {
        errs.push(format!("entry {:#x} != {:#x}", hdr.entry, DOL_ENTRY));
    }
    if hdr.text.is_empty() {
        errs.push("no text sections".into());
    }
    let all = hdr.text.iter().chain(hdr.data.iter());
    for s in all {
        if !(MEM1_START..MEM1_END).contains(&s.addr) {
            errs.push(format!("section at {:#010x} outside MEM1", s.addr));
        }
        if !(MEM1_START..MEM1_END).contains(&(s.addr + s.size.saturating_sub(1))) {
            errs.push(format!(
                "section at {:#010x} size {:#x} overruns MEM1",
                s.addr, s.size
            ));
        }
        if (s.file_offset as usize + s.size as usize) > buf.len() {
            errs.push(format!(
                "section at {:#010x}: file offset {:#x} + size {:#x} runs past file end",
                s.addr, s.file_offset, s.size
            ));
        }
    }
    if hdr.bss_size > 0 && !(MEM1_START..MEM1_END).contains(&hdr.bss_addr) {
        errs.push(format!("bss at {:#010x} outside MEM1", hdr.bss_addr));
    }
    errs
}

// ---------------------------------------------------------------------------
// ELF32-BE parsing + packing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Section {
    name: String,
    addr: u32,
    offset: u32,
    size: u32,
    flags: u32,
    sh_type: u32,
}

pub struct PackedDol {
    pub header: DolHeader,
    pub image: Vec<u8>,
}

fn be32_at(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn parse_elf(raw: &[u8]) -> Result<(Vec<Section>, u32), String> {
    if raw.len() < 0x34 || &raw[0..4] != b"\x7fELF" {
        return Err("not an ELF".into());
    }
    if raw[4] != 1 || raw[5] != 2 {
        return Err("need ELF32 big-endian".into());
    }
    if u16::from_be_bytes([raw[18], raw[19]]) != 20 {
        return Err("need EM_PPC (machine 20)".into());
    }
    let entry = be32_at(raw, 0x18);
    let shoff = be32_at(raw, 0x20) as usize;
    let shentsize = u16::from_be_bytes([raw[0x2E], raw[0x2F]]) as usize;
    let shnum = u16::from_be_bytes([raw[0x30], raw[0x31]]) as usize;
    let shstrndx = u16::from_be_bytes([raw[0x32], raw[0x33]]) as usize;
    if shoff == 0 || shnum == 0 {
        return Err("no sections".into());
    }
    let shstr_off = {
        let base = shoff + shstrndx * shentsize;
        be32_at(raw, base + 0x10) as usize
    };

    let mut sections = Vec::new();
    for i in 0..shnum {
        let base = shoff + i * shentsize;
        if base + 0x28 > raw.len() {
            return Err("section header out of range".into());
        }
        let name_off = be32_at(raw, base) as usize;
        let sh_type = be32_at(raw, base + 4);
        let flags = be32_at(raw, base + 8);
        let addr = be32_at(raw, base + 12);
        let offset = be32_at(raw, base + 16);
        let size = be32_at(raw, base + 20);
        if addr == 0 || size == 0 {
            continue;
        }
        let name = {
            let mut end = shstr_off + name_off;
            while end < raw.len() && raw[end] != 0 {
                end += 1;
            }
            String::from_utf8_lossy(&raw[shstr_off + name_off..end]).into_owned()
        };
        sections.push(Section { name, addr, offset, size, flags, sh_type });
    }
    Ok((sections, entry))
}

/// Pack a `.dol` image from a raw ELF32 big-endian PowerPC buffer.
pub fn pack_elf(raw: &[u8]) -> Result<PackedDol, String> {
    const SHF_ALLOC: u32 = 0x2;
    const SHF_EXECINSTR: u32 = 0x4;
    const SHT_NOBITS: u32 = 8;

    let (mut sections, entry) = parse_elf(raw)?;
    sections.sort_by_key(|s| s.addr);

    let mut texts: Vec<&Section> = Vec::new();
    let mut datas: Vec<&Section> = Vec::new();
    let mut bss_addr = 0u32;
    let mut bss_size = 0u32;

    for s in &sections {
        if s.flags & SHF_ALLOC == 0 {
            continue;
        }
        if s.sh_type == SHT_NOBITS {
            if bss_addr == 0 {
                bss_addr = s.addr;
            }
            bss_size = (s.addr + s.size).saturating_sub(bss_addr);
            continue;
        }
        if s.flags & SHF_EXECINSTR != 0 {
            texts.push(s);
        } else {
            datas.push(s);
        }
    }
    if texts.len() > MAX_TEXT {
        return Err(format!("too many text sections ({})", texts.len()));
    }
    if datas.len() > MAX_DATA {
        return Err(format!("too many data sections ({})", datas.len()));
    }

    let mut out = vec![0u8; 0x100];
    let mut w32 = |off: usize, v: u32| {
        out[off..off + 4].copy_from_slice(&v.to_be_bytes());
    };

    let mut cur_off = 0x100u32;
    let mut text_secs: Vec<DolSection> = Vec::new();
    let mut data_secs: Vec<DolSection> = Vec::new();

    for s in &texts {
        text_secs.push(DolSection { file_offset: cur_off, addr: s.addr, size: s.size });
        cur_off += s.size;
    }
    for s in &datas {
        data_secs.push(DolSection { file_offset: cur_off, addr: s.addr, size: s.size });
        cur_off += s.size;
    }

    for i in 0..MAX_TEXT {
        let def = DolSection { file_offset: 0, addr: 0, size: 0 };
        let s = text_secs.get(i).unwrap_or(&def);
        w32(0x00 + i * 4, s.file_offset);
        w32(0x48 + i * 4, s.addr);
        w32(0x90 + i * 4, s.size);
    }
    for i in 0..MAX_DATA {
        let def = DolSection { file_offset: 0, addr: 0, size: 0 };
        let s = data_secs.get(i).unwrap_or(&def);
        w32(0x1C + i * 4, s.file_offset);
        w32(0x64 + i * 4, s.addr);
        w32(0xAC + i * 4, s.size);
    }
    w32(0xD8, bss_addr);
    w32(0xDC, bss_size);
    w32(0xE0, entry);

    for s in texts.iter().chain(datas.iter()) {
        out.extend_from_slice(&raw[s.offset as usize..(s.offset + s.size) as usize]);
    }

    Ok(PackedDol {
        header: DolHeader {
            text: text_secs,
            data: data_secs,
            bss_addr,
            bss_size,
            entry,
        },
        image: out,
    })
}
