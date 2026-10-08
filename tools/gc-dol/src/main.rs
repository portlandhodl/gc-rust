//! gc-dol: convert a big-endian 32-bit PowerPC ELF executable into a
//! GameCube .dol image.

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::process::ExitCode;

use gc_dol::pack_elf;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let elf_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: gc-dol <in.elf> <out.dol>");
            eprintln!("       gc-dol --validate <file.dol>     (check invariants, exit 0/1)");
            return ExitCode::FAILURE;
        }
    };
    if elf_path == "--validate" {
        let dol_path = match args.next() {
            Some(p) => p,
            None => {
                eprintln!("usage: gc-dol --validate <file.dol>");
                return ExitCode::FAILURE;
            }
        };
        return validate_cmd(&dol_path);
    }

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
    match pack_elf(&raw) {
        Ok(packed) => {
            if let Err(e) = File::create(&dol_path).and_then(|mut f| f.write_all(&packed.image)) {
                eprintln!("gc-dol: write {dol_path}: {e}");
                return ExitCode::FAILURE;
            }
            println!(
                "gc-dol: {} -> {} ({} bytes, entry {:#x})",
                elf_path,
                dol_path,
                packed.image.len(),
                packed.header.entry
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gc-dol: pack {elf_path}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn validate_cmd(dol_path: &str) -> ExitCode {
    let raw = match fs::read(dol_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("gc-dol: open {dol_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let errs = gc_dol::validate_dol(&raw);
    if errs.is_empty() {
        println!("valid: {dol_path}");
        ExitCode::SUCCESS
    } else {
        for e in &errs {
            eprintln!("invalid: {dol_path}: {e}");
        }
        ExitCode::FAILURE
    }
}
