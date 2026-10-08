//! Nintendo DSP-ADPCM (GC) decoder — canonical math per YAGCD and the
//! public decoders (`vgmstream`'s `decode_ngc_dsp`).
//!
//! Format: 8-byte frames; frame header `hdr`: `coef_index = hdr >> 4` (4
//! bits), `scale = 1 << (hdr & 0x0f)`; 14 signed-nibble samples per frame,
//! high nibble first. Per-sample:
//!
//! ```text
//! s = sign_extend_4(nib) * scale << 11
//! out = clamp16((s + 1024 + c1*hist1 + c2*hist2) >> 11)
//! ```
//!
//! where `(c1, c2)` come from the predictor coefficient table (8 pairs = 16
//! s16 values) — with GC DSP
//! assets the table is carried in the .dsp file header (16 × s16). This
//! module is mono; interleave stereo as you like (channels own separate
//! histories/coefs).

/// Sample history across frames (init to (0,0) at stream start).
#[derive(Copy, Clone, Default)]
pub struct AdpcmHistory {
    pub yn1: i16,
    pub yn2: i16,
}

/// Decode one 8-byte frame into `out` (14 samples). Returns samples written.
pub fn decode_frame(frame: &[u8; 8], coefs: &[i16; 16], hist: &mut AdpcmHistory, out: &mut [i16]) -> usize {
    let coef_sel = (frame[0] >> 4) as usize & 0x07; // 8 predictor pairs
    let scale = 1i32 << (frame[0] & 0x0f);
    let c1 = coefs[coef_sel * 2] as i32;
    let c2 = coefs[coef_sel * 2 + 1] as i32;

    let mut h1 = hist.yn1 as i32;
    let mut h2 = hist.yn2 as i32;
    let n = out.len().min(14);
    for i in 0..n {
        let byte = frame[1 + i / 2];
        let nib = if i & 1 == 0 { byte >> 4 } else { byte & 0x0f };
        let nib = ((nib << 4) as i8 as i32) >> 4; // sign-extend 4-bit
        let acc = (((nib * scale) << 11) + 1024 + c1 * h1 + c2 * h2) >> 11;
        let sample = acc.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        out[i] = sample;
        h2 = h1;
        h1 = sample as i32;
    }
    hist.yn1 = h1 as i16;
    hist.yn2 = h2 as i16;
    n
}

/// Decode a whole mono ADPCM stream (every 8-byte frame → 14 s16 samples).
pub fn decode_all(data: &[u8], coefs: &[i16; 16]) -> alloc::vec::Vec<i16> {
    let frames = data.len() / 8;
    let mut out = alloc::vec::Vec::with_capacity(frames * 14);
    let mut hist = AdpcmHistory::default();
    let mut tmp = [0i16; 14];
    for f in 0..frames {
        let frame: &[u8; 8] = (&data[f * 8..f * 8 + 8]).try_into().unwrap();
        let n = decode_frame(frame, coefs, &mut hist, &mut tmp);
        out.extend_from_slice(&tmp[..n]);
    }
    out
}

/// Audio samples per ADPCM frame.
pub const SAMPLES_PER_FRAME: usize = 14;
/// Bytes per ADPCM frame.
pub const BYTES_PER_FRAME: usize = 8;
