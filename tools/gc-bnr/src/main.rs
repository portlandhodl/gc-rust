//! gc-bnr: build a GameCube `opening.bnr` from a 96x32 PPM and text.

use std::env;
use std::fs;
use std::process::ExitCode;

use gc_bnr::{build_bnr1, parse_ppm, BannerText};

const USAGE: &str = "usage: gc-bnr --image <96x32.ppm> --name <short> --company <short> \\
              [--full-name <long>] [--full-company <long>] [--desc <text>] -o <opening.bnr>
       gc-bnr --image <96x32.ppm> --text <banner.txt> -o <opening.bnr>
  banner.txt: five lines = name, company, full name, full company, description
  (\\n in a description becomes a line break)";

fn main() -> ExitCode {
    let mut image = None;
    let mut out = None;
    let mut text = BannerText::default();
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let Some(val) = args.next() else {
            eprintln!("gc-bnr: {flag} needs a value\n{USAGE}");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--image" => image = Some(val),
            "-o" => out = Some(val),
            "--name" => text.game_name = val,
            "--company" => text.company = val,
            "--full-name" => text.full_game_name = val,
            "--full-company" => text.full_company = val,
            "--desc" => text.description = val.replace("\\n", "\n"),
            "--text" => match fs::read_to_string(&val) {
                Ok(t) => {
                    let mut lines = t.lines().map(str::to_owned);
                    let mut next = || lines.next().unwrap_or_default();
                    text.game_name = next();
                    text.company = next();
                    text.full_game_name = next();
                    text.full_company = next();
                    text.description = next().replace("\\n", "\n");
                }
                Err(e) => {
                    eprintln!("gc-bnr: read {val}: {e}");
                    return ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("gc-bnr: unknown flag {flag}\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    let (Some(image), Some(out)) = (image, out) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    // long fields default to the short ones
    if text.full_game_name.is_empty() {
        text.full_game_name = text.game_name.clone();
    }
    if text.full_company.is_empty() {
        text.full_company = text.company.clone();
    }

    let result = fs::read(&image)
        .map_err(|e| format!("read {image}: {e}"))
        .and_then(|raw| parse_ppm(&raw))
        .and_then(|img| build_bnr1(&img, &text))
        .and_then(|bnr| fs::write(&out, &bnr).map_err(|e| format!("write {out}: {e}")));
    match result {
        Ok(()) => {
            println!("gc-bnr: {image} -> {out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gc-bnr: {e}");
            ExitCode::FAILURE
        }
    }
}
