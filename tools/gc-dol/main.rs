//! `gc-dol`: convert a big-endian 32-bit PowerPC ELF executable into a
//! GameCube `.dol` image.
//!
//! `.dol` format (as produced by devkitPro's `elf2dol`):
//!
//! ```text
//! offset  size  field
//!   0x00   28   u32 textOffsets[7]
//!   0x1C   44   u32 dataOffsets[11]
//!   0x48   28   u32 textAddresses[7]
//!   0x64   44   u32 dataAddresses[11]
//!   0x90   28   u32 textSizes[7]
//!   0xAC   44   u32 dataSizes[11]
//!   0xD8    4   u32 bssAddress
//!   0xDC    4   u32 bssSize
//!   0xE0    4   u32 entryPoint
//!   0xE4   28   padding
//!   0x100   .   section payloads (text first, then data)
//! ```
//!
//! Text = SHF_ALLOC|SHF_EXECINSTR, data = SHF_ALLOC|!exec (has file bytes),
//! bss = NOBITS part of the allocated range.

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::process::ExitCode;

#[derive(Debug)]
struct Section {
    name: String,
    addr: u32,
    offset: u32,
    size: u32,
    flags: u32,
    sh_type: u32,
}

#[derive(Debug)]
struct Elf32 {
    entry: u32,
    sections: Vec<Section>,
    data: Vec<u8>,
}

fn u16be(b: &[u8], off: usize) -> u32 {
    u16::from_be_bytes([b[off], b[off + 1]]) as u32
}
fn u32be(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn parse_elf(data: Vec<u8>) -> Result<Elf32, String> {
    if data.len() < 0x34 || &data[0..4] != b"\x7fELF" {
        return Err("not an ELF".into());
    }
    if data[4] != 1 || data[5] != 2 {
        return Err("need ELF32 big-endian".into());
    }
    let entry = u32be(&data, 0x18);
    let shoff = u32be(&data, 0x20) as usize;
    let shentsize = u16be(&data, 0x2E) as usize;
    let shnum = u16be(&data, 0x30) as usize;
    let shstrndx = u16be(&data, 0x32) as usize;

    if shoff == 0 || shnum == 0 {
        return Err("no sections".into());
    }

    // section header strings
    let shstr_off = {
        let base = shoff + shstrndx * shentsize;
        u32be(&data, base + 0x10) as usize
    };

    let mut sections = Vec::new();
    for i in 0..shnum {
        let base = shoff + i * shentsize;
        if base + 0x28 > data.len() {
            return Err("section header out of range".into());
        }
        let name_off = u32be(&data, base) as usize;
        let sh_type = u32be(&data, base + 4);
        let flags = u32be(&data, base + 8);
        let addr = u32be(&data, base + 12);
        let offset = u32be(&data, base + 16);
        let size = u32be(&data, base + 20);
        if addr == 0 || size == 0 {
            continue;
        }
        let name = {
            let mut end = shstr_off + name_off;
            while end < data.len() && data[end] != 0 {
                end += 1;
            }
            String::from_utf8_lossy(&data[shstr_off + name_off..end]).into_owned()
        };
        sections.push(Section { name, addr, offset, size, flags, sh_type });
    }
    Ok(Elf32 { entry, sections, data })
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let elf_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: gc-dol <in.elf> <out.dol>");
            return ExitCode::FAILURE;
        }
    };
    let dol_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: gc-dol <in.elf> <out.dol>");
            return ExitCode::FAILURE;
        }
    };

    let raw = match fs::read(&elf_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gc-dol: open {elf_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let elf = match parse_elf(raw) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("gc-dol: parse {elf_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    const SHF_ALLOC: u32 = 0x2;
    const SHF_EXECINSTR: u32 = 0x4;
    const SHT_NOBITS: u32 = 8;

    let mut texts: Vec<&Section> = Vec::new();
    let mut datas: Vec<&Section> = Vec::new();
    let mut bss_addr = 0u32;
    let mut bss_size = 0u32;

    let mut sections: Vec<&Section> = elf.sections.iter().collect();
    sections.sort_by_key(|s| s.addr);

    for s in &sections {
        if s.flags & SHF_ALLOC == 0 {
            continue;
        }
        if s.sh_type == SHT_NOBITS {
            if bss_addr == 0 {
                bss_addr = s.addr;
            }
            bss_size = (s.addr + s.size) - bss_addr;
            continue;
        }
        if s.flags & SHF_EXECINSTR != 0 {
            texts.push(s);
        } else {
            datas.push(s);
        }
    }
    if texts.len() > 7 {
        eprintln!("gc-dol: too many text sections ({})", texts.len());
        return ExitCode::FAILURE;
    }
    if datas.len() > 11 {
        eprintln!("gc-dol: too many data sections ({})", datas.len());
        return ExitCode::FAILURE;
    }

    // Header is 0x100 bytes. Field layout per devkitPro elf2dol (dolmain.c):
    //   0x00 textOffsets[7]    (28 bytes -> 0x1C)
    //   0x1C dataOffsets[11]   (44 -> 0x48)
    //   0x48 textAddresses[7]  (28 -> 0x64)
    //   0x64 dataAddresses[11] (44 -> 0x90)
    //   0x90 textSizes[7]      (28 -> 0xAC)
    //   0xAC dataSizes[11]     (44 -> 0xD8)
    //   0xD8 bss, 0xDC bssSize, 0xE0 entry, 0xE4..0xFF padding
    let mut out = vec![0u8; 0x100];
    let write32 = |buf: &mut Vec<u8>, off: usize, v: u32| {
        buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
    };

    let mut text_offs = [0u32; 7];
    let mut text_addr = [0u32; 7];
    let mut text_size = [0u32; 7];
    let mut data_offs = [0u32; 11];
    let mut data_addr = [0u32; 11];
    let mut data_size = [0u32; 11];

    let mut cur_off = 0x100u32;
    for (i, s) in texts.iter().enumerate() {
        text_offs[i] = cur_off;
        text_addr[i] = s.addr;
        text_size[i] = s.size;
        cur_off += s.size;
    }
    for (i, s) in datas.iter().enumerate() {
        data_offs[i] = cur_off;
        data_addr[i] = s.addr;
        data_size[i] = s.size;
        cur_off += s.size;
    }

    for i in 0..7 {
        write32(&mut out, 0x00 + i * 4, text_offs[i]);
        write32(&mut out, 0x48 + i * 4, text_addr[i]);
        write32(&mut out, 0x90 + i * 4, text_size[i]);
    }
    for i in 0..11 {
        write32(&mut out, 0x1C + i * 4, data_offs[i]);
        write32(&mut out, 0x64 + i * 4, data_addr[i]);
        write32(&mut out, 0xAC + i * 4, data_size[i]);
    }
    write32(&mut out, 0xD8, bss_addr);
    write32(&mut out, 0xDC, bss_size);
    write32(&mut out, 0xE0, elf.entry);

    for s in texts.iter().chain(datas.iter()) {
        let src = &elf.data[s.offset as usize..(s.offset + s.size) as usize];
        out.extend_from_slice(src);
    }

    let mut f = match File::create(&dol_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gc-dol: create {dol_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&out).and_then(|_| f.flush()) {
        eprintln!("gc-dol: write {dol_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!("gc-dol: {} -> {} ({} bytes)", elf_path, dol_path, out.len());
    ExitCode::SUCCESS
}
