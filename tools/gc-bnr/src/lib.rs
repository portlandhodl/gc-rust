//! gc-bnr: build a GameCube `opening.bnr` (BNR1) banner.
//!
//! Layout (what the IPL, Swiss and Dolphin read):
//!
//! ```text
//! 0x0000  "BNR1"
//! 0x0004  0x1c bytes padding
//! 0x0020  96x32 RGB5A3 image, 4x4-texel tiles, big-endian u16 (0x1800)
//! 0x1820  game name        0x20  (ANSI, NUL padded)
//! 0x1840  company          0x20
//! 0x1860  full game name   0x40
//! 0x18a0  full company     0x40
//! 0x18e0  description      0x80
//! 0x1960  end
//! ```
//!
//! Swiss shows a folder holding `default.dol` + `opening.bnr` as a single
//! entry with this banner and text.

pub const WIDTH: usize = 96;
pub const HEIGHT: usize = 32;
pub const PIXEL_LEN: usize = WIDTH * HEIGHT * 2;
pub const BNR1_LEN: usize = 0x1960;

const PIXEL_OFF: usize = 0x20;
const DESC_OFF: usize = PIXEL_OFF + PIXEL_LEN;

/// The five text fields of a banner (English slot).
#[derive(Debug, Clone, Default)]
pub struct BannerText {
    pub game_name: String,
    pub company: String,
    pub full_game_name: String,
    pub full_company: String,
    pub description: String,
}

/// An RGB888 image, row-major.
#[derive(Debug, Clone)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 3]>,
}

/// Parse a binary PPM (P6, maxval 255).
pub fn parse_ppm(buf: &[u8]) -> Result<Rgb, String> {
    // header: magic, width, height, maxval as whitespace-separated tokens,
    // '#' comments allowed, then exactly one whitespace byte before data
    let mut pos = 0usize;
    let mut tokens: Vec<String> = Vec::new();
    while tokens.len() < 4 {
        while pos < buf.len() && (buf[pos].is_ascii_whitespace() || buf[pos] == b'#') {
            if buf[pos] == b'#' {
                while pos < buf.len() && buf[pos] != b'\n' {
                    pos += 1;
                }
            } else {
                pos += 1;
            }
        }
        let start = pos;
        while pos < buf.len() && !buf[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if start == pos {
            return Err("truncated PPM header".into());
        }
        tokens.push(String::from_utf8_lossy(&buf[start..pos]).into_owned());
    }
    pos += 1; // the single whitespace byte after maxval
    if tokens[0] != "P6" {
        return Err(format!("not a binary PPM (magic {:?}, want P6)", tokens[0]));
    }
    let num = |s: &str| s.parse::<usize>().map_err(|_| format!("bad PPM number {s:?}"));
    let (width, height, maxval) = (num(&tokens[1])?, num(&tokens[2])?, num(&tokens[3])?);
    if maxval != 255 {
        return Err(format!("PPM maxval {maxval} unsupported (want 255)"));
    }
    let need = width * height * 3;
    let data = buf.get(pos..pos + need).ok_or("PPM pixel data truncated")?;
    let pixels = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    Ok(Rgb { width, height, pixels })
}

/// Opaque RGB5A3 texel: top bit set, 5:5:5.
pub fn rgb5a3_opaque(c: [u8; 3]) -> u16 {
    0x8000 | (u16::from(c[0] >> 3) << 10) | (u16::from(c[1] >> 3) << 5) | u16::from(c[2] >> 3)
}

/// Encode a 96x32 image as tiled RGB5A3 (4x4 tiles, left-to-right then
/// top-to-bottom; texels row-major inside a tile).
pub fn encode_banner_pixels(img: &Rgb) -> Result<Vec<u8>, String> {
    if img.width != WIDTH || img.height != HEIGHT {
        return Err(format!("banner image must be {WIDTH}x{HEIGHT}, got {}x{}", img.width, img.height));
    }
    let mut out = Vec::with_capacity(PIXEL_LEN);
    for ty in (0..HEIGHT).step_by(4) {
        for tx in (0..WIDTH).step_by(4) {
            for y in ty..ty + 4 {
                for x in tx..tx + 4 {
                    out.extend_from_slice(&rgb5a3_opaque(img.pixels[y * WIDTH + x]).to_be_bytes());
                }
            }
        }
    }
    Ok(out)
}

fn put_text(out: &mut [u8], off: usize, len: usize, field: &str, text: &str) -> Result<(), String> {
    if !text.is_ascii() {
        return Err(format!("{field}: only ASCII text is supported"));
    }
    // keep a terminating NUL so every reader sees the end of the string
    if text.len() >= len {
        return Err(format!("{field}: {} bytes, max {}", text.len(), len - 1));
    }
    out[off..off + text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

/// Build a complete BNR1 file.
pub fn build_bnr1(img: &Rgb, text: &BannerText) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; BNR1_LEN];
    out[0..4].copy_from_slice(b"BNR1");
    out[PIXEL_OFF..PIXEL_OFF + PIXEL_LEN].copy_from_slice(&encode_banner_pixels(img)?);
    put_text(&mut out, DESC_OFF, 0x20, "game name", &text.game_name)?;
    put_text(&mut out, DESC_OFF + 0x20, 0x20, "company", &text.company)?;
    put_text(&mut out, DESC_OFF + 0x40, 0x40, "full game name", &text.full_game_name)?;
    put_text(&mut out, DESC_OFF + 0x80, 0x40, "full company", &text.full_company)?;
    put_text(&mut out, DESC_OFF + 0xc0, 0x80, "description", &text.description)?;
    Ok(out)
}
