//! gc-iso: produce a **bootable GameCube ISO (GCM)** from a gc-std DOL,
//! the Rust apploader payload, and any number of extra payload files.
//!
//! Disc layout produced (byte offsets):
//!
//! ```text
//! 0x00000  boot.bin (0x440 B) — gamecode/name/magic
//! 0x00440  bi2.bin  (0x2000 B) — FST offset/size pointers
//! 0x02440  apploader: 0x20-B header {entry,size,trailer} + payload
//! pad →    FST (files: "main.dol" + extras)
//! pad →    main.dol  (the packed DOL binary)
//! pad →    extras (readme.txt etc., 2048-aligned)
//! ```
//!
//! The apploader's section table (inside its payload) gets patched in place
//! so it walks `main.dol`'s sections, Dolphin HLE-loads them and hands over
//! control at the DOL entrypoint. The resulting ISO boots in Dolphin
//! (batch mode) and on real hardware via Swiss.
//!
//! usage:
//!   gc-iso --apploader <apploader.elf> --dol <app.dol> \
//!          [--file  LBA_OFFSET:PATH]... [-o out.iso] --name TITLE
//!
//! extras land at the given absolute disc offsets (2048-aligned) and appear
//! in the FST as their basename.

use std::fs::{self, File};
use std::io::Write;
use std::process::ExitCode;

// ---- minimal ELF32-BE reader ------------------------------------------------

const PP_LO: u32 = 0x8120_0000; // apploader payload load address

struct ElfSection {
    name: String,
    addr: u32,
    off: u32,
    size: u32,
    bss: bool,
}

fn parse_elf_alloc_sections(elf: &[u8]) -> Result<(u32, Vec<ElfSection>), String> {
    if elf.len() < 0x34 || &elf[0..4] != b"\x7fELF" {
        return Err("not an ELF".into());
    }
    let be32 = |o: usize| u32::from_be_bytes([elf[o], elf[o + 1], elf[o + 2], elf[o + 3]]);
    let be16 = |o: usize| u16::from_be_bytes([elf[o], elf[o + 1]]);
    let entry = be32(0x18);
    let shoff = be32(0x20) as usize;
    let shentsize = be16(0x2e) as usize;
    let shnum = be16(0x30) as usize;
    let shstrndx = be16(0x32) as usize;
    if shoff == 0 || shnum == 0 {
        return Err("no sections".into());
    }
    let str_sh = shoff + shstrndx * shentsize;
    let str_off = be32(str_sh + 0x10) as usize;
    let str_size = be32(str_sh + 0x14) as usize;
    let strs = &elf[str_off..str_off + str_size];

    let mut out = Vec::new();
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        let name_off = be32(sh + 0) as usize;
        let flags = be32(sh + 0x08);
        let addr = be32(sh + 0x0c);
        let off = be32(sh + 0x10);
        let size = be32(sh + 0x14);
        let sh_type = be32(sh + 0x04);
        if flags & 2 == 0 || size == 0 {
            continue; // not SHF_ALLOC
        }
        let mut end = name_off;
        while end < strs.len() && strs[end] != 0 {
            end += 1;
        }
        let name = String::from_utf8_lossy(&strs[name_off..end]).into_owned();
        out.push(ElfSection {
            bss: sh_type == 8, // SHT_NOBITS
            name,
            addr,
            off,
            size,
        });
    }
    Ok((entry, out))
}

/// Flatten all allocatable sections into a single binary image starting at
/// PP_LO (any BSS trailing zeros are just appended).
fn flatten_apploader(elf: &[u8]) -> Result<(u32, Vec<u8>), String> {
    let (entry, secs) = parse_elf_alloc_sections(elf)?;
    let mut size = 0usize;
    for s in &secs {
        if s.addr < PP_LO {
            return Err(format!("section {} below payload base", s.name));
        }
        size = size.max((s.addr - PP_LO) as usize + s.size as usize);
    }
    if size == 0 {
        return Err("empty apploader".into());
    }
    let mut blob = vec![0u8; size];
    for s in &secs {
        if !s.bss {
            blob[(s.addr - PP_LO) as usize..(s.addr - PP_LO + s.size) as usize]
                .copy_from_slice(&elf[s.off as usize..(s.off + s.size) as usize]);
        }
    }
    Ok((entry, blob))
}

// ---- ISO assembly ------------------------------------------------------------

fn be32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_be_bytes());
}

