//! Round-trip tests for the gc-dol packer, plus a sweep over any real
//! built DOLs in dist/ (when present).

use gc_dol::{pack_elf, parse_dol_header, validate_dol, DolSection, DOL_ENTRY, MEM1_END, MEM1_START};

/// Build the smallest valid ELF32-BE with one .text (with code), one
/// .rodata and one .bss. The section-builder code doubles as a fixture
/// for the parser.
pub fn make_test_elf() -> Vec<u8> {
    // ELF header (52 bytes) + programless; then 4 section headers (40 each)
    // + section name strings + one byte of code + 4 bytes of rodata.
    // Layout: eh @0 (0x34), sec0@.., strings, .text contents, shstrtab,
    //         then 4 section headers.
    let entry: u32 = DOL_ENTRY; // 0x80003100

    let text_contents: &[u8] = b"\x94\x21\xff\xf0\x93\xe1\x00\x0c"; // stwu r1,-16(r1)/stmw
    let rodata_contents: &[u8] = b"ABCD";

    // choose offsets (each section start aligned to 4):
    let shstr = b"\0.text\0.rodata\0.bss\0.shstrtab\0";
    let off_text = 0x034usize; // right after ELF header
    let off_rodata = off_text + text_contents.len(); // 0x03c? aligned
    let off_shstr = 0x050; // arbitrary but non-overlapping
    let off_shdrs = 0x100; // section headers table

    let addr_text: u32 = entry;
    let addr_rodata: u32 = 0x80008100;
    let addr_bss: u32 = 0x80008200;

    // 5 section headers (null + .text + .rodata + .bss + .shstrtab)
    let mut elf = vec![0u8; off_shdrs + 5 * 40];

    // ELF header
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 1; // ELFCLASS32
    elf[5] = 2; // ELFDATA2MSB
    elf[6] = 1; // EV_CURRENT
    elf[16..18].copy_from_slice(&2u16.to_be_bytes()); // ET_EXEC
    elf[18..20].copy_from_slice(&20u16.to_be_bytes()); // EM_PPC
    elf[20..24].copy_from_slice(&1u32.to_be_bytes());
    elf[24..28].copy_from_slice(&entry.to_be_bytes());
    elf[28..32].copy_from_slice(&0u32.to_be_bytes()); // phoff
    elf[32..36].copy_from_slice(&(off_shdrs as u32).to_be_bytes()); // shoff

    // section header helper
    let shdr = |elf: &mut Vec<u8>, idx: usize, name_off, ty, flags, addr, off, size| {
        let base = off_shdrs + idx * 40;
        elf[base..base + 4].copy_from_slice(&(name_off as u32).to_be_bytes());
        elf[base + 4..base + 8].copy_from_slice(&(ty as u32).to_be_bytes());
        elf[base + 8..base + 12].copy_from_slice(&(flags as u32).to_be_bytes());
        elf[base + 12..base + 16].copy_from_slice(&(addr as u32).to_be_bytes());
        elf[base + 16..base + 20].copy_from_slice(&(off as u32).to_be_bytes());
        elf[base + 20..base + 24].copy_from_slice(&(size as u32).to_be_bytes());
    };

    // write the code/data/shstrtab contents
    elf[off_text..off_text + text_contents.len()].copy_from_slice(text_contents);
    elf[off_rodata..off_rodata + rodata_contents.len()].copy_from_slice(rodata_contents);
    elf[off_shstr..off_shstr + shstr.len()].copy_from_slice(shstr);

    // name offsets in shstr: .text=1, .rodata=7, .bss=15, .shstrtab=20
    // sec 0: null
    shdr(&mut elf, 0, 0, 0, 0, 0, 0, 0);
    // sec 1: .text
    shdr(
        &mut elf,
        1,
        1,
        1, // PROGBITS
        0x6, // SHF_ALLOC|SHF_EXECINSTR
        addr_text,
        off_text,
        text_contents.len(),
    );
    // sec 2: .rodata
    shdr(&mut elf, 2, 7, 1, 0x2, addr_rodata, off_rodata, rodata_contents.len());
    // sec 3: .bss (SHT_NOBITS=8, SHF_ALLOC|SHF_WRITE=0x3)
    shdr(&mut elf, 3, 15, 8, 0x3, addr_bss, 0, 0x200);
    // sec 4: .shstrtab (STRTAB, no flags)
    shdr(&mut elf, 4, 20, 3, 0, 0, off_shstr, shstr.len());

    elf[46..48].copy_from_slice(&40u16.to_be_bytes()); // shentsize
    elf[48..50].copy_from_slice(&5u16.to_be_bytes()); // shnum
    elf[50..52].copy_from_slice(&4u16.to_be_bytes()); // shstrndx

    elf
}

#[test]
fn packs_and_validates_synthetic_elf() {
    let elf = make_test_elf();
    let packed = pack_elf(&elf).expect("pack failed");
    let errs = validate_dol(&packed.image);
    assert!(errs.is_empty(), "packed image invalid: {errs:?}");

    let hdr = packed.header;
    assert_eq!(hdr.entry, DOL_ENTRY);
    assert_eq!(hdr.text.len(), 1);
    assert_eq!(hdr.text[0].addr, 0x80003100);
    assert_eq!(hdr.text[0].size, 8);
    assert_eq!(hdr.data.len(), 1);
    assert_eq!(hdr.data[0].addr, 0x80008100);
    assert_eq!(hdr.bss_addr, 0x80008200);
    assert_eq!(hdr.bss_size, 0x200);
}

#[test]
fn sections_are_32_byte_padded() {
    // .text is 8 bytes and .rodata 4: both must still occupy whole 32-byte
    // units in the file, or Dolphin's DolReader refuses the image.
    let packed = pack_elf(&make_test_elf()).expect("pack failed");
    let hdr = &packed.header;
    assert_eq!(hdr.text[0].file_offset, 0x100);
    assert_eq!(hdr.data[0].file_offset, 0x120);
    assert_eq!(packed.image.len(), 0x140);

    // an image truncated to the unpadded size must be rejected
    let truncated = &packed.image[..0x124];
    assert!(!validate_dol(truncated).is_empty());
}

#[test]
fn rejects_little_endian() {
    let mut elf = make_test_elf();
    elf[5] = 1; // ELFDATA2LSB
    assert!(pack_elf(&elf).is_err());
}

#[test]
fn rejects_wrong_magic() {
    let mut elf = make_test_elf();
    elf[0..4].copy_from_slice(b"NOPE");
    assert!(pack_elf(&elf).is_err());
}

#[test]
fn validates_real_dols_in_dist() {
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../dist");
    if !dist.is_dir() {
        eprintln!("dist/ not present; skipping real-dol validation");
        return;
    }
    let mut checked = 0;
    for entry in std::fs::read_dir(dist).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().map(|e| e == "dol").unwrap_or(false) {
            let raw = std::fs::read(entry.path()).unwrap();
            let errs = validate_dol(&raw);
            assert!(errs.is_empty(), "{}: {errs:?}", entry.path().display());
            // and the header round-trips
            let hdr = parse_dol_header(&raw).unwrap();
            assert_eq!(hdr.entry, DOL_ENTRY);
            for s in hdr.text.iter().chain(hdr.data.iter()) {
                assert!((MEM1_START..MEM1_END).contains(&s.addr));
            }
            checked += 1;
        }
    }
    eprintln!("validated {checked} dol file(s)");
    if checked == 0 {
        panic!("dist/ present but empty — run `make` first");
    }
}
