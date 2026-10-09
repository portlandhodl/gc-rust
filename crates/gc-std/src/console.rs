//! Framebuffer text console.
//!
//! Renders into the external framebuffer (YUY2 4:2:2). Monospace 8x16
//! glyphs (libogc's console font, zlib license), with scrolling and
//! `\n`/`\r`/`\t`/backspace handling.

use crate::video::Video;

const FONT_W: usize = 8;
const FONT_H: usize = 16;

/// Colors in YCbCr (BT.601) so they can be written straight into the XFB.
#[derive(Copy, Clone)]
pub struct ColorYuv {
    pub y: u8,
    pub u: u8,
    pub v: u8,
}

impl ColorYuv {
    pub const WHITE: ColorYuv = ColorYuv { y: 235, u: 128, v: 128 };
    pub const BLACK: ColorYuv = ColorYuv { y: 16, u: 128, v: 128 };

    pub const fn from_rgb(r: u8, g: u8, b: u8) -> ColorYuv {
        let (r, g, b) = (r as i32, g as i32, b as i32);
        let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
        let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
        let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
        let y = if y < 16 { 16 } else if y > 235 { 235 } else { y };
        let u = if u < 0 { 0 } else if u > 255 { 255 } else { u };
        let v = if v < 0 { 0 } else if v > 255 { 255 } else { v };
        ColorYuv { y: y as u8, u: u as u8, v: v as u8 }
    }
}

pub struct Console {
    fb: *mut u32, // YUY2 pixel buffer
    width_px: usize,
    height_px: usize,
    margin: usize,
    cx: usize,
    cy: usize,
    cols: usize,
    rows: usize,
    /// packed pixel words for fg/bg (constant Y only; U,V shared)
    fg_word: u32,
    bg_word: u32,
    fg: ColorYuv,
    bg: ColorYuv,
}

impl Console {
    pub(crate) fn new(video: &Video, margin: usize) -> Console {
        let m = video.mode();
        let width = usize::from(m.fbWidth);
        let height = usize::from(m.xfbHeight);
        let con = Console {
            fb: video.framebuffer().cast(),
            width_px: width,
            height_px: height,
            margin,
            cx: 0,
            cy: 0,
            cols: (width - 2 * margin) / FONT_W,
            rows: (height - 2 * margin) / FONT_H,
            fg_word: pack_word(ColorYuv::WHITE),
            bg_word: pack_word(ColorYuv::BLACK),
            fg: ColorYuv::WHITE,
            bg: ColorYuv::BLACK,
        };
        con.clear();
        con
    }

    pub fn set_fg(&mut self, c: ColorYuv) {
        self.fg = c;
        self.fg_word = pack_word(c);
    }
    pub fn set_bg(&mut self, c: ColorYuv) {
        self.bg = c;
        self.bg_word = pack_word(c);
    }

    pub fn clear(&self) {
        unsafe {
            let mut p = self.fb;
            for _ in 0..(self.width_px * self.height_px / 2) {
                p.write_volatile(self.bg_word);
                p = p.add(1);
            }
        }
    }

    fn scroll_up(&mut self) {
        let wpr = self.width_px / 2;
        let off = FONT_H * wpr;
        unsafe {
            let mut dst = self.fb;
            let mut src = self.fb.add(off);
            for _ in 0..((self.height_px - FONT_H) * wpr) {
                dst.write_volatile(src.read_volatile());
                dst = dst.add(1);
                src = src.add(1);
            }
            let mut p = self.fb.add((self.height_px - FONT_H) * wpr);
            for _ in 0..(FONT_H * wpr) {
                p.write_volatile(self.bg_word);
                p = p.add(1);
            }
        }
    }

    pub fn put_char(&mut self, ch: u8) {
        match ch {
            b'\n' => {
                self.cx = 0;
                self.cy += 1;
            }
            b'\r' => self.cx = 0,
            b'\t' => {
                self.cx = (self.cx + 8) & !7;
                if self.cx >= self.cols {
                    self.cx = 0;
                    self.cy += 1;
                }
            }
            0x08 => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            c if (32..127).contains(&c) => {
                self.render_glyph(c);
                self.cx += 1;
                if self.cx >= self.cols {
                    self.cx = 0;
                    self.cy += 1;
                }
            }
            _ => {}
        }
        if self.cy >= self.rows {
            self.scroll_up();
            self.cy = self.rows - 1;
        }
    }

    /// Write the 8-bit row of a glyph as 4 YUY2 words. U/V of the pair is
    /// taken from whichever side is foreground (gray U/V is 128 anyway for
    /// the built-in colors, so this looks right).
    #[inline]
    fn render_glyph(&self, c: u8) {
        // FONT_8X16 holds all 256 CP437 glyphs from code 0, so index by c
        let base = c as usize * FONT_H;
        let glyph = &crate::font::FONT_8X16[base..base + FONT_H];
        let px = self.margin + self.cx * FONT_W;
        let py = self.margin + self.cy * FONT_H;
        let wpr = self.width_px / 2;
        let fg = self.fg_word;
        let bg = self.bg_word;
        let (fy, fu, fv) = (self.fg.y, self.fg.u, self.fg.v);
        let (by, _bu, _bv) = (self.bg.y, self.bg.u, self.bg.v);
        unsafe {
            for (row, &bits) in glyph.iter().enumerate() {
                let mut p = self.fb.add((py + row) * wpr + px / 2);
                for i in 0..4 {
                    let b0 = (bits >> (7 - i * 2)) & 1;
                    let b1 = (bits >> (6 - i * 2)) & 1;
                    let word = match (b0, b1) {
                        (0, 0) => bg,
                        (1, 1) => fg,
                        (1, 0) => u32::from_be_bytes([fy, fu, by, fv]),
                        (0, 1) => u32::from_be_bytes([by, fu, fy, fv]),
                        _ => unreachable!(),
                    };
                    p.write_volatile(word);
                    p = p.add(1);
                }
            }
        }
    }

    pub fn write_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.put_char(b);
        }
    }
}

#[inline(always)]
fn pack_word(c: ColorYuv) -> u32 {
    u32::from_be_bytes([c.y, c.u, c.y, c.v])
}

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_str(s);
        Ok(())
    }
}