const BOOT_BIN_SIZE: usize = 0x440;
const BI2_LBA: usize = 0x440;
const BI2_SIZE: usize = 0x2000;
const APPIMG_LBA: usize = 0x2440;

struct FileSpec {
    fst_name: String,
    data: Vec<u8>,
    lba: u32,
}

fn main() -> ExitCode {
    let mut apploader_elf: Option<String> = None;
    let mut dol_path: Option<String> = None;
    let mut name = "gc-rust".to_string();
    let mut out_path: Option<String> = None;
    let mut extras: Vec<(u32, String)> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--apploader" => apploader_elf = args.next(),
            "--dol" => dol_path = args.next(),
            "--name" => {
                if let Some(n) = args.next() {
                    name = n;
                }
            }
            "--file" => {
                if let Some(spec) = args.next() {
                    let (lba, path) = match spec.split_once(':') {
                        Some(s) => s,
                        None => {
                            eprintln!("gc-iso: --file OFFSET:PATH");
                            return ExitCode::FAILURE;
                        }
                    };
                    let Ok(lba) = u32::from_str_radix(lba.trim_start_matches("0x"), 16) else {
                        eprintln!("gc-iso: bad offset `{lba}`");
                        return ExitCode::FAILURE;
                    };
                    if lba & 0x7ff != 0 {
                        eprintln!("gc-iso: offset not 2048-aligned");
                        return ExitCode::FAILURE;
                    }
                    extras.push((lba, path.to_string()));
                }
            }
            "-o" | "--out" => out_path = args.next(),
            _ => {
                eprintln!("gc-iso: unknown arg {a}");
                return ExitCode::FAILURE;
            }
        }
    }

    let (Some(apploader_elf), Some(dol_path), Some(out_path)) = (apploader_elf, dol_path, out_path)
    else {
        eprintln!(
            "gc-iso usage: --apploader A.elf --dol A.dol -o out.iso [--name NAME] [--file OFFSET:PATH]..."
        );
        return ExitCode::FAILURE;
    };

    let elf = fs::read(&apploader_elf).expect("apploader elf");
    let dol = fs::read(&dol_path).expect("dol");

    // ---- apploader payload ---------------------------------------------------
    let (app_entry, mut app_blob) = match flatten_apploader(&elf) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gc-iso: apploader: {e}");
            return ExitCode::FAILURE;
        }
    };

    // ---- patch the section table ---------------------------------------------
    let dol_header = gc_dol::parse_dol_header(&dol).expect("dol parse");
    // refer to the DOL's own sections (dol payload offsets are file offsets
    // inside the .dol container; we place the .dol at a fixed LBA)
    let mut sections: Vec<(u32, u32, u32)> = Vec::new(); // (disc_offset, len, mem_addr)
    for s in dol_header.text.iter().chain(dol_header.data.iter()) {
        sections.push((s.file_offset, s.size, s.addr));
    }

    // locate table magic in the payload
    let magic = [0xACu8, 0x1D, 0x50, 0x00];
    let tpos = app_blob
        .windows(4)
        .position(|w| w == magic)
        .expect("apploader payload missing section-table magic");
    let cnt = sections.len();
    if cnt > (96 - 3) / 3 {
        eprintln!("gc-iso: too many dol sections");
        return ExitCode::FAILURE;
    }
    be32(&mut app_blob, tpos + 8, cnt as u32); // the count slot
    // entries patched after we know the DOL's disc LBA (below)

    let app_total = 0x20 + app_blob.len();

    // ---- lay out the image ----------------------------------------------------
    let dol_lba = ((APPIMG_LBA + app_total + 0x700) & !0x7ff) as u32; // align 2048
    let mut cur = dol_lba as usize + dol.len();
    // extras: honor given offsets (absolute)
    let mut files: Vec<FileSpec> = Vec::new();
    for (lba, path) in &extras {
        let data = fs::read(path).expect("extra file");
        if (*lba as usize) < cur {
            eprintln!("gc-iso: extra file offset {lba:#x} overlaps earlier content ({cur:#x})");
            return ExitCode::FAILURE;
        }
        let base = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        files.push(FileSpec { fst_name: base, data, lba: *lba });
    }
    let fst_lba = (cur + 0x7ff) & !0x7ff;

    // patch the actual disc offsets into the apploader's table
    for (i, (_, sz, addr)) in sections.iter().enumerate() {
        let (foff, _, _) = sections[i];
        be32(&mut app_blob, tpos + 12 + i * 12, dol_lba + foff);
        be32(&mut app_blob, tpos + 12 + i * 12 + 4, *sz);
        be32(&mut app_blob, tpos + 12 + i * 12 + 8, *addr);
    }
    let _ = cnt;

    // FST content (each entry is 12 bytes; a string bank follows)
    let mut strbank: Vec<u8> = Vec::new();
    strbank.push(0u8);
    let n_entries = 1 + 1 + files.len(); // root + dol + extras
    let mut fst = vec![0u8; n_entries * 12];
    // root directory entry: flags=1 (dir), name off 0, parent 0,
    // next-n = total entries
    {
        let r = &mut fst[0..12];
        be32(r, 0, 0x0100_0000);
        be32(r, 4, 0);
        be32(r, 8, n_entries as u32);
    }
    let mut ent = 1;
    let mut push_file = |fst: &mut Vec<u8>, strbank: &mut Vec<u8>, ent: usize, nm: &str, lba: u32, size: u32| {
        let s_off = strbank.len() as u32;
        strbank.extend_from_slice(nm.as_bytes());
        strbank.push(0);
        let r = &mut fst[ent * 12..ent * 12 + 12];
        be32(r, 0, s_off); // flags=0 (file), name offset (top byte = 0)
        be32(r, 4, lba);
        be32(r, 8, size);
    };
    push_file(&mut fst, &mut strbank, ent, "main.dol", dol_lba, dol.len() as u32);
    ent += 1;
    for f in &files {
        push_file(&mut fst, &mut strbank, ent, &f.fst_name, f.lba, f.data.len() as u32);
        ent += 1;
    }
    fst.extend_from_slice(&strbank);

    let mut end = fst_lba + fst.len();
    for f in &files {
        end = end.max(f.lba as usize + f.data.len());
    }
    let image_len = (end + 0x3ffff) & !0x3ffff; // pad to 256 KiB
    let mut img = vec![0x00u8; image_len];

    // boot.bin
    {
        let b = &mut img[0..BOOT_BIN_SIZE];
        b[0..4].copy_from_slice(b"GRSE"); // gamecode
        b[4..6].copy_from_slice(b"01"); // company
        b[0x08] = 0; // no streaming
        be32(b, 0x0c, 0x1e11d9); // tMD-ish? no: keep zeroed? put DOL region
        be32(b, 0x18, 0x8000_3100); // debug / entry area (harmless)
        be32(b, 0x1c, 0xC233_9F3D); // GC "magic"
        let title = format!("{:<max$}", name, max = 0x3e0 - 1);
        b[0x20..0x20 + title.len()].copy_from_slice(title.as_bytes());
    }
    // bi2.bin
    {
        let b = &mut img[BI2_LBA..BI2_LBA + BI2_SIZE];
        be32(b, 0x424, fst_lba as u32);
        be32(b, 0x428, fst.len() as u32);
    }
    // apploader at 0x2440
    {
        let hdr = &mut img[APPIMG_LBA..APPIMG_LBA + 0x20];
        // date string: 16 ASCII bytes (untracked; zeros fine)
        be32(hdr, 0x10, app_entry);
        be32(hdr, 0x14, app_blob.len() as u32);
        be32(hdr, 0x18, 0); // trailer
        img[APPIMG_LBA + 0x20..APPIMG_LBA + 0x20 + app_blob.len()].copy_from_slice(&app_blob);
    }
    img[dol_lba as usize..dol_lba as usize + dol.len()].copy_from_slice(&dol);
    img[fst_lba..fst_lba + fst.len()].copy_from_slice(&fst);
    for f in files {
        img[f.lba as usize..f.lba as usize + f.data.len()].copy_from_slice(&f.data);
    }

    File::create(&out_path)
        .and_then(|mut fh| fh.write_all(&img))
        .expect("write iso");
    println!(
        "gc-iso: {} ({} bytes): dol@0x{:x} fst@0x{:x} apploader@0x2440 ({}B)",
        out_path,
        img.len(),
        dol_lba,
        fst_lba,
        app_blob.len()
    );
    for (i, s) in sections.iter().enumerate() {
        println!("  section {i}: disc+0x{:x} → mem 0x{:08x} ({}B)", s.0 + dol_lba, s.2, s.1);
    }
    ExitCode::SUCCESS
}
