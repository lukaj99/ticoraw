//! ITU-T T.81 lossless JPEG (SOF3) encoder: one Huffman table, any predictor,
//! interleaved components, no restart markers.

use super::BitWriter;

/// Code lengths: SSSS 0..=14 get 4-bit codes, 15 and 16 get 5-bit codes.
const COUNTS: [u8; 16] = [0, 0, 0, 15, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

fn code(ssss: u32) -> (u64, u32) {
    if ssss < 15 {
        (u64::from(ssss), 4)
    } else {
        (0b11110 + u64::from(ssss - 15), 5)
    }
}

/// Encodes `rows x cols` pixels of `nf` interleaved components (`samples` in raster order).
#[must_use]
pub fn encode(
    samples: &[u16],
    rows: usize,
    cols: usize,
    nf: usize,
    precision: u8,
    predictor: u8,
) -> Vec<u8> {
    assert_eq!(samples.len(), rows * cols * nf);
    let mut j = vec![0xFF, 0xD8];
    // DHT: one DC table, id 0.
    j.extend_from_slice(&[0xFF, 0xC4]);
    j.extend_from_slice(&(2 + 1 + 16 + 17u16).to_be_bytes());
    j.push(0x00);
    j.extend_from_slice(&COUNTS);
    j.extend(0..=16u8);
    // SOF3.
    j.extend_from_slice(&[0xFF, 0xC3]);
    j.extend_from_slice(&(8 + 3 * nf as u16).to_be_bytes());
    j.push(precision);
    j.extend_from_slice(&(rows as u16).to_be_bytes());
    j.extend_from_slice(&(cols as u16).to_be_bytes());
    j.push(nf as u8);
    for c in 0..nf {
        j.extend_from_slice(&[c as u8, 0x11, 0]);
    }
    // SOS: every component uses table 0; Ss = predictor, Se = 0, Ah/Al = 0.
    j.extend_from_slice(&[0xFF, 0xDA]);
    j.extend_from_slice(&(6 + 2 * nf as u16).to_be_bytes());
    j.push(nf as u8);
    for c in 0..nf {
        j.extend_from_slice(&[c as u8, 0x00]);
    }
    j.extend_from_slice(&[predictor, 0, 0]);

    let stride = cols * nf;
    let initial = 1i32 << (precision - 1);
    let mut w = BitWriter::default();
    for y in 0..rows {
        for x in 0..cols {
            for c in 0..nf {
                let i = y * stride + x * nf + c;
                let at = |k: usize| i32::from(samples[k]);
                let pred = match (y, x) {
                    (0, 0) => initial,
                    (0, _) => at(i - nf),
                    (_, 0) => at(i - stride),
                    _ => {
                        let (ra, rb, rc) = (at(i - nf), at(i - stride), at(i - stride - nf));
                        match predictor {
                            1 => ra,
                            2 => rb,
                            3 => rc,
                            4 => ra + rb - rc,
                            5 => ra + ((rb - rc) >> 1),
                            6 => rb + ((ra - rc) >> 1),
                            _ => (ra + rb) >> 1,
                        }
                    }
                };
                // Differences are modulo 2^16, in (-32768, 32768].
                let mut d = (at(i) - pred).rem_euclid(65536);
                if d > 32768 {
                    d -= 65536;
                }
                let ssss = if d == 0 {
                    0
                } else {
                    32 - d.unsigned_abs().leading_zeros()
                };
                let (bits, len) = code(ssss);
                w.put(bits, len);
                if (1..16).contains(&ssss) {
                    let v = if d > 0 { d } else { d + (1 << ssss) - 1 };
                    w.put(v as u64, ssss);
                }
            }
        }
    }
    w.align_ones();
    for b in w.into_bytes() {
        j.push(b);
        if b == 0xFF {
            j.push(0x00);
        }
    }
    j.extend_from_slice(&[0xFF, 0xD9]);
    j
}
